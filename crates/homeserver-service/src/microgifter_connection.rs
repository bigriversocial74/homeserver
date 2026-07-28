use crate::{database, AppState};
use anyhow::{anyhow, bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode as HttpStatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{DateTime, Timelike, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use futures_util::StreamExt;
use keyring::Entry;
use rand::{rngs::OsRng, RngCore};
use reqwest::Method;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0014_microgifter_entitlement_update_client.sql");
const MIGRATION_KEY: &str = "0014_microgifter_entitlement_update_client";
const PROVIDER_KEY: &str = "microgifter";
const CONTRACT_VERSION: &str = "v1";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";
const PAIRING_EXCHANGE_PATH: &str = "/api/homeserver/v1/pairing/exchange";
const ENTITLEMENT_REFRESH_PATH: &str = "/api/homeserver/v1/entitlements/refresh";
const HEARTBEAT_PATH: &str = "/api/homeserver/v1/devices/heartbeat";
const CREDENTIAL_ROTATION_PATH: &str = "/api/homeserver/v1/devices/credentials/rotate";
const UPDATE_AUTHORIZATION_PATH: &str = "/api/homeserver/v1/updates/authorize";
const UPDATE_RECEIPT_PATH: &str = "/api/homeserver/v1/updates/receipts";
const REPLACEMENT_START_PATH: &str = "/api/homeserver/v1/devices/replacements/start";
const REPLACEMENT_COMPLETE_PATH: &str = "/api/homeserver/v1/devices/replacements/complete";
const MAX_CONTROL_BODY_BYTES: usize = 128 * 1024;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CONNECTIONS: i64 = 50;
const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
const INITIAL_REFRESH_DELAY: Duration = Duration::from_secs(45);

pub const CAPABILITY_REGISTRY: &[&str] = &[
    "pairing.v1",
    "device-registration.v1",
    "device-heartbeat.v1",
    "entitlement-lease.v1",
    "credential-rotation.v1",
    "merchant-assignments.v1",
    "site-assignments.v1",
    "dataset-grants.v1",
    "sync.incremental.v1",
    "operational-data.v1",
    "campaign-actions.v1",
    "signed-updates.v1",
    "update-authorization.v1",
    "update-receipts.v1",
    "device-replacement.v1",
];

pub const APPLICATION_ERROR_CODES: &[&str] = &[
    "microgifter_sync_code_invalid",
    "microgifter_sync_code_expired",
    "microgifter_sync_code_used",
    "microgifter_pairing_interrupted",
    "microgifter_connection_not_found",
    "microgifter_connection_inactive",
    "microgifter_entitlement_missing",
    "microgifter_entitlement_signature_invalid",
    "microgifter_entitlement_key_unknown",
    "microgifter_entitlement_expired",
    "microgifter_entitlement_device_mismatch",
    "microgifter_entitlement_connection_mismatch",
    "microgifter_capability_unsupported",
    "microgifter_cloud_offline",
    "microgifter_credentials_rejected",
    "microgifter_credential_rotation_failed",
    "microgifter_update_not_entitled",
    "microgifter_update_authorization_expired",
    "microgifter_update_deferred",
    "microgifter_duplicate_device_identity",
    "microgifter_device_replacement_required",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderLifecycleState {
    Unpaired,
    PairingPending,
    Active,
    Offline,
    Grace,
    Suspended,
    Revoked,
    Replacing,
    Error,
}

impl ProviderLifecycleState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unpaired => "unpaired",
            Self::PairingPending => "pairing_pending",
            Self::Active => "active",
            Self::Offline => "offline",
            Self::Grace => "grace",
            Self::Suspended => "suspended",
            Self::Revoked => "revoked",
            Self::Replacing => "replacing",
            Self::Error => "error",
        }
    }

    fn from_database(value: &str) -> Self {
        match value {
            "unpaired" => Self::Unpaired,
            "pairing_pending" => Self::PairingPending,
            "active" => Self::Active,
            "offline" => Self::Offline,
            "grace" => Self::Grace,
            "suspended" => Self::Suspended,
            "revoked" => Self::Revoked,
            "replacing" => Self::Replacing,
            _ => Self::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionState {
    Active,
    Grace,
    Suspended,
    Canceled,
    Unknown,
}

impl SubscriptionState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Grace => "grace",
            Self::Suspended => "suspended",
            Self::Canceled => "canceled",
            Self::Unknown => "unknown",
        }
    }

    fn lifecycle(self) -> ProviderLifecycleState {
        match self {
            Self::Active => ProviderLifecycleState::Active,
            Self::Grace => ProviderLifecycleState::Grace,
            Self::Suspended | Self::Canceled => ProviderLifecycleState::Suspended,
            Self::Unknown => ProviderLifecycleState::Error,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateClass {
    Bootstrap,
    Security,
    Maintenance,
    Feature,
    Preview,
    Recovery,
}

impl UpdateClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::Security => "security",
            Self::Maintenance => "maintenance",
            Self::Feature => "feature",
            Self::Preview => "preview",
            Self::Recovery => "recovery",
        }
    }

    fn always_available(self) -> bool {
        matches!(self, Self::Bootstrap | Self::Security | Self::Recovery)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAuthorizationDecision {
    Authorized,
    Denied,
    NotRequired,
}

impl UpdateAuthorizationDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Authorized => "authorized",
            Self::Denied => "denied",
            Self::NotRequired => "not_required",
        }
    }
}

pub trait PairingProvider {
    fn provider_key(&self) -> &'static str;
    fn pairing_exchange_path(&self) -> &'static str;
}

pub trait EntitlementProvider {
    fn entitlement_refresh_path(&self) -> &'static str;
}

pub trait DeviceStatusProvider {
    fn heartbeat_path(&self) -> &'static str;
}

pub trait DatasetGrantProvider {
    fn capability_registry(&self) -> &'static [&'static str];
}

pub trait UpdateAuthorizationProvider {
    fn update_authorization_path(&self) -> &'static str;
}

pub trait ProviderReceiptSink {
    fn update_receipt_path(&self) -> &'static str;
}

#[derive(Debug, Clone, Copy)]
struct MicrogifterProviderAdapter;

impl PairingProvider for MicrogifterProviderAdapter {
    fn provider_key(&self) -> &'static str {
        PROVIDER_KEY
    }

    fn pairing_exchange_path(&self) -> &'static str {
        PAIRING_EXCHANGE_PATH
    }
}

impl EntitlementProvider for MicrogifterProviderAdapter {
    fn entitlement_refresh_path(&self) -> &'static str {
        ENTITLEMENT_REFRESH_PATH
    }
}

impl DeviceStatusProvider for MicrogifterProviderAdapter {
    fn heartbeat_path(&self) -> &'static str {
        HEARTBEAT_PATH
    }
}

impl DatasetGrantProvider for MicrogifterProviderAdapter {
    fn capability_registry(&self) -> &'static [&'static str] {
        CAPABILITY_REGISTRY
    }
}

impl UpdateAuthorizationProvider for MicrogifterProviderAdapter {
    fn update_authorization_path(&self) -> &'static str {
        UPDATE_AUTHORIZATION_PATH
    }
}

impl ProviderReceiptSink for MicrogifterProviderAdapter {
    fn update_receipt_path(&self) -> &'static str {
        UPDATE_RECEIPT_PATH
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Assignment {
    pub id: String,
    pub display_name: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EntitlementLeaseClaims {
    pub schema_version: u32,
    pub lease_id: String,
    pub provider_id: String,
    pub account_id: String,
    pub connection_id: String,
    pub device_id: String,
    pub issued_at_utc: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub subscription_state: SubscriptionState,
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
    #[serde(default)]
    pub denied_capabilities: Vec<String>,
    #[serde(default)]
    pub merchant_scope: Vec<Assignment>,
    #[serde(default)]
    pub site_scope: Vec<Assignment>,
    #[serde(default)]
    pub device_allowance: Value,
    pub update_eligibility: bool,
    #[serde(default)]
    pub allowed_update_channels: Vec<String>,
    pub minimum_homeserver_version: Option<String>,
    pub signing_key_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedEntitlementLease {
    pub payload: EntitlementLeaseClaims,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectMicrogifterRequest {
    pub sync_code: String,
    pub device_display_name: String,
    pub cloud_base_url: String,
    pub merchant_id: Option<String>,
    pub site_id: Option<String>,
    pub request_id: Option<String>,
    pub replacement_id: Option<String>,
    pub make_default: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionReferenceRequest {
    pub connection_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePreferencesRequest {
    pub selected_channel: String,
    pub install_mode: String,
    pub maintenance_start_minute_utc: u16,
    pub maintenance_duration_minutes: u16,
    pub defer_until_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizeUpdateRequest {
    pub connection_id: Option<String>,
    pub update_id: String,
    pub version: String,
    pub update_class: UpdateClass,
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartDeviceReplacementRequest {
    pub connection_id: String,
    pub new_device_display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompleteDeviceReplacementRequest {
    pub replacement_id: String,
    pub new_connection_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePreferencesSnapshot {
    pub selected_channel: String,
    pub install_mode: String,
    pub maintenance_start_minute_utc: u16,
    pub maintenance_duration_minutes: u16,
    pub defer_until_utc: Option<String>,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitySnapshot {
    pub capability_id: String,
    pub grant_state: String,
    pub source: String,
    pub expires_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSnapshot {
    pub receipt_id: String,
    pub event_type: String,
    pub result_category: String,
    pub error_category: Option<String>,
    pub previous_state: Option<String>,
    pub new_state: Option<String>,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrogifterConnectionSnapshot {
    pub connection_id: String,
    pub provider_connection_id: Option<String>,
    pub device_id: String,
    pub device_display_name: String,
    pub owner_account_id: Option<String>,
    pub cloud_base_url: String,
    pub lifecycle_state: ProviderLifecycleState,
    pub contract_version: String,
    pub subscription_state: Option<SubscriptionState>,
    pub entitlement_lease_id: Option<String>,
    pub entitlement_expires_at_utc: Option<String>,
    pub update_eligible: bool,
    pub update_channel: String,
    pub last_heartbeat_at_utc: Option<String>,
    pub last_successful_sync_at_utc: Option<String>,
    pub last_entitlement_refresh_at_utc: Option<String>,
    pub last_credential_rotation_at_utc: Option<String>,
    pub last_update_check_at_utc: Option<String>,
    pub last_update_result: Option<String>,
    pub granted_capabilities: Vec<CapabilitySnapshot>,
    pub assigned_merchant_count: u64,
    pub assigned_site_count: u64,
    pub replacement_state: String,
    pub health_category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrogifterStatusSnapshot {
    pub local_operation_available: bool,
    pub provider_key: String,
    pub contract_version: String,
    pub supported_capabilities: Vec<String>,
    pub connections: Vec<MicrogifterConnectionSnapshot>,
    pub update_preferences: UpdatePreferencesSnapshot,
    pub recent_receipts: Vec<ReceiptSnapshot>,
    pub privacy_boundary: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAuthorizationSnapshot {
    pub authorization_id: String,
    pub update_id: String,
    pub version: String,
    pub update_class: UpdateClass,
    pub channel: String,
    pub decision: UpdateAuthorizationDecision,
    pub reason_code: Option<String>,
    pub expires_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceReplacementSnapshot {
    pub replacement_id: String,
    pub state: String,
    pub sync_code: Option<String>,
    pub expires_at_utc: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (HttpStatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct PairingExchangePayload<'a> {
    provider_key: &'a str,
    sync_code: &'a str,
    request_id: &'a str,
    installation_id: &'a str,
    device_display_name: &'a str,
    homeserver_version: &'a str,
    device_public_key: &'a str,
    requested_capabilities: &'a [&'a str],
    merchant_id: Option<&'a str>,
    site_id: Option<&'a str>,
    replacement_id: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct PairingSigningKey {
    key_id: String,
    public_key_base64: String,
    not_before_utc: Option<String>,
    not_after_utc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PairingExchangeData {
    provider_connection_id: String,
    device_id: String,
    device_token: String,
    owner_account_id: String,
    #[serde(default)]
    scopes: Vec<String>,
    entitlement_signing_key: PairingSigningKey,
    entitlement_lease: SignedEntitlementLease,
}

#[derive(Debug, Serialize)]
struct ProviderRequestEnvelope<'a> {
    request_id: &'a str,
    connection_id: &'a str,
    device_id: &'a str,
    payload: &'a Value,
}

#[derive(Debug, Deserialize)]
struct ProviderApiEnvelope<T> {
    ok: bool,
    message: String,
    data: Option<T>,
    error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDeviceSecrets {
    device_token: String,
    signing_key_base64: String,
}

impl Drop for StoredDeviceSecrets {
    fn drop(&mut self) {
        self.device_token.zeroize();
        self.signing_key_base64.zeroize();
    }
}

#[derive(Debug)]
struct ProviderConnectionRecord {
    provider_connection_id: Option<String>,
    cloud_base_url: String,
    device_id: String,
    credential_key: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct EntitlementRefreshData {
    entitlement_lease: SignedEntitlementLease,
}

#[derive(Debug, Deserialize)]
struct HeartbeatData {
    state: Option<String>,
    entitlement_lease: Option<SignedEntitlementLease>,
}

#[derive(Debug, Deserialize)]
struct CredentialRotationData {
    device_token: String,
}

#[derive(Debug, Deserialize)]
struct ProviderUpdateAuthorizationData {
    authorization_id: String,
    decision: UpdateAuthorizationDecision,
    reason_code: Option<String>,
    #[serde(rename = "issued_at_utc")]
    _issued_at_utc: String,
    expires_at_utc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProviderReplacementStartData {
    replacement_id: String,
    sync_code: String,
    expires_at_utc: String,
}

#[derive(Debug, Deserialize)]
struct ProviderReplacementCompleteData {
    completed: bool,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "Phase 6A migration is not registered exactly once"
    );
    backfill_existing_microgifter_connections(connection)?;
    seed_client_capabilities(connection)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    ensure!(
        !APPLICATION_ERROR_CODES.is_empty(),
        "Phase 6A application error registry is unavailable"
    );
    for table in [
        "provider_connection_profiles",
        "provider_entitlement_signing_keys",
        "provider_entitlement_leases",
        "provider_connection_capabilities",
        "provider_connection_assignments",
        "provider_pairing_attempts",
        "provider_connection_receipts",
        "homeserver_update_preferences",
        "provider_update_authorizations",
        "provider_device_replacements",
        "provider_device_identity_observations",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let preferences: i64 = connection.query_row(
        "SELECT COUNT(*) FROM homeserver_update_preferences WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        preferences == 1,
        "Phase 6A update preferences are unavailable"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM provider_pairing_attempts WHERE state IN ('completed','failed','expired') AND started_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-30 days')",
        [],
    )?;
    transaction.execute(
        "DELETE FROM provider_connection_receipts WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    transaction.execute(
        "DELETE FROM provider_connection_receipts WHERE receipt_id NOT IN (SELECT receipt_id FROM provider_connection_receipts ORDER BY created_at_utc DESC,receipt_id DESC LIMIT 10000)",
        [],
    )?;
    transaction.execute(
        "DELETE FROM provider_device_identity_observations WHERE observed_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/providers/microgifter/status", get(status_handler))
        .route("/v1/providers/microgifter/connect", post(connect_handler))
        .route(
            "/v1/providers/microgifter/entitlement/refresh",
            post(refresh_entitlement_handler),
        )
        .route(
            "/v1/providers/microgifter/heartbeat",
            post(heartbeat_handler),
        )
        .route(
            "/v1/providers/microgifter/credentials/rotate",
            post(rotate_credentials_handler),
        )
        .route(
            "/v1/providers/microgifter/update-preferences",
            get(update_preferences_handler).post(update_preferences_update_handler),
        )
        .route(
            "/v1/providers/microgifter/updates/authorize",
            post(authorize_update_handler),
        )
        .route(
            "/v1/providers/microgifter/device-replacement/start",
            post(start_replacement_handler),
        )
        .route(
            "/v1/providers/microgifter/device-replacement/complete",
            post(complete_replacement_handler),
        )
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now() + INITIAL_REFRESH_DELAY;
    let mut interval = tokio::time::interval_at(start, REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let connection_ids = match phase6a_connection_ids(&state) {
                    Ok(values) => values,
                    Err(error) => {
                        warn!(?error, "unable to inspect Phase 6A Microgifter connections");
                        continue;
                    }
                };
                for connection_id in connection_ids {
                    if let Err(error) = send_heartbeat(state.clone(), &connection_id).await {
                        warn!(?error, %connection_id, "Microgifter heartbeat failed");
                    }
                    if let Err(error) = refresh_entitlement(state.clone(), &connection_id).await {
                        warn!(?error, %connection_id, "Microgifter entitlement refresh failed");
                    }
                }
                if let Err(error) = submit_pending_update_receipts(state.clone()).await {
                    warn!(?error, "Microgifter update receipt submission failed");
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    info!("Phase 6A Microgifter connection worker stopped");
                    return;
                }
            }
        }
    }
}

async fn status_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<MicrogifterStatusSnapshot> {
    tokio::task::spawn_blocking(move || status_snapshot(&*state.connection()?))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("microgifter_status_failed", error))
}

async fn connect_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectMicrogifterRequest>,
) -> ApiResult<MicrogifterConnectionSnapshot> {
    connect_microgifter(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("microgifter_pairing_failed", error))
}

async fn refresh_entitlement_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionReferenceRequest>,
) -> ApiResult<MicrogifterConnectionSnapshot> {
    refresh_entitlement(state, &request.connection_id)
        .await
        .map(Json)
        .map_err(|error| action_error("microgifter_entitlement_refresh_failed", error))
}

async fn heartbeat_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionReferenceRequest>,
) -> ApiResult<MicrogifterConnectionSnapshot> {
    send_heartbeat(state, &request.connection_id)
        .await
        .map(Json)
        .map_err(|error| action_error("microgifter_heartbeat_failed", error))
}

async fn rotate_credentials_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionReferenceRequest>,
) -> ApiResult<MicrogifterConnectionSnapshot> {
    rotate_credentials(state, &request.connection_id)
        .await
        .map(Json)
        .map_err(|error| action_error("microgifter_credential_rotation_failed", error))
}

async fn update_preferences_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<UpdatePreferencesSnapshot> {
    tokio::task::spawn_blocking(move || update_preferences(&*state.connection()?))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("microgifter_update_preferences_failed", error))
}

async fn update_preferences_update_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdatePreferencesRequest>,
) -> ApiResult<UpdatePreferencesSnapshot> {
    tokio::task::spawn_blocking(move || save_update_preferences(&*state.connection()?, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("microgifter_update_preferences_rejected", error))
}

async fn authorize_update_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AuthorizeUpdateRequest>,
) -> ApiResult<UpdateAuthorizationSnapshot> {
    authorize_update(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("microgifter_update_authorization_failed", error))
}

async fn start_replacement_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartDeviceReplacementRequest>,
) -> ApiResult<DeviceReplacementSnapshot> {
    start_device_replacement(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("microgifter_device_replacement_start_failed", error))
}

async fn complete_replacement_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CompleteDeviceReplacementRequest>,
) -> ApiResult<DeviceReplacementSnapshot> {
    complete_device_replacement(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("microgifter_device_replacement_complete_failed", error))
}

async fn connect_microgifter(
    state: Arc<AppState>,
    request: ConnectMicrogifterRequest,
) -> Result<MicrogifterConnectionSnapshot> {
    let adapter = MicrogifterProviderAdapter;
    let sync_code = request.sync_code.trim();
    ensure!(
        (20..=128).contains(&sync_code.len())
            && sync_code
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character)),
        "Microgifter Sync Code is invalid"
    );
    let display_name = sanitize_text(&request.device_display_name, 120, "device display name")?;
    let cloud_base_url = normalize_cloud_base_url(&request.cloud_base_url)?;
    let merchant_id = sanitize_optional_text(request.merchant_id.as_deref(), 120, "merchant id")?;
    let site_id = sanitize_optional_text(request.site_id.as_deref(), 120, "site id")?;
    let replacement_id =
        sanitize_optional_text(request.replacement_id.as_deref(), 120, "replacement id")?;
    let request_id = request
        .request_id
        .as_deref()
        .map(validate_request_id)
        .transpose()?
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    if let Some(connection_id) = completed_pairing_connection(&*state.connection()?, &request_id)? {
        return connection_snapshot(&*state.connection()?, &connection_id);
    }

    let installation_id = database::installation_id(&*state.connection()?)?;
    let local_connection_id = Uuid::new_v4().to_string();
    let credential_key = format!("{installation_id}:cloud:{local_connection_id}");
    record_pairing_pending(
        &*state.connection()?,
        &request_id,
        &cloud_base_url,
        &display_name,
    )?;

    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key_base64 = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().as_bytes());
    let payload = PairingExchangePayload {
        provider_key: adapter.provider_key(),
        sync_code,
        request_id: &request_id,
        installation_id: &installation_id,
        device_display_name: &display_name,
        homeserver_version: env!("CARGO_PKG_VERSION"),
        device_public_key: &public_key_base64,
        requested_capabilities: adapter.capability_registry(),
        merchant_id: merchant_id.as_deref(),
        site_id: site_id.as_deref(),
        replacement_id: replacement_id.as_deref(),
    };
    let response = provider_client()?
        .post(format!(
            "{}{}",
            cloud_base_url,
            adapter.pairing_exchange_path()
        ))
        .header("X-MG-Request-ID", &request_id)
        .json(&payload)
        .send()
        .await
        .context("unable to reach the Microgifter Sync Code service");
    let exchange = match response {
        Ok(response) => decode_provider_response::<PairingExchangeData>(response).await,
        Err(error) => Err(error),
    };
    let exchange = match exchange {
        Ok(exchange) => exchange,
        Err(error) => {
            mark_pairing_failed(
                &*state.connection()?,
                &request_id,
                &public_provider_error(&error),
            )?;
            return Err(error);
        }
    };

    validate_identifier(
        &exchange.provider_connection_id,
        190,
        "provider connection id",
    )?;
    validate_identifier(&exchange.owner_account_id, 190, "owner account id")?;
    ensure!(
        Uuid::parse_str(&exchange.device_id).is_ok(),
        "Microgifter returned an invalid device identity"
    );
    ensure!(
        exchange.device_token.len() >= 32,
        "Microgifter returned incomplete device credentials"
    );
    validate_entitlement_signing_key(&exchange.entitlement_signing_key)?;

    let machine_fingerprint = machine_fingerprint_hash(&installation_id)?;
    ensure_no_duplicate_device(
        &*state.connection()?,
        &exchange.device_id,
        &machine_fingerprint,
    )?;

    let secrets = StoredDeviceSecrets {
        device_token: exchange.device_token.clone(),
        signing_key_base64: URL_SAFE_NO_PAD.encode(signing_key.to_bytes()),
    };
    save_secrets(&credential_key, &secrets)?;

    let save_result = persist_pairing(
        &*state.connection()?,
        PersistPairing {
            local_connection_id: &local_connection_id,
            provider_connection_id: &exchange.provider_connection_id,
            request_id: &request_id,
            display_name: &display_name,
            cloud_base_url: &cloud_base_url,
            owner_account_id: &exchange.owner_account_id,
            device_id: &exchange.device_id,
            public_key_base64: &public_key_base64,
            credential_key: &credential_key,
            scopes: &exchange.scopes,
            signing_key: &exchange.entitlement_signing_key,
            machine_fingerprint_hash: &machine_fingerprint,
            make_default: request.make_default.unwrap_or(false),
        },
    );
    if let Err(error) = save_result {
        let _ = delete_secrets(&credential_key);
        mark_pairing_failed(
            &*state.connection()?,
            &request_id,
            "pairing_persistence_failed",
        )?;
        return Err(error);
    }

    if let Err(error) = accept_entitlement_lease(
        &*state.connection()?,
        &local_connection_id,
        &exchange.entitlement_lease,
    ) {
        set_lifecycle_state(
            &*state.connection()?,
            &local_connection_id,
            ProviderLifecycleState::Error,
        )?;
        record_receipt(
            &*state.connection()?,
            Some(&local_connection_id),
            Some(&exchange.device_id),
            "entitlement.lease_rejected",
            Some(&request_id),
            Some("pairing_pending"),
            Some("error"),
            "error",
            Some(&public_entitlement_error(&error)),
            &json!({"lease_id": exchange.entitlement_lease.payload.lease_id}),
        )?;
        return Err(error).context("pairing completed but the entitlement lease was rejected");
    }

    if let Some(replacement_id) = replacement_id {
        link_replacement_to_new_connection(
            &*state.connection()?,
            &replacement_id,
            &local_connection_id,
            &exchange.device_id,
        )?;
    }

    record_receipt(
        &*state.connection()?,
        Some(&local_connection_id),
        Some(&exchange.device_id),
        "pairing.completed",
        Some(&request_id),
        Some("pairing_pending"),
        Some("active"),
        "success",
        None,
        &json!({
            "provider_connection_id": exchange.provider_connection_id,
            "contract_version": CONTRACT_VERSION,
        }),
    )?;
    connection_snapshot(&*state.connection()?, &local_connection_id)
}

async fn refresh_entitlement(
    state: Arc<AppState>,
    connection_id: &str,
) -> Result<MicrogifterConnectionSnapshot> {
    validate_uuid(connection_id, "connection id")?;
    let payload = json!({
        "homeserver_version": env!("CARGO_PKG_VERSION"),
        "supported_capabilities": CAPABILITY_REGISTRY,
    });
    let adapter = MicrogifterProviderAdapter;
    let result = signed_provider_post::<EntitlementRefreshData>(
        &state,
        connection_id,
        adapter.entitlement_refresh_path(),
        &payload,
    )
    .await;
    match result {
        Ok(data) => {
            accept_entitlement_lease(
                &*state.connection()?,
                connection_id,
                &data.entitlement_lease,
            )?;
            let device_id =
                provider_connection_record(&*state.connection()?, connection_id)?.device_id;
            record_receipt(
                &*state.connection()?,
                Some(connection_id),
                Some(&device_id),
                "entitlement.lease_accepted",
                None,
                None,
                None,
                "success",
                None,
                &json!({"lease_id": data.entitlement_lease.payload.lease_id}),
            )?;
        }
        Err(error) => {
            mark_connection_offline(
                &*state.connection()?,
                connection_id,
                &public_provider_error(&error),
            )?;
            return Err(error);
        }
    }
    connection_snapshot(&*state.connection()?, connection_id)
}

async fn send_heartbeat(
    state: Arc<AppState>,
    connection_id: &str,
) -> Result<MicrogifterConnectionSnapshot> {
    validate_uuid(connection_id, "connection id")?;
    let payload = heartbeat_payload(
        &*state.connection()?,
        connection_id,
        &state.config.server_name,
    )?;
    let adapter = MicrogifterProviderAdapter;
    let result = signed_provider_post::<HeartbeatData>(
        &state,
        connection_id,
        adapter.heartbeat_path(),
        &payload,
    )
    .await;
    match result {
        Ok(data) => {
            if data.state.as_deref() == Some("revoked") {
                set_lifecycle_state(
                    &*state.connection()?,
                    connection_id,
                    ProviderLifecycleState::Revoked,
                )?;
                mark_cloud_connection_state(&*state.connection()?, connection_id, "revoked", None)?;
                bail!("Microgifter connection was revoked");
            }
            if let Some(lease) = data.entitlement_lease {
                accept_entitlement_lease(&*state.connection()?, connection_id, &lease)?;
            }
            let connection = state.connection()?;
            connection.execute(
                "UPDATE provider_connection_profiles SET last_heartbeat_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
                params![connection_id],
            )?;
            record_receipt(
                &connection,
                Some(connection_id),
                None,
                "heartbeat.sent",
                None,
                None,
                None,
                "success",
                None,
                &json!({}),
            )?;
        }
        Err(error) => {
            mark_connection_offline(
                &*state.connection()?,
                connection_id,
                &public_provider_error(&error),
            )?;
            return Err(error);
        }
    }
    connection_snapshot(&*state.connection()?, connection_id)
}

async fn rotate_credentials(
    state: Arc<AppState>,
    connection_id: &str,
) -> Result<MicrogifterConnectionSnapshot> {
    validate_uuid(connection_id, "connection id")?;
    let record = provider_connection_record(&*state.connection()?, connection_id)?;
    let payload = json!({
        "device_id": record.device_id,
        "rotation_request_id": Uuid::new_v4().to_string(),
    });
    let rotated = signed_provider_post::<CredentialRotationData>(
        &state,
        connection_id,
        CREDENTIAL_ROTATION_PATH,
        &payload,
    )
    .await?;
    ensure!(
        rotated.device_token.len() >= 32,
        "Microgifter returned an invalid rotated credential"
    );
    let mut secrets = load_secrets(&record.credential_key)?;
    secrets.device_token = rotated.device_token;
    save_secrets(&record.credential_key, &secrets)?;
    let connection = state.connection()?;
    connection.execute(
        "UPDATE provider_connection_profiles SET last_credential_rotation_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
        params![connection_id],
    )?;
    record_receipt(
        &connection,
        Some(connection_id),
        Some(&record.device_id),
        "credential.rotated",
        None,
        None,
        None,
        "success",
        None,
        &json!({}),
    )?;
    connection_snapshot(&connection, connection_id)
}

async fn authorize_update(
    state: Arc<AppState>,
    request: AuthorizeUpdateRequest,
) -> Result<UpdateAuthorizationSnapshot> {
    validate_identifier(&request.update_id, 190, "update id")?;
    validate_version(&request.version)?;
    validate_channel(&request.channel)?;

    if request.update_class.always_available() {
        let snapshot = store_update_authorization(
            &*state.connection()?,
            request.connection_id.as_deref(),
            &request,
            UpdateAuthorizationDecision::NotRequired,
            Some("independent_security_or_recovery_update"),
            None,
            None,
        )?;
        return Ok(snapshot);
    }

    let connection_id = match request.connection_id.as_deref() {
        Some(value) => value.to_owned(),
        None => default_phase6a_connection_id(&*state.connection()?)?
            .context("paid update authorization requires an active Microgifter connection")?,
    };
    validate_uuid(&connection_id, "connection id")?;
    let local_decision = local_update_policy(&*state.connection()?, &connection_id, &request)?;
    if local_decision.0 == UpdateAuthorizationDecision::Denied {
        return store_update_authorization(
            &*state.connection()?,
            Some(&connection_id),
            &request,
            local_decision.0,
            local_decision.1.as_deref(),
            None,
            None,
        );
    }

    let payload = serde_json::to_value(&request)?;
    let remote = signed_provider_post::<ProviderUpdateAuthorizationData>(
        &state,
        &connection_id,
        MicrogifterProviderAdapter.update_authorization_path(),
        &payload,
    )
    .await;
    match remote {
        Ok(remote) => store_update_authorization(
            &*state.connection()?,
            Some(&connection_id),
            &request,
            remote.decision,
            remote.reason_code.as_deref(),
            Some(&remote.authorization_id),
            remote.expires_at_utc.as_deref(),
        ),
        Err(error) => {
            if local_decision.0 == UpdateAuthorizationDecision::Authorized {
                store_update_authorization(
                    &*state.connection()?,
                    Some(&connection_id),
                    &request,
                    UpdateAuthorizationDecision::Authorized,
                    Some("valid_offline_entitlement_lease"),
                    None,
                    entitlement_expiration(&*state.connection()?, &connection_id)?.as_deref(),
                )
            } else {
                Err(error)
            }
        }
    }
}

async fn start_device_replacement(
    state: Arc<AppState>,
    request: StartDeviceReplacementRequest,
) -> Result<DeviceReplacementSnapshot> {
    validate_uuid(&request.connection_id, "connection id")?;
    let new_device_display_name = sanitize_text(
        &request.new_device_display_name,
        120,
        "new device display name",
    )?;
    let record = provider_connection_record(&*state.connection()?, &request.connection_id)?;
    let payload = json!({
        "old_device_id": record.device_id,
        "new_device_display_name": new_device_display_name,
    });
    let response = signed_provider_post::<ProviderReplacementStartData>(
        &state,
        &request.connection_id,
        REPLACEMENT_START_PATH,
        &payload,
    )
    .await?;
    validate_identifier(&response.replacement_id, 190, "replacement id")?;
    ensure!(
        (20..=128).contains(&response.sync_code.len()),
        "replacement Sync Code is invalid"
    );
    let connection = state.connection()?;
    connection.execute(
        "INSERT INTO provider_device_replacements (replacement_id,provider_key,old_connection_id,old_device_id,state,created_at_utc) VALUES (?1,?2,?3,?4,'pending',strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(replacement_id) DO UPDATE SET state='pending',failure_code=NULL",
        params![response.replacement_id, PROVIDER_KEY, request.connection_id, record.device_id],
    )?;
    connection.execute(
        "UPDATE provider_connection_profiles SET lifecycle_state='replacing',replacement_state='pending',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
        params![request.connection_id],
    )?;
    record_receipt(
        &connection,
        Some(&request.connection_id),
        Some(&record.device_id),
        "device.replacement_initiated",
        None,
        Some("active"),
        Some("replacing"),
        "success",
        None,
        &json!({"replacement_id": response.replacement_id}),
    )?;
    Ok(DeviceReplacementSnapshot {
        replacement_id: response.replacement_id,
        state: "pending".to_owned(),
        sync_code: Some(response.sync_code),
        expires_at_utc: Some(response.expires_at_utc),
    })
}

async fn complete_device_replacement(
    state: Arc<AppState>,
    request: CompleteDeviceReplacementRequest,
) -> Result<DeviceReplacementSnapshot> {
    validate_identifier(&request.replacement_id, 190, "replacement id")?;
    validate_uuid(&request.new_connection_id, "new connection id")?;
    let (old_connection_id, old_device_id) = state.connection()?.query_row(
        "SELECT old_connection_id,old_device_id FROM provider_device_replacements WHERE replacement_id=?1 AND new_connection_id=?2 AND state IN ('paired','activated')",
        params![request.replacement_id, request.new_connection_id],
        |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
    ).context("device replacement is not ready to complete")?;
    let payload = json!({
        "replacement_id": request.replacement_id,
        "old_device_id": old_device_id,
    });
    let response = signed_provider_post::<ProviderReplacementCompleteData>(
        &state,
        &request.new_connection_id,
        REPLACEMENT_COMPLETE_PATH,
        &payload,
    )
    .await?;
    ensure!(
        response.completed,
        "Microgifter did not complete the device replacement"
    );

    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE provider_device_replacements SET state='completed',completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),failure_code=NULL WHERE replacement_id=?1",
        params![request.replacement_id],
    )?;
    transaction.execute(
        "UPDATE provider_connection_profiles SET lifecycle_state='active',replacement_state='completed',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
        params![request.new_connection_id],
    )?;
    if let Some(old_connection_id) = old_connection_id.as_deref() {
        transaction.execute(
            "UPDATE provider_connection_profiles SET lifecycle_state='revoked',replacement_state='completed',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
            params![old_connection_id],
        )?;
        transaction.execute(
            "UPDATE cloud_connections SET state='revoked',is_default=0,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
            params![old_connection_id],
        )?;
    }
    transaction.commit()?;
    if let Some(old_connection_id) = old_connection_id.as_deref() {
        if let Ok(old_record) = provider_connection_record(&connection, old_connection_id) {
            let _ = delete_secrets(&old_record.credential_key);
        }
    }
    record_receipt(
        &connection,
        Some(&request.new_connection_id),
        None,
        "device.replacement_completed",
        None,
        Some("replacing"),
        Some("active"),
        "success",
        None,
        &json!({"replacement_id": request.replacement_id}),
    )?;
    Ok(DeviceReplacementSnapshot {
        replacement_id: request.replacement_id,
        state: "completed".to_owned(),
        sync_code: None,
        expires_at_utc: None,
    })
}

async fn signed_provider_post<T: DeserializeOwned>(
    state: &AppState,
    connection_id: &str,
    path: &str,
    payload: &Value,
) -> Result<T> {
    let record = provider_connection_record(&*state.connection()?, connection_id)?;
    ensure!(
        !matches!(record.state.as_str(), "revoked" | "disconnected"),
        "Microgifter connection is inactive"
    );
    let secrets = load_secrets(&record.credential_key)?;
    let request_id = Uuid::new_v4().to_string();
    let body_value = serde_json::to_value(ProviderRequestEnvelope {
        request_id: &request_id,
        connection_id,
        device_id: &record.device_id,
        payload,
    })?;
    let body = canonical_json_string(&body_value)?;
    let timestamp = Utc::now().timestamp().to_string();
    let nonce = Uuid::new_v4().simple().to_string();
    let canonical = canonical_request(&Method::POST, path, &timestamp, &nonce, &body);
    let signing_bytes = decode_base64(&secrets.signing_key_base64)
        .context("HomeServer signing key encoding is invalid")?;
    let signing_array: [u8; 32] = signing_bytes
        .try_into()
        .map_err(|_| anyhow!("HomeServer signing key length is invalid"))?;
    let signing_key = SigningKey::from_bytes(&signing_array);
    let signature = URL_SAFE_NO_PAD.encode(signing_key.sign(canonical.as_bytes()).to_bytes());

    let response = provider_client()?
        .post(format!("{}{}", record.cloud_base_url, path))
        .bearer_auth(&secrets.device_token)
        .header("X-MG-Homeserver-ID", &record.device_id)
        .header(
            "X-MG-Provider-Connection-ID",
            record
                .provider_connection_id
                .as_deref()
                .unwrap_or(connection_id),
        )
        .header("X-MG-Contract-Version", CONTRACT_VERSION)
        .header("X-MG-Request-ID", &request_id)
        .header("X-MG-Timestamp", timestamp)
        .header("X-MG-Nonce", nonce)
        .header("X-MG-Signature", signature)
        .header("X-MG-Homeserver-Version", env!("CARGO_PKG_VERSION"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .context("Microgifter provider request failed");
    let decoded = match response {
        Ok(response) => decode_provider_response::<T>(response).await,
        Err(error) => Err(error),
    };
    match decoded {
        Ok(value) => {
            mark_cloud_connection_success(&*state.connection()?, connection_id)?;
            Ok(value)
        }
        Err(error) => {
            mark_connection_offline(
                &*state.connection()?,
                connection_id,
                &public_provider_error(&error),
            )?;
            Err(error)
        }
    }
}

async fn decode_provider_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_PROVIDER_RESPONSE_BYTES as u64,
            "Microgifter response exceeds the HomeServer size limit"
        );
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("unable to read Microgifter response")?;
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_PROVIDER_RESPONSE_BYTES,
            "Microgifter response exceeds the HomeServer size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let envelope: ProviderApiEnvelope<T> = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "Microgifter returned invalid JSON with HTTP {}",
            status.as_u16()
        )
    })?;
    if !status.is_success() || !envelope.ok {
        let code = envelope
            .error_code
            .unwrap_or_else(|| format!("http_{}", status.as_u16()));
        bail!(
            "Microgifter request failed ({code}): {}",
            envelope.message.chars().take(500).collect::<String>()
        );
    }
    envelope
        .data
        .context("Microgifter response did not contain data")
}

fn provider_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .user_agent(format!(
            "Microgifter-HomeServer/{}/{}",
            env!("CARGO_PKG_VERSION"),
            CONTRACT_VERSION
        ))
        .build()?)
}

struct PersistPairing<'a> {
    local_connection_id: &'a str,
    provider_connection_id: &'a str,
    request_id: &'a str,
    display_name: &'a str,
    cloud_base_url: &'a str,
    owner_account_id: &'a str,
    device_id: &'a str,
    public_key_base64: &'a str,
    credential_key: &'a str,
    scopes: &'a [String],
    signing_key: &'a PairingSigningKey,
    machine_fingerprint_hash: &'a str,
    make_default: bool,
}

fn persist_pairing(connection: &Connection, value: PersistPairing<'_>) -> Result<()> {
    let connection_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM cloud_connections", [], |row| {
            row.get(0)
        })?;
    ensure!(
        connection_count < MAX_CONNECTIONS,
        "cloud connection limit reached"
    );
    ensure!(
        connection.query_row(
            "SELECT COUNT(*) FROM cloud_connections WHERE provider_key=?1 AND device_id=?2",
            params![PROVIDER_KEY, value.device_id],
            |row| row.get::<_, i64>(0),
        )? == 0,
        "Microgifter device identity is already registered locally"
    );
    let default_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM cloud_connections WHERE is_default=1",
        [],
        |row| row.get(0),
    )?;
    let make_default = value.make_default || default_count == 0;
    let transaction = connection.unchecked_transaction()?;
    if make_default {
        transaction.execute("UPDATE cloud_connections SET is_default=0", [])?;
    }
    transaction.execute(
        "INSERT INTO cloud_connections (connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,public_key_base64,credential_key,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,NULL,NULL,?5,?6,?7,'connected',?8,?9,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            value.local_connection_id,
            PROVIDER_KEY,
            value.display_name,
            value.cloud_base_url,
            value.device_id,
            value.public_key_base64,
            value.credential_key,
            serde_json::to_string(value.scopes)?,
            i64::from(make_default),
        ],
    )?;
    transaction.execute(
        "INSERT INTO provider_connection_profiles (connection_id,provider_key,provider_connection_id,contract_version,lifecycle_state,owner_account_id,device_display_name,connector_version,capability_registry_version,subscription_state,update_eligible,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,'pairing_pending',?5,?6,?7,'v1','unknown',0,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            value.local_connection_id,
            PROVIDER_KEY,
            value.provider_connection_id,
            CONTRACT_VERSION,
            value.owner_account_id,
            value.display_name,
            env!("CARGO_PKG_VERSION"),
        ],
    )?;
    transaction.execute(
        "INSERT INTO provider_entitlement_signing_keys (provider_key,key_id,public_key_base64,state,not_before_utc,not_after_utc,source) VALUES (?1,?2,?3,'active',?4,?5,'pairing') ON CONFLICT(provider_key,key_id) DO UPDATE SET public_key_base64=excluded.public_key_base64,state='active',not_before_utc=excluded.not_before_utc,not_after_utc=excluded.not_after_utc,source='pairing'",
        params![
            PROVIDER_KEY,
            value.signing_key.key_id,
            value.signing_key.public_key_base64,
            value.signing_key.not_before_utc,
            value.signing_key.not_after_utc,
        ],
    )?;
    transaction.execute(
        "INSERT INTO provider_device_identity_observations (observation_id,provider_key,device_id,installation_id,machine_fingerprint_hash,connection_id,disposition,observed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'trusted',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            Uuid::new_v4().to_string(),
            PROVIDER_KEY,
            value.device_id,
            database::installation_id(&transaction)?,
            value.machine_fingerprint_hash,
            value.local_connection_id,
        ],
    )?;
    transaction.execute(
        "UPDATE provider_pairing_attempts SET state='completed',connection_id=?1,error_code=NULL,completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE request_id=?2",
        params![value.local_connection_id, value.request_id],
    )?;
    transaction.commit()?;
    Ok(())
}

fn accept_entitlement_lease(
    connection: &Connection,
    local_connection_id: &str,
    lease: &SignedEntitlementLease,
) -> Result<()> {
    validate_entitlement_lease(connection, local_connection_id, lease)?;
    let payload = &lease.payload;
    let lifecycle = payload.subscription_state.lifecycle();
    let payload_json = serde_json::to_string(payload)?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE provider_entitlement_leases SET state='superseded' WHERE connection_id=?1 AND state='accepted' AND lease_id<>?2",
        params![local_connection_id, payload.lease_id],
    )?;
    transaction.execute(
        "INSERT INTO provider_entitlement_leases (lease_id,connection_id,provider_key,account_id,device_id,schema_version,issued_at_utc,not_before_utc,expires_at_utc,subscription_state,granted_capabilities_json,denied_capabilities_json,merchant_scope_json,site_scope_json,device_allowance_json,update_eligibility,allowed_update_channels_json,minimum_homeserver_version,signing_key_id,payload_json,signature_base64,state,accepted_at_utc,rejection_code) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,'accepted',strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL) ON CONFLICT(lease_id) DO UPDATE SET expires_at_utc=excluded.expires_at_utc,subscription_state=excluded.subscription_state,granted_capabilities_json=excluded.granted_capabilities_json,denied_capabilities_json=excluded.denied_capabilities_json,merchant_scope_json=excluded.merchant_scope_json,site_scope_json=excluded.site_scope_json,device_allowance_json=excluded.device_allowance_json,update_eligibility=excluded.update_eligibility,allowed_update_channels_json=excluded.allowed_update_channels_json,minimum_homeserver_version=excluded.minimum_homeserver_version,payload_json=excluded.payload_json,signature_base64=excluded.signature_base64,state='accepted',accepted_at_utc=excluded.accepted_at_utc,rejection_code=NULL",
        params![
            payload.lease_id,
            local_connection_id,
            PROVIDER_KEY,
            payload.account_id,
            payload.device_id,
            payload.schema_version,
            payload.issued_at_utc,
            payload.not_before_utc,
            payload.expires_at_utc,
            payload.subscription_state.as_str(),
            serde_json::to_string(&payload.granted_capabilities)?,
            serde_json::to_string(&payload.denied_capabilities)?,
            serde_json::to_string(&payload.merchant_scope)?,
            serde_json::to_string(&payload.site_scope)?,
            serde_json::to_string(&payload.device_allowance)?,
            i64::from(payload.update_eligibility),
            serde_json::to_string(&payload.allowed_update_channels)?,
            payload.minimum_homeserver_version,
            payload.signing_key_id,
            payload_json,
            lease.signature,
        ],
    )?;
    transaction.execute(
        "UPDATE provider_connection_profiles SET lifecycle_state=?1,owner_account_id=?2,entitlement_lease_id=?3,entitlement_expires_at_utc=?4,subscription_state=?5,update_eligible=?6,last_entitlement_refresh_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?7",
        params![
            lifecycle.as_str(),
            payload.account_id,
            payload.lease_id,
            payload.expires_at_utc,
            payload.subscription_state.as_str(),
            i64::from(payload.update_eligibility),
            local_connection_id,
        ],
    )?;
    transaction.execute(
        "DELETE FROM provider_connection_capabilities WHERE connection_id=?1 AND source IN ('lease','account','device','server')",
        params![local_connection_id],
    )?;
    for capability in &payload.granted_capabilities {
        transaction.execute(
            "INSERT INTO provider_connection_capabilities (connection_id,capability_id,grant_state,source,expires_at_utc) VALUES (?1,?2,'granted','lease',?3)",
            params![local_connection_id, capability, payload.expires_at_utc],
        )?;
    }
    for capability in &payload.denied_capabilities {
        transaction.execute(
            "INSERT INTO provider_connection_capabilities (connection_id,capability_id,grant_state,source,expires_at_utc) VALUES (?1,?2,'denied','lease',?3)",
            params![local_connection_id, capability, payload.expires_at_utc],
        )?;
    }
    transaction.execute(
        "DELETE FROM provider_connection_assignments WHERE connection_id=?1",
        params![local_connection_id],
    )?;
    for assignment in &payload.merchant_scope {
        transaction.execute(
            "INSERT INTO provider_connection_assignments (connection_id,assignment_type,assignment_id,parent_assignment_id,display_name,state) VALUES (?1,'merchant',?2,?3,?4,'active')",
            params![local_connection_id, assignment.id, assignment.parent_id, assignment.display_name],
        )?;
    }
    for assignment in &payload.site_scope {
        transaction.execute(
            "INSERT INTO provider_connection_assignments (connection_id,assignment_type,assignment_id,parent_assignment_id,display_name,state) VALUES (?1,'site',?2,?3,?4,'active')",
            params![local_connection_id, assignment.id, assignment.parent_id, assignment.display_name],
        )?;
    }
    transaction.execute(
        "UPDATE cloud_connections SET state=?1,last_error=NULL,last_success_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?2",
        params![cloud_state_for_lifecycle(lifecycle), local_connection_id],
    )?;
    transaction.commit()?;
    Ok(())
}

fn validate_entitlement_lease(
    connection: &Connection,
    local_connection_id: &str,
    lease: &SignedEntitlementLease,
) -> Result<()> {
    let payload = &lease.payload;
    ensure!(
        payload.schema_version == 1,
        "unsupported entitlement lease schema"
    );
    ensure!(
        payload.provider_id == PROVIDER_KEY,
        "entitlement provider is invalid"
    );
    ensure!(
        payload.connection_id == local_connection_id
            || provider_connection_id(connection, local_connection_id)?.as_deref()
                == Some(payload.connection_id.as_str()),
        "entitlement connection identity does not match"
    );
    let record = provider_connection_record(connection, local_connection_id)?;
    ensure!(
        payload.device_id == record.device_id,
        "entitlement device identity does not match"
    );
    validate_identifier(&payload.account_id, 190, "entitlement account id")?;
    validate_identifier(&payload.lease_id, 190, "entitlement lease id")?;
    ensure!(
        payload.signing_key_id.len() <= 120,
        "entitlement signing key id is invalid"
    );
    let issued = parse_utc(&payload.issued_at_utc)?;
    let not_before = parse_utc(&payload.not_before_utc)?;
    let expires = parse_utc(&payload.expires_at_utc)?;
    let now = Utc::now();
    ensure!(
        issued <= now + chrono::Duration::minutes(10),
        "entitlement lease issue time is in the future"
    );
    ensure!(
        not_before <= now + chrono::Duration::minutes(2),
        "entitlement lease is not active yet"
    );
    ensure!(expires > now, "entitlement lease is expired");
    ensure!(
        expires > not_before,
        "entitlement lease validity window is invalid"
    );
    let mut capability_values = std::collections::BTreeSet::new();
    for capability in payload
        .granted_capabilities
        .iter()
        .chain(payload.denied_capabilities.iter())
    {
        ensure!(
            capability_values.insert(capability.as_str()),
            "entitlement lease contains duplicate capability decisions"
        );
        ensure!(
            CAPABILITY_REGISTRY.contains(&capability.as_str()),
            "entitlement lease contains an unsupported capability"
        );
    }
    for channel in &payload.allowed_update_channels {
        validate_channel(channel)?;
    }
    let (public_key_base64, key_state, key_not_before, key_not_after): (
        String,
        String,
        Option<String>,
        Option<String>,
    ) = connection
        .query_row(
            "SELECT public_key_base64,state,not_before_utc,not_after_utc FROM provider_entitlement_signing_keys WHERE provider_key=?1 AND key_id=?2",
            params![PROVIDER_KEY, payload.signing_key_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .context("entitlement signing key is unknown")?;
    ensure!(
        key_state == "active",
        "entitlement signing key is not active"
    );
    if let Some(value) = key_not_before {
        ensure!(
            parse_utc(&value)? <= now,
            "entitlement signing key is not active yet"
        );
    }
    if let Some(value) = key_not_after {
        ensure!(
            parse_utc(&value)? > now,
            "entitlement signing key is expired"
        );
    }
    let public_key_bytes = decode_base64(&public_key_base64)?;
    let public_key_array: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| anyhow!("entitlement public key length is invalid"))?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key_array).context("entitlement public key is invalid")?;
    let signature_bytes = decode_base64(&lease.signature)?;
    let signature = Signature::from_slice(&signature_bytes)
        .context("entitlement signature length is invalid")?;
    let canonical_payload = serde_json::to_vec(payload)?;
    verifying_key
        .verify(&canonical_payload, &signature)
        .context("entitlement lease signature verification failed")?;
    Ok(())
}

fn heartbeat_payload(
    connection: &Connection,
    connection_id: &str,
    server_name: &str,
) -> Result<Value> {
    let snapshot = connection_snapshot(connection, connection_id)?;
    let schema_version = connection
        .query_row(
            "SELECT migration_key FROM schema_migrations ORDER BY migration_key DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "unknown".to_owned());
    Ok(json!({
        "connection_state": snapshot.lifecycle_state,
        "provider_connection_id": snapshot.provider_connection_id,
        "device_id": snapshot.device_id,
        "device_display_name": snapshot.device_display_name,
        "homeserver_version": env!("CARGO_PKG_VERSION"),
        "microgifter_connector_version": env!("CARGO_PKG_VERSION"),
        "cloud_contract_version": snapshot.contract_version,
        "update_manifest_schema_version": 1,
        "local_database_schema_version": schema_version,
        "update_channel": snapshot.update_channel,
        "last_successful_synchronization": snapshot.last_successful_sync_at_utc,
        "last_update_check": snapshot.last_update_check_at_utc,
        "last_update_result": snapshot.last_update_result,
        "entitlement_lease_expiration": snapshot.entitlement_expires_at_utc,
        "health_category": snapshot.health_category,
        "granted_capability_identifiers": snapshot.granted_capabilities.iter().filter(|value| value.grant_state == "granted").map(|value| value.capability_id.clone()).collect::<Vec<_>>(),
        "assigned_merchant_count": snapshot.assigned_merchant_count,
        "assigned_site_count": snapshot.assigned_site_count,
        "credential_rotation_status": snapshot.last_credential_rotation_at_utc.as_ref().map(|_| "rotated").unwrap_or("not_rotated"),
        "replacement_status": snapshot.replacement_state,
        "server_name": server_name.chars().take(120).collect::<String>(),
    }))
}

fn local_update_policy(
    connection: &Connection,
    connection_id: &str,
    request: &AuthorizeUpdateRequest,
) -> Result<(UpdateAuthorizationDecision, Option<String>)> {
    if request.update_class.always_available() {
        return Ok((
            UpdateAuthorizationDecision::NotRequired,
            Some("independent_security_or_recovery_update".to_owned()),
        ));
    }
    let (state, subscription, expires, eligible): (String, Option<String>, Option<String>, i64) =
        connection.query_row(
            "SELECT lifecycle_state,subscription_state,entitlement_expires_at_utc,update_eligible FROM provider_connection_profiles WHERE connection_id=?1 AND contract_version='v1'",
            params![connection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).context("Microgifter entitlement profile is unavailable")?;
    if !matches!(state.as_str(), "active" | "offline" | "grace") {
        return Ok((
            UpdateAuthorizationDecision::Denied,
            Some("connection_not_entitled".to_owned()),
        ));
    }
    if !matches!(subscription.as_deref(), Some("active") | Some("grace")) {
        return Ok((
            UpdateAuthorizationDecision::Denied,
            Some("subscription_not_entitled".to_owned()),
        ));
    }
    let Some(expires) = expires else {
        return Ok((
            UpdateAuthorizationDecision::Denied,
            Some("entitlement_lease_missing".to_owned()),
        ));
    };
    if parse_utc(&expires)? <= Utc::now() {
        return Ok((
            UpdateAuthorizationDecision::Denied,
            Some("entitlement_lease_expired".to_owned()),
        ));
    }
    if eligible != 1 {
        return Ok((
            UpdateAuthorizationDecision::Denied,
            Some("update_eligibility_denied".to_owned()),
        ));
    }
    let channels_json: String = connection.query_row(
        "SELECT allowed_update_channels_json FROM provider_entitlement_leases WHERE connection_id=?1 AND state='accepted' ORDER BY accepted_at_utc DESC LIMIT 1",
        params![connection_id],
        |row| row.get(0),
    )?;
    let channels: Vec<String> = serde_json::from_str(&channels_json)?;
    if !channels.iter().any(|value| value == &request.channel) {
        return Ok((
            UpdateAuthorizationDecision::Denied,
            Some("update_channel_not_entitled".to_owned()),
        ));
    }
    if request.update_class == UpdateClass::Preview && request.channel != "preview" {
        return Ok((
            UpdateAuthorizationDecision::Denied,
            Some("preview_channel_required".to_owned()),
        ));
    }
    Ok((
        UpdateAuthorizationDecision::Authorized,
        Some("valid_entitlement_lease".to_owned()),
    ))
}

fn store_update_authorization(
    connection: &Connection,
    connection_id: Option<&str>,
    request: &AuthorizeUpdateRequest,
    decision: UpdateAuthorizationDecision,
    reason_code: Option<&str>,
    provider_authorization_id: Option<&str>,
    expires_at_utc: Option<&str>,
) -> Result<UpdateAuthorizationSnapshot> {
    let authorization_id = provider_authorization_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO provider_update_authorizations (authorization_id,connection_id,update_id,version,update_class,channel,decision,reason_code,issued_at_utc,expires_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,strftime('%Y-%m-%dT%H:%M:%fZ','now'),?9) ON CONFLICT(connection_id,update_id) DO UPDATE SET authorization_id=excluded.authorization_id,version=excluded.version,update_class=excluded.update_class,channel=excluded.channel,decision=excluded.decision,reason_code=excluded.reason_code,issued_at_utc=excluded.issued_at_utc,expires_at_utc=excluded.expires_at_utc,receipt_submitted_at_utc=NULL",
        params![
            authorization_id,
            connection_id,
            request.update_id,
            request.version,
            request.update_class.as_str(),
            request.channel,
            decision.as_str(),
            reason_code,
            expires_at_utc,
        ],
    )?;
    record_receipt(
        connection,
        connection_id,
        None,
        match decision {
            UpdateAuthorizationDecision::Authorized => "update.authorized",
            UpdateAuthorizationDecision::Denied => "update.denied",
            UpdateAuthorizationDecision::NotRequired => "update.authorization_not_required",
        },
        None,
        None,
        None,
        if decision == UpdateAuthorizationDecision::Denied {
            "denied"
        } else {
            "success"
        },
        reason_code,
        &json!({
            "update_id": request.update_id,
            "version": request.version,
            "update_class": request.update_class,
            "channel": request.channel,
        }),
    )?;
    Ok(UpdateAuthorizationSnapshot {
        authorization_id,
        update_id: request.update_id.clone(),
        version: request.version.clone(),
        update_class: request.update_class,
        channel: request.channel.clone(),
        decision,
        reason_code: reason_code.map(ToOwned::to_owned),
        expires_at_utc: expires_at_utc.map(ToOwned::to_owned),
    })
}

pub(crate) fn ensure_update_download_allowed(
    connection: &Connection,
    update_id: &str,
) -> Result<()> {
    let authorization = connection
        .query_row(
            "SELECT decision,expires_at_utc,update_class FROM provider_update_authorizations WHERE update_id=?1 ORDER BY issued_at_utc DESC LIMIT 1",
            params![update_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()?;
    let Some((decision, expires_at_utc, update_class)) = authorization else {
        // Existing schema-v1 manifests predate entitlement classes. Preserve their signed
        // updater path as independent security-compatible updates until a classified
        // manifest/authorization exists.
        return Ok(());
    };
    ensure!(
        matches!(decision.as_str(), "authorized" | "not_required"),
        "HomeServer update is not authorized by the current entitlement policy"
    );
    if let Some(expires_at_utc) = expires_at_utc {
        ensure!(
            parse_utc(&expires_at_utc)? > Utc::now(),
            "HomeServer update authorization is expired"
        );
    }
    if !matches!(update_class.as_str(), "bootstrap" | "security" | "recovery") {
        ensure_update_install_window(connection)?;
    }
    Ok(())
}

pub(crate) fn ensure_update_install_window(connection: &Connection) -> Result<()> {
    let preferences = update_preferences(connection)?;
    let now = Utc::now();
    match preferences.install_mode.as_str() {
        "install_now" | "when_idle" => Ok(()),
        "defer_until" => {
            let Some(defer_until) = preferences.defer_until_utc else {
                bail!("deferred update date is not configured");
            };
            ensure!(
                parse_utc(&defer_until)? <= now,
                "HomeServer update is deferred until the configured date"
            );
            Ok(())
        }
        "tonight" | "maintenance_window" => {
            let minute = now.hour() as u16 * 60 + now.minute() as u16;
            let start = preferences.maintenance_start_minute_utc;
            let duration = preferences.maintenance_duration_minutes;
            let end = (start as u32 + duration as u32) % 1440;
            let allowed = if start as u32 + (duration as u32) < 1440 {
                minute >= start && minute < start.saturating_add(duration)
            } else {
                minute >= start || minute < end as u16
            };
            ensure!(
                allowed,
                "HomeServer update is outside the maintenance window"
            );
            Ok(())
        }
        _ => bail!("HomeServer update install mode is invalid"),
    }
}

pub(crate) fn record_update_result_receipt(
    connection: &Connection,
    update_id: &str,
    version: &str,
    state: &str,
    failure_code: Option<&str>,
) -> Result<()> {
    let connection_id = connection
        .query_row(
            "SELECT connection_id FROM provider_update_authorizations WHERE update_id=?1 ORDER BY issued_at_utc DESC LIMIT 1",
            params![update_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    record_receipt(
        connection,
        connection_id.as_deref(),
        None,
        match state {
            "succeeded" => "update.installed",
            "rolled_back" => "update.rolled_back",
            _ => "update.failed",
        },
        None,
        None,
        None,
        if state == "succeeded" {
            "success"
        } else {
            "error"
        },
        failure_code,
        &json!({"update_id": update_id, "version": version, "state": state}),
    )
}

async fn submit_pending_update_receipts(state: Arc<AppState>) -> Result<()> {
    let pending = {
        let connection = state.connection()?;
        let mut statement = connection.prepare(
            "SELECT a.authorization_id,a.connection_id,a.update_id,a.version,r.state,r.failure_code FROM provider_update_authorizations a JOIN update_records r ON r.update_id=a.update_id WHERE a.connection_id IS NOT NULL AND a.receipt_submitted_at_utc IS NULL AND r.state IN ('succeeded','failed','rolled_back') ORDER BY a.issued_at_utc LIMIT 20",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (authorization_id, connection_id, update_id, version, result_state, failure_code) in pending
    {
        let payload = json!({
            "authorization_id": authorization_id,
            "update_id": update_id,
            "version": version,
            "result_state": result_state,
            "failure_code": failure_code,
        });
        let response = signed_provider_post::<Value>(
            &state,
            &connection_id,
            MicrogifterProviderAdapter.update_receipt_path(),
            &payload,
        )
        .await;
        if response.is_ok() {
            state.connection()?.execute(
                "UPDATE provider_update_authorizations SET receipt_submitted_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE authorization_id=?1",
                params![authorization_id],
            )?;
        }
    }
    Ok(())
}

fn status_snapshot(connection: &Connection) -> Result<MicrogifterStatusSnapshot> {
    let mut statement = connection.prepare(
        "SELECT p.connection_id FROM provider_connection_profiles p JOIN cloud_connections c ON c.connection_id=p.connection_id WHERE p.provider_key=?1 ORDER BY c.is_default DESC,p.updated_at_utc DESC,p.connection_id",
    )?;
    let connection_ids = statement
        .query_map(params![PROVIDER_KEY], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let connections = connection_ids
        .iter()
        .map(|connection_id| connection_snapshot(connection, connection_id))
        .collect::<Result<Vec<_>>>()?;
    Ok(MicrogifterStatusSnapshot {
        local_operation_available: true,
        provider_key: PROVIDER_KEY.to_owned(),
        contract_version: CONTRACT_VERSION.to_owned(),
        supported_capabilities: CAPABILITY_REGISTRY
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        connections,
        update_preferences: update_preferences(connection)?,
        recent_receipts: recent_receipts(connection, 25)?,
        privacy_boundary: vec![
            "No Knowledge Vault contents".to_owned(),
            "No documents, prompts, conversations, or model responses".to_owned(),
            "No local file names or filesystem contents".to_owned(),
            "No encryption keys, API secrets, pairing credentials, or backup contents".to_owned(),
            "No unrelated provider or wrapper data".to_owned(),
        ],
    })
}

fn connection_snapshot(
    connection: &Connection,
    connection_id: &str,
) -> Result<MicrogifterConnectionSnapshot> {
    let mut snapshot = connection.query_row(
        "SELECT p.connection_id,p.provider_connection_id,c.device_id,p.device_display_name,p.owner_account_id,c.cloud_base_url,p.lifecycle_state,p.contract_version,p.subscription_state,p.entitlement_lease_id,p.entitlement_expires_at_utc,p.update_eligible,p.last_heartbeat_at_utc,c.last_success_utc,p.last_entitlement_refresh_at_utc,p.last_credential_rotation_at_utc,p.last_update_check_at_utc,p.last_update_result,p.replacement_state FROM provider_connection_profiles p JOIN cloud_connections c ON c.connection_id=p.connection_id WHERE p.connection_id=?1 AND p.provider_key=?2",
        params![connection_id, PROVIDER_KEY],
        |row| {
            Ok(MicrogifterConnectionSnapshot {
                connection_id: row.get(0)?,
                provider_connection_id: row.get(1)?,
                device_id: row.get(2)?,
                device_display_name: row.get(3)?,
                owner_account_id: row.get(4)?,
                cloud_base_url: row.get(5)?,
                lifecycle_state: ProviderLifecycleState::from_database(&row.get::<_, String>(6)?),
                contract_version: row.get(7)?,
                subscription_state: row.get::<_, Option<String>>(8)?.map(|value| match value.as_str() {
                    "active" => SubscriptionState::Active,
                    "grace" => SubscriptionState::Grace,
                    "suspended" => SubscriptionState::Suspended,
                    "canceled" => SubscriptionState::Canceled,
                    _ => SubscriptionState::Unknown,
                }),
                entitlement_lease_id: row.get(9)?,
                entitlement_expires_at_utc: row.get(10)?,
                update_eligible: row.get::<_, i64>(11)? == 1,
                update_channel: String::new(),
                last_heartbeat_at_utc: row.get(12)?,
                last_successful_sync_at_utc: row.get(13)?,
                last_entitlement_refresh_at_utc: row.get(14)?,
                last_credential_rotation_at_utc: row.get(15)?,
                last_update_check_at_utc: row.get(16)?,
                last_update_result: row.get(17)?,
                granted_capabilities: Vec::new(),
                assigned_merchant_count: 0,
                assigned_site_count: 0,
                replacement_state: row.get(18)?,
                health_category: String::new(),
            })
        },
    ).context("Microgifter connection profile was not found")?;
    snapshot.update_channel = update_preferences(connection)?.selected_channel;
    snapshot.granted_capabilities = capability_snapshot(connection, connection_id)?;
    snapshot.assigned_merchant_count = assignment_count(connection, connection_id, "merchant")?;
    snapshot.assigned_site_count = assignment_count(connection, connection_id, "site")?;
    snapshot.health_category = match snapshot.lifecycle_state {
        ProviderLifecycleState::Active => "healthy",
        ProviderLifecycleState::Offline
        | ProviderLifecycleState::Grace
        | ProviderLifecycleState::Replacing => "attention",
        ProviderLifecycleState::Unpaired | ProviderLifecycleState::PairingPending => "setup",
        ProviderLifecycleState::Suspended
        | ProviderLifecycleState::Revoked
        | ProviderLifecycleState::Error => "blocked",
    }
    .to_owned();
    Ok(snapshot)
}

fn capability_snapshot(
    connection: &Connection,
    connection_id: &str,
) -> Result<Vec<CapabilitySnapshot>> {
    let mut statement = connection.prepare(
        "SELECT capability_id,grant_state,source,expires_at_utc FROM provider_connection_capabilities WHERE connection_id=?1 ORDER BY capability_id,source",
    )?;
    let rows = statement.query_map(params![connection_id], |row| {
        Ok(CapabilitySnapshot {
            capability_id: row.get(0)?,
            grant_state: row.get(1)?,
            source: row.get(2)?,
            expires_at_utc: row.get(3)?,
        })
    })?;
    let snapshots = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(snapshots)
}

fn recent_receipts(connection: &Connection, limit: usize) -> Result<Vec<ReceiptSnapshot>> {
    let mut statement = connection.prepare(
        "SELECT receipt_id,event_type,result_category,error_category,previous_state,new_state,created_at_utc FROM provider_connection_receipts ORDER BY created_at_utc DESC,receipt_id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map(params![limit.clamp(1, 100) as i64], |row| {
        Ok(ReceiptSnapshot {
            receipt_id: row.get(0)?,
            event_type: row.get(1)?,
            result_category: row.get(2)?,
            error_category: row.get(3)?,
            previous_state: row.get(4)?,
            new_state: row.get(5)?,
            created_at_utc: row.get(6)?,
        })
    })?;
    let receipts = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(receipts)
}

fn update_preferences(connection: &Connection) -> Result<UpdatePreferencesSnapshot> {
    connection.query_row(
        "SELECT selected_channel,install_mode,maintenance_start_minute_utc,maintenance_duration_minutes,defer_until_utc,updated_at_utc FROM homeserver_update_preferences WHERE singleton_id=1",
        [],
        |row| {
            Ok(UpdatePreferencesSnapshot {
                selected_channel: row.get(0)?,
                install_mode: row.get(1)?,
                maintenance_start_minute_utc: row.get::<_, i64>(2)?.clamp(0, 1439) as u16,
                maintenance_duration_minutes: row.get::<_, i64>(3)?.clamp(15, 720) as u16,
                defer_until_utc: row.get(4)?,
                updated_at_utc: row.get(5)?,
            })
        },
    ).map_err(Into::into)
}

fn save_update_preferences(
    connection: &Connection,
    request: UpdatePreferencesRequest,
) -> Result<UpdatePreferencesSnapshot> {
    validate_channel(&request.selected_channel)?;
    ensure!(
        matches!(
            request.install_mode.as_str(),
            "install_now" | "when_idle" | "tonight" | "maintenance_window" | "defer_until"
        ),
        "update install mode is invalid"
    );
    ensure!(
        request.maintenance_start_minute_utc <= 1439,
        "maintenance start minute is invalid"
    );
    ensure!(
        (15..=720).contains(&request.maintenance_duration_minutes),
        "maintenance duration is invalid"
    );
    let defer_until = request
        .defer_until_utc
        .as_deref()
        .map(parse_utc)
        .transpose()?
        .map(|value| value.to_rfc3339());
    if request.install_mode == "defer_until" {
        let defer_is_future = match defer_until.as_deref() {
            Some(value) => parse_utc(value)
                .map(|date| date > Utc::now())
                .unwrap_or(false),
            None => false,
        };
        ensure!(
            defer_is_future,
            "deferred update date must be in the future"
        );
    }
    connection.execute(
        "UPDATE homeserver_update_preferences SET selected_channel=?1,install_mode=?2,maintenance_start_minute_utc=?3,maintenance_duration_minutes=?4,defer_until_utc=?5,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![
            request.selected_channel,
            request.install_mode,
            request.maintenance_start_minute_utc as i64,
            request.maintenance_duration_minutes as i64,
            defer_until,
        ],
    )?;
    update_preferences(connection)
}

fn backfill_existing_microgifter_connections(connection: &Connection) -> Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO provider_connection_profiles (connection_id,provider_key,provider_connection_id,contract_version,lifecycle_state,owner_account_id,device_display_name,connector_version,capability_registry_version,subscription_state,update_eligible,created_at_utc,updated_at_utc) SELECT connection_id,provider_key,NULL,'legacy',CASE state WHEN 'connected' THEN 'active' WHEN 'degraded' THEN 'offline' WHEN 'revoked' THEN 'revoked' ELSE 'error' END,NULL,display_name,?1,'v1','unknown',0,created_at_utc,updated_at_utc FROM cloud_connections WHERE provider_key=?2",
        params![env!("CARGO_PKG_VERSION"), PROVIDER_KEY],
    )?;
    Ok(())
}

fn seed_client_capabilities(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare("SELECT connection_id FROM provider_connection_profiles WHERE provider_key=?1")?;
    let connection_ids = statement
        .query_map(params![PROVIDER_KEY], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for connection_id in connection_ids {
        for capability in CAPABILITY_REGISTRY {
            connection.execute(
                "INSERT OR IGNORE INTO provider_connection_capabilities (connection_id,capability_id,grant_state,source) VALUES (?1,?2,'unavailable','client')",
                params![connection_id, capability],
            )?;
        }
    }
    Ok(())
}

fn record_pairing_pending(
    connection: &Connection,
    request_id: &str,
    cloud_base_url: &str,
    device_display_name: &str,
) -> Result<()> {
    connection.execute(
        "INSERT INTO provider_pairing_attempts (attempt_id,provider_key,request_id,cloud_base_url,device_display_name,state,started_at_utc) VALUES (?1,?2,?3,?4,?5,'pending',strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(request_id) DO UPDATE SET cloud_base_url=excluded.cloud_base_url,device_display_name=excluded.device_display_name,state=CASE WHEN provider_pairing_attempts.state='completed' THEN 'completed' ELSE 'pending' END,error_code=NULL",
        params![Uuid::new_v4().to_string(), PROVIDER_KEY, request_id, cloud_base_url, device_display_name],
    )?;
    Ok(())
}

fn completed_pairing_connection(
    connection: &Connection,
    request_id: &str,
) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT connection_id FROM provider_pairing_attempts WHERE request_id=?1 AND state='completed'",
            params![request_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn mark_pairing_failed(connection: &Connection, request_id: &str, error_code: &str) -> Result<()> {
    connection.execute(
        "UPDATE provider_pairing_attempts SET state='failed',error_code=?1,completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE request_id=?2",
        params![bounded(error_code, 120), request_id],
    )?;
    Ok(())
}

fn phase6a_connection_ids(state: &AppState) -> Result<Vec<String>> {
    let connection = state.connection()?;
    let mut statement = connection.prepare(
        "SELECT connection_id FROM provider_connection_profiles WHERE provider_key=?1 AND contract_version='v1' AND lifecycle_state IN ('active','offline','grace') ORDER BY updated_at_utc",
    )?;
    let rows = statement.query_map(params![PROVIDER_KEY], |row| row.get::<_, String>(0))?;
    let connection_ids = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(connection_ids)
}

fn default_phase6a_connection_id(connection: &Connection) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT p.connection_id FROM provider_connection_profiles p JOIN cloud_connections c ON c.connection_id=p.connection_id WHERE p.provider_key=?1 AND p.contract_version='v1' AND p.lifecycle_state IN ('active','offline','grace') ORDER BY c.is_default DESC,p.updated_at_utc DESC LIMIT 1",
            params![PROVIDER_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn provider_connection_record(
    connection: &Connection,
    connection_id: &str,
) -> Result<ProviderConnectionRecord> {
    connection
        .query_row(
            "SELECT c.connection_id,p.provider_connection_id,c.cloud_base_url,c.device_id,c.credential_key,c.state FROM cloud_connections c JOIN provider_connection_profiles p ON p.connection_id=c.connection_id WHERE c.connection_id=?1 AND c.provider_key=?2",
            params![connection_id, PROVIDER_KEY],
            |row| {
                Ok(ProviderConnectionRecord {
                    provider_connection_id: row.get(1)?,
                    cloud_base_url: row.get(2)?,
                    device_id: row.get(3)?,
                    credential_key: row.get(4)?,
                    state: row.get(5)?,
                })
            },
        )
        .context("Microgifter connection was not found")
}

fn provider_connection_id(connection: &Connection, connection_id: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT provider_connection_id FROM provider_connection_profiles WHERE connection_id=?1",
            params![connection_id],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(Into::into)
}

fn entitlement_expiration(connection: &Connection, connection_id: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT entitlement_expires_at_utc FROM provider_connection_profiles WHERE connection_id=?1",
            params![connection_id],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(Into::into)
}

fn assignment_count(
    connection: &Connection,
    connection_id: &str,
    assignment_type: &str,
) -> Result<u64> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM provider_connection_assignments WHERE connection_id=?1 AND assignment_type=?2 AND state='active'",
        params![connection_id, assignment_type],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

fn set_lifecycle_state(
    connection: &Connection,
    connection_id: &str,
    state: ProviderLifecycleState,
) -> Result<()> {
    connection.execute(
        "UPDATE provider_connection_profiles SET lifecycle_state=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?2",
        params![state.as_str(), connection_id],
    )?;
    Ok(())
}

fn mark_connection_offline(
    connection: &Connection,
    connection_id: &str,
    error_code: &str,
) -> Result<()> {
    let previous = connection
        .query_row(
            "SELECT lifecycle_state FROM provider_connection_profiles WHERE connection_id=?1",
            params![connection_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "error".to_owned());
    if !matches!(previous.as_str(), "revoked" | "suspended" | "replacing") {
        connection.execute(
            "UPDATE provider_connection_profiles SET lifecycle_state='offline',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
            params![connection_id],
        )?;
    }
    mark_cloud_connection_state(connection, connection_id, "degraded", Some(error_code))?;
    record_receipt(
        connection,
        Some(connection_id),
        None,
        "connection.offline",
        None,
        Some(&previous),
        Some("offline"),
        "warning",
        Some(error_code),
        &json!({}),
    )
}

fn mark_cloud_connection_success(connection: &Connection, connection_id: &str) -> Result<()> {
    connection.execute(
        "UPDATE cloud_connections SET state='connected',last_success_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
        params![connection_id],
    )?;
    let lease_valid = connection
        .query_row(
            "SELECT entitlement_expires_at_utc > strftime('%Y-%m-%dT%H:%M:%fZ','now') FROM provider_connection_profiles WHERE connection_id=?1",
            params![connection_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or(0)
        == 1;
    if lease_valid {
        connection.execute(
            "UPDATE provider_connection_profiles SET lifecycle_state=CASE subscription_state WHEN 'grace' THEN 'grace' WHEN 'active' THEN 'active' ELSE lifecycle_state END,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1 AND lifecycle_state='offline'",
            params![connection_id],
        )?;
    }
    Ok(())
}

fn mark_cloud_connection_state(
    connection: &Connection,
    connection_id: &str,
    state: &str,
    last_error: Option<&str>,
) -> Result<()> {
    connection.execute(
        "UPDATE cloud_connections SET state=?1,last_error=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?3",
        params![state, last_error.map(|value| bounded(value, 120)), connection_id],
    )?;
    Ok(())
}

fn cloud_state_for_lifecycle(state: ProviderLifecycleState) -> &'static str {
    match state {
        ProviderLifecycleState::Active => "connected",
        ProviderLifecycleState::Revoked => "revoked",
        _ => "degraded",
    }
}

fn link_replacement_to_new_connection(
    connection: &Connection,
    replacement_id: &str,
    new_connection_id: &str,
    new_device_id: &str,
) -> Result<()> {
    let changed = connection.execute(
        "UPDATE provider_device_replacements SET new_connection_id=?1,new_device_id=?2,state='paired',failure_code=NULL WHERE replacement_id=?3 AND state='pending'",
        params![new_connection_id, new_device_id, replacement_id],
    )?;
    ensure!(changed == 1, "device replacement request is not pending");
    connection.execute(
        "UPDATE provider_connection_profiles SET lifecycle_state='replacing',replacement_state='activating',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
        params![new_connection_id],
    )?;
    Ok(())
}

fn machine_fingerprint_hash(installation_id: &str) -> Result<String> {
    let credential_key = format!("{installation_id}:phase6a:machine-anchor");
    let entry = credential_entry(&credential_key)?;
    let anchor = match entry.get_password() {
        Ok(value) => value,
        Err(keyring::Error::NoEntry) => {
            let mut bytes = [0_u8; 32];
            OsRng.fill_bytes(&mut bytes);
            let value = URL_SAFE_NO_PAD.encode(bytes);
            entry
                .set_password(&value)
                .context("unable to store the machine identity anchor")?;
            value
        }
        Err(error) => return Err(error).context("unable to read the machine identity anchor"),
    };
    let mut hasher = Sha256::new();
    hasher.update(installation_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(anchor.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

fn ensure_no_duplicate_device(
    connection: &Connection,
    device_id: &str,
    machine_fingerprint_hash: &str,
) -> Result<()> {
    let prior = connection
        .query_row(
            "SELECT machine_fingerprint_hash FROM provider_device_identity_observations WHERE provider_key=?1 AND device_id=?2 AND disposition='trusted' ORDER BY observed_at_utc DESC LIMIT 1",
            params![PROVIDER_KEY, device_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if prior
        .as_deref()
        .is_some_and(|value| value != machine_fingerprint_hash)
    {
        connection.execute(
            "INSERT INTO provider_device_identity_observations (observation_id,provider_key,device_id,installation_id,machine_fingerprint_hash,connection_id,disposition,observed_at_utc) VALUES (?1,?2,?3,?4,?5,NULL,'duplicate',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![
                Uuid::new_v4().to_string(),
                PROVIDER_KEY,
                device_id,
                database::installation_id(connection)?,
                machine_fingerprint_hash,
            ],
        )?;
        bail!("duplicate HomeServer device identity detected; use device replacement");
    }
    Ok(())
}

fn validate_entitlement_signing_key(value: &PairingSigningKey) -> Result<()> {
    validate_identifier(&value.key_id, 120, "entitlement signing key id")?;
    let bytes = decode_base64(&value.public_key_base64)?;
    ensure!(
        bytes.len() == 32,
        "entitlement public key length is invalid"
    );
    if let Some(not_before) = &value.not_before_utc {
        parse_utc(not_before)?;
    }
    if let Some(not_after) = &value.not_after_utc {
        parse_utc(not_after)?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "durable receipt fields mirror the persisted audit schema"
)]
fn record_receipt(
    connection: &Connection,
    connection_id: Option<&str>,
    device_id: Option<&str>,
    event_type: &str,
    request_id: Option<&str>,
    previous_state: Option<&str>,
    new_state: Option<&str>,
    result_category: &str,
    error_category: Option<&str>,
    metadata: &Value,
) -> Result<()> {
    ensure!(
        matches!(result_category, "success" | "warning" | "error" | "denied"),
        "receipt result category is invalid"
    );
    let sanitized = sanitize_receipt_metadata(metadata);
    connection.execute(
        "INSERT INTO provider_connection_receipts (receipt_id,provider_key,connection_id,device_id,event_type,request_id,previous_state,new_state,result_category,error_category,metadata_json,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            Uuid::new_v4().to_string(),
            PROVIDER_KEY,
            connection_id,
            device_id,
            bounded(event_type, 120),
            request_id.map(|value| bounded(value, 190)),
            previous_state.map(|value| bounded(value, 60)),
            new_state.map(|value| bounded(value, 60)),
            result_category,
            error_category.map(|value| bounded(value, 120)),
            serde_json::to_string(&sanitized)?,
        ],
    )?;
    Ok(())
}

fn sanitize_receipt_metadata(value: &Value) -> Value {
    fn sanitize(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut result = serde_json::Map::new();
                for (key, value) in object {
                    let normalized = key.to_lowercase();
                    if [
                        "secret",
                        "token",
                        "sync_code",
                        "pairing_code",
                        "credential",
                        "private_key",
                        "authorization",
                        "prompt",
                        "conversation",
                        "document",
                        "file_name",
                        "filesystem",
                        "backup_content",
                    ]
                    .iter()
                    .any(|blocked| normalized.contains(blocked))
                    {
                        result.insert(key.clone(), Value::String("[redacted]".to_owned()));
                    } else {
                        result.insert(key.clone(), sanitize(value));
                    }
                }
                Value::Object(result)
            }
            Value::Array(values) => Value::Array(values.iter().take(100).map(sanitize).collect()),
            Value::String(value) => Value::String(value.chars().take(500).collect()),
            other => other.clone(),
        }
    }
    sanitize(value)
}

fn credential_entry(credential_key: &str) -> Result<Entry> {
    Entry::new(CREDENTIAL_SERVICE, credential_key)
        .context("unable to open the HomeServer credential vault")
}

fn save_secrets(credential_key: &str, secrets: &StoredDeviceSecrets) -> Result<()> {
    let payload = serde_json::to_string(secrets)?;
    credential_entry(credential_key)?
        .set_password(&payload)
        .context("unable to save HomeServer provider credentials")
}

fn load_secrets(credential_key: &str) -> Result<StoredDeviceSecrets> {
    let payload = credential_entry(credential_key)?
        .get_password()
        .context("HomeServer provider credentials are unavailable")?;
    serde_json::from_str(&payload).context("HomeServer provider credentials are invalid")
}

fn delete_secrets(credential_key: &str) -> Result<()> {
    match credential_entry(credential_key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to delete HomeServer provider credentials"),
    }
}

fn normalize_cloud_base_url(value: &str) -> Result<String> {
    let url = Url::parse(value.trim()).context("Microgifter cloud URL is invalid")?;
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Microgifter cloud URL cannot contain credentials"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "Microgifter cloud URL cannot contain a query or fragment"
    );
    let host = url
        .host_str()
        .context("Microgifter cloud URL host is required")?;
    let loopback = host
        .parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or_else(|_| host.eq_ignore_ascii_case("localhost"));
    ensure!(
        url.scheme() == "https" || (loopback && url.scheme() == "http"),
        "Microgifter cloud URL must use HTTPS"
    );
    ensure!(
        url.path() == "/" || url.path().is_empty(),
        "Microgifter cloud URL cannot contain a path"
    );
    let mut normalized = url;
    normalized.set_path("");
    Ok(normalized.as_str().trim_end_matches('/').to_owned())
}

fn canonical_json_string(value: &Value) -> Result<String> {
    fn canonicalize(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                let mut result = serde_json::Map::new();
                for key in keys {
                    result.insert(key.clone(), canonicalize(&object[key]));
                }
                Value::Object(result)
            }
            Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
            _ => value.clone(),
        }
    }
    Ok(serde_json::to_string(&canonicalize(value))?)
}

fn canonical_request(
    method: &Method,
    path: &str,
    timestamp: &str,
    nonce: &str,
    body: &str,
) -> String {
    let body_hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    format!(
        "{}\n{}\n{}\n{}\n{}",
        method.as_str(),
        path,
        timestamp,
        nonce,
        body_hash
    )
}

fn decode_base64(value: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| STANDARD.decode(value))
        .context("base64 value is invalid")
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("UTC timestamp '{value}' is invalid"))
}

fn validate_channel(value: &str) -> Result<()> {
    ensure!(
        matches!(value, "stable" | "beta" | "preview"),
        "update channel is invalid"
    );
    Ok(())
}

fn validate_version(value: &str) -> Result<()> {
    semver::Version::parse(value).context("HomeServer version is invalid")?;
    Ok(())
}

fn validate_uuid(value: &str, label: &str) -> Result<()> {
    ensure!(Uuid::parse_str(value).is_ok(), "{label} is invalid");
    Ok(())
}

fn validate_request_id(value: &str) -> Result<String> {
    validate_identifier(value, 190, "request id")?;
    Ok(value.to_owned())
}

fn validate_identifier(value: &str, maximum: usize, label: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= maximum
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_.:-".contains(character)),
        "{label} is invalid"
    );
    Ok(())
}

fn sanitize_text(value: &str, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        !value.is_empty()
            && value.chars().count() <= maximum
            && value.chars().all(|character| !character.is_control()),
        "{label} is invalid"
    );
    Ok(value.to_owned())
}

fn sanitize_optional_text(
    value: Option<&str>,
    maximum: usize,
    label: &str,
) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    sanitize_text(value, maximum, label).map(Some)
}

fn bounded(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn public_provider_error(error: &anyhow::Error) -> String {
    let text = error.to_string().to_lowercase();
    if text.contains("expired") && text.contains("code") {
        "microgifter_sync_code_expired"
    } else if text.contains("already") && text.contains("used") {
        "microgifter_sync_code_used"
    } else if text.contains("invalid") && text.contains("code") {
        "microgifter_sync_code_invalid"
    } else if text.contains("401") || text.contains("403") || text.contains("credential") {
        "microgifter_credentials_rejected"
    } else if text.contains("timed out") || text.contains("connect") || text.contains("dns") {
        "microgifter_cloud_offline"
    } else if text.contains("duplicate") {
        "microgifter_duplicate_device_identity"
    } else {
        "microgifter_provider_request_failed"
    }
    .to_owned()
}

fn public_entitlement_error(error: &anyhow::Error) -> String {
    let text = error.to_string().to_lowercase();
    if text.contains("signature") {
        "microgifter_entitlement_signature_invalid"
    } else if text.contains("signing key") || text.contains("public key") {
        "microgifter_entitlement_key_unknown"
    } else if text.contains("expired") {
        "microgifter_entitlement_expired"
    } else if text.contains("device") {
        "microgifter_entitlement_device_mismatch"
    } else if text.contains("connection") {
        "microgifter_entitlement_connection_mismatch"
    } else if text.contains("capability") {
        "microgifter_capability_unsupported"
    } else {
        "microgifter_entitlement_rejected"
    }
    .to_owned()
}

fn task_error(error: tokio::task::JoinError) -> (HttpStatusCode, Json<ApiError>) {
    internal_error("microgifter_task_failed", anyhow!(error))
}

fn action_error(error: &'static str, source: anyhow::Error) -> (HttpStatusCode, Json<ApiError>) {
    (
        HttpStatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error,
            message: bounded(&source.to_string(), 500),
        }),
    )
}

fn internal_error(error: &'static str, source: anyhow::Error) -> (HttpStatusCode, Json<ApiError>) {
    warn!(?source, error, "Phase 6A request failed");
    (
        HttpStatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error,
            message: "HomeServer could not complete the Microgifter connection request.".to_owned(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_registry_is_unique_and_stable() {
        let mut values = CAPABILITY_REGISTRY.to_vec();
        values.sort_unstable();
        values.dedup();
        assert_eq!(values.len(), CAPABILITY_REGISTRY.len());
        assert!(CAPABILITY_REGISTRY.contains(&"pairing.v1"));
        assert!(CAPABILITY_REGISTRY.contains(&"signed-updates.v1"));
        assert!(CAPABILITY_REGISTRY.contains(&"device-replacement.v1"));
    }

    #[test]
    fn security_updates_do_not_depend_on_pairing() {
        assert!(UpdateClass::Bootstrap.always_available());
        assert!(UpdateClass::Security.always_available());
        assert!(UpdateClass::Recovery.always_available());
        assert!(!UpdateClass::Feature.always_available());
        assert!(!UpdateClass::Preview.always_available());
    }

    #[test]
    fn receipt_metadata_redacts_private_values() {
        let sanitized = sanitize_receipt_metadata(&json!({
            "device_id": "safe",
            "device_token": "secret",
            "prompt": "private",
            "nested": {"credential_reference": "private", "count": 2}
        }));
        assert_eq!(sanitized["device_id"], "safe");
        assert_eq!(sanitized["device_token"], "[redacted]");
        assert_eq!(sanitized["prompt"], "[redacted]");
        assert_eq!(sanitized["nested"]["credential_reference"], "[redacted]");
        assert_eq!(sanitized["nested"]["count"], 2);
    }

    #[test]
    fn entitlement_signature_round_trip_uses_exact_claims() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let claims = EntitlementLeaseClaims {
            schema_version: 1,
            lease_id: "lease-1".to_owned(),
            provider_id: PROVIDER_KEY.to_owned(),
            account_id: "account-1".to_owned(),
            connection_id: "connection-1".to_owned(),
            device_id: "device-1".to_owned(),
            issued_at_utc: Utc::now().to_rfc3339(),
            not_before_utc: Utc::now().to_rfc3339(),
            expires_at_utc: (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            subscription_state: SubscriptionState::Active,
            granted_capabilities: vec!["pairing.v1".to_owned()],
            denied_capabilities: vec![],
            merchant_scope: vec![],
            site_scope: vec![],
            device_allowance: json!({"maximum": 1}),
            update_eligibility: true,
            allowed_update_channels: vec!["stable".to_owned()],
            minimum_homeserver_version: None,
            signing_key_id: "test-key".to_owned(),
        };
        let bytes = serde_json::to_vec(&claims).expect("claims");
        let signature = signing_key.sign(&bytes);
        signing_key
            .verifying_key()
            .verify(&bytes, &signature)
            .expect("signature");
    }

    #[test]
    fn provider_lifecycle_states_remain_distinct() {
        let values = [
            ProviderLifecycleState::Unpaired,
            ProviderLifecycleState::PairingPending,
            ProviderLifecycleState::Active,
            ProviderLifecycleState::Offline,
            ProviderLifecycleState::Grace,
            ProviderLifecycleState::Suspended,
            ProviderLifecycleState::Revoked,
            ProviderLifecycleState::Replacing,
            ProviderLifecycleState::Error,
        ];
        let mut names = values
            .iter()
            .map(|value| value.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), values.len());
    }
}
