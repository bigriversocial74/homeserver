use crate::{software_authority, update, update_store, AppState};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::{
    engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD, Engine as _,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use keyring::Entry;
use microgifter_homeserver_core::{
    SignedUpdateManifest, UpdateActionResult, UpdateChannel, UpdateInstallerContract,
    UpdateManifestPayload, UpdateState, PRODUCT_NAME, UPDATE_MANIFEST_SCHEMA_VERSION,
};
use reqwest::Url;
use rusqlite::{params, Connection, OptionalExtension};
use semver::Version;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;
use tracing::{info, warn};
use uuid::Uuid;
use zeroize::Zeroize;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0018_vp3_activation_update_client.sql");
const MIGRATION_KEY: &str = "0018_vp3_activation_update_client";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";
const CREDENTIAL_KEY: &str = "vp3-software-authority-device-credential";
const MAX_JSON_BYTES: usize = 64 * 1024;
const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;
const MIN_INSTALLER_BYTES: u64 = 1_000_000;

#[derive(Debug, Serialize)]
pub struct Vp3ClientSnapshot {
    pub configured: bool,
    pub activation_state: String,
    pub account_id: Option<i64>,
    pub device_public_id: Option<String>,
    pub license_public_id: Option<String>,
    pub lease_public_id: Option<String>,
    pub lease_expires_at_utc: Option<String>,
    pub last_heartbeat_at_utc: Option<String>,
    pub last_manifest_checked_at_utc: Option<String>,
    pub last_error_code: Option<String>,
    pub credential_in_os_vault: bool,
    pub authority: software_authority::SoftwareAuthoritySnapshot,
}

#[derive(Debug, Deserialize)]
pub struct ActivateVp3Request {
    account_id: i64,
    device_public_id: String,
    license_public_id: Option<String>,
    device_fingerprint: String,
    credential: String,
    enrollment_code: String,
    confirmation: String,
}

impl Drop for ActivateVp3Request {
    fn drop(&mut self) {
        self.credential.zeroize();
        self.enrollment_code.zeroize();
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfirmationRequest {
    confirmation: String,
}

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct LeaseEnvelope {
    lease_public_id: String,
    document: String,
    signature: String,
    key_id: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct ActivationEnvelope {
    device_public_id: String,
    status: String,
    lease: LeaseEnvelope,
}

#[derive(Debug, Deserialize)]
struct HeartbeatEnvelope {
    status: String,
    software_authority: String,
    license_public_id: String,
    update_channel: String,
}

#[derive(Debug, Deserialize)]
struct ManifestEnvelope {
    available: bool,
    current_version: Option<String>,
    update_channel: String,
    release_public_id: Option<String>,
    version: Option<String>,
    channel: Option<String>,
    emergency_override: Option<bool>,
    manifest: Option<String>,
    signature: Option<String>,
    signature_algorithm: Option<String>,
    signing_key_id: Option<String>,
    manifest_hash: Option<String>,
    installer_authorization: Option<InstallerAuthorization>,
}

#[derive(Debug, Deserialize)]
struct InstallerAuthorization {
    token: String,
    expires_at: String,
    download_path: String,
}

#[derive(Debug)]
struct ClientIdentity {
    account_id: i64,
    device_public_id: String,
    license_public_id: Option<String>,
    device_fingerprint: String,
    lease_expires_at_utc: Option<String>,
}

#[derive(Debug)]
struct VerifiedLease {
    lease_public_id: String,
    expires_at_utc: String,
    update_channel: String,
    document_hash: String,
    signature_hash: String,
    key_id: String,
}

#[derive(Debug)]
struct VerifiedRelease {
    update_id: String,
    release_public_id: String,
    version: String,
    channel: String,
    manifest_document: String,
    manifest_signature: String,
    signing_key_id: String,
    manifest_hash: String,
    file_name: String,
    sha256: String,
    size_bytes: u64,
    authenticode_thumbprint: String,
    minimum_version: Option<String>,
    release_notes_hash: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    let applied: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    if applied == 0 {
        connection.execute_batch(MIGRATION)?;
    }
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "VP3 activation client migration is not registered exactly once"
    );
    let singleton_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM vp3_authority_client_state WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        singleton_count == 1,
        "VP3 activation client state is unavailable"
    );
    let _: i64 = connection.query_row("SELECT COUNT(*) FROM vp3_update_bindings", [], |row| {
        row.get(0)
    })?;
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM vp3_update_bindings WHERE update_id NOT IN (SELECT update_id FROM vp3_update_bindings ORDER BY checked_at_utc DESC,update_id DESC LIMIT 100)",
        [],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/software-authority/vp3", get(status_handler))
        .route(
            "/v1/software-authority/vp3/activate",
            post(activate_handler),
        )
        .route(
            "/v1/software-authority/vp3/heartbeat",
            post(heartbeat_handler),
        )
        .route("/v1/software-authority/vp3/lease", post(lease_handler))
        .route(
            "/v1/software-authority/vp3/check-update",
            post(check_update_handler),
        )
        .route(
            "/v1/software-authority/vp3/download-update",
            post(download_update_handler),
        )
        .route(
            "/v1/software-authority/vp3/submit-receipts",
            post(submit_receipts_handler),
        )
        .route(
            "/v1/software-authority/vp3/disconnect",
            post(disconnect_handler),
        )
        .layer(DefaultBodyLimit::max(MAX_JSON_BYTES))
        .with_state(state)
}

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut interval = tokio::time::interval_at(start, Duration::from_secs(5 * 60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if !is_active(&state) {
                    continue;
                }
                if let Err(error) = refresh_lease_if_needed(&state).await {
                    warn!(?error, "VP3 entitlement lease refresh failed");
                }
                if let Err(error) = heartbeat(&state).await {
                    warn!(?error, "VP3 software-authority heartbeat failed");
                }
                if let Err(error) = submit_pending_receipts(&state).await {
                    warn!(?error, "VP3 software-authority receipt submission failed");
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

async fn status_handler(State(state): State<Arc<AppState>>) -> ApiResult<Vp3ClientSnapshot> {
    snapshot(&state)
        .map(Json)
        .map_err(|error| internal_error("vp3_status_failed", error))
}

async fn activate_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ActivateVp3Request>,
) -> ApiResult<Vp3ClientSnapshot> {
    activate(&state, request)
        .await
        .and_then(|_| snapshot(&state))
        .map(Json)
        .map_err(|error| action_error("vp3_activation_failed", error))
}

async fn heartbeat_handler(State(state): State<Arc<AppState>>) -> ApiResult<Vp3ClientSnapshot> {
    heartbeat(&state)
        .await
        .and_then(|_| snapshot(&state))
        .map(Json)
        .map_err(|error| action_error("vp3_heartbeat_failed", error))
}

async fn lease_handler(State(state): State<Arc<AppState>>) -> ApiResult<Vp3ClientSnapshot> {
    refresh_lease(&state)
        .await
        .and_then(|_| snapshot(&state))
        .map(Json)
        .map_err(|error| action_error("vp3_lease_refresh_failed", error))
}

async fn check_update_handler(State(state): State<Arc<AppState>>) -> ApiResult<UpdateActionResult> {
    check_for_update(&state)
        .await
        .map(Json)
        .map_err(|error| action_error("vp3_update_check_failed", error))
}

async fn download_update_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<UpdateActionResult> {
    download_update(&state)
        .await
        .map(Json)
        .map_err(|error| action_error("vp3_update_download_failed", error))
}

async fn submit_receipts_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Vp3ClientSnapshot> {
    submit_pending_receipts(&state)
        .await
        .and_then(|_| snapshot(&state))
        .map(Json)
        .map_err(|error| action_error("vp3_receipt_submission_failed", error))
}

async fn disconnect_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfirmationRequest>,
) -> ApiResult<Vp3ClientSnapshot> {
    ensure!(
        request.confirmation == "DISCONNECT VP3",
        "type DISCONNECT VP3 to remove local VP3 software authority"
    )
    .map_err(|error| action_error("vp3_disconnect_confirmation_required", error))?;
    disconnect(&state)
        .and_then(|_| snapshot(&state))
        .map(Json)
        .map_err(|error| action_error("vp3_disconnect_failed", error))
}

fn snapshot(state: &AppState) -> Result<Vp3ClientSnapshot> {
    let connection = state.connection()?;
    let authority = software_authority::status_snapshot(&connection)?;
    connection.query_row(
        "SELECT account_id,device_public_id,license_public_id,activation_state,lease_public_id,lease_expires_at_utc,last_heartbeat_at_utc,last_manifest_checked_at_utc,last_error_code FROM vp3_authority_client_state WHERE singleton_id=1",
        [],
        |row| {
            let activation_state: String = row.get(3)?;
            Ok(Vp3ClientSnapshot {
                configured: activation_state == "active",
                activation_state,
                account_id: row.get(0)?,
                device_public_id: row.get(1)?,
                license_public_id: row.get(2)?,
                lease_public_id: row.get(4)?,
                lease_expires_at_utc: row.get(5)?,
                last_heartbeat_at_utc: row.get(6)?,
                last_manifest_checked_at_utc: row.get(7)?,
                last_error_code: row.get(8)?,
                credential_in_os_vault: load_credential().is_ok(),
                authority,
            })
        },
    )
    .context("unable to load VP3 client state")
}

fn is_active(state: &AppState) -> bool {
    state
        .connection()
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT activation_state FROM vp3_authority_client_state WHERE singleton_id=1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .ok()
        })
        .is_some_and(|value| value == "active")
}

async fn activate(state: &AppState, mut request: ActivateVp3Request) -> Result<()> {
    ensure!(
        request.confirmation == "ACTIVATE VP3",
        "type ACTIVATE VP3 to authorize the VP3 cutover"
    );
    ensure!(request.account_id > 0, "VP3 account ID is required");
    validate_public_id(&request.device_public_id, "VP3 device public ID")?;
    ensure!(
        valid_sha256(&request.device_fingerprint),
        "VP3 device fingerprint must be a SHA-256 value"
    );
    ensure!(
        (32..=256).contains(&request.credential.len())
            && request
                .credential
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character)),
        "VP3 device credential is invalid"
    );
    ensure!(
        (8..=256).contains(&request.enrollment_code.len()),
        "VP3 enrollment code is invalid"
    );
    configured_verifying_key(
        &state.config.vp3_lease_public_key_base64,
        "VP3 lease public key",
    )?;
    configured_verifying_key(
        &state.config.vp3_release_public_key_base64,
        "VP3 release public key",
    )?;

    save_credential(&request.credential)?;
    {
        let connection = state.connection()?;
        connection.execute(
            "UPDATE vp3_authority_client_state SET account_id=?1,device_public_id=?2,license_public_id=?3,device_fingerprint=?4,credential_key=?5,activation_state='activating',last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
            params![
                request.account_id,
                request.device_public_id,
                request.license_public_id,
                request.device_fingerprint.to_lowercase(),
                CREDENTIAL_KEY,
            ],
        )?;
    }

    let activation = post_device::<ActivationEnvelope>(
        state,
        "activate.php",
        &request.credential,
        json!({
            "account_id": request.account_id,
            "device_public_id": request.device_public_id,
            "enrollment_code": request.enrollment_code,
            "request_id": request_id("activate"),
        }),
    )
    .await;

    let lease = match activation {
        Ok(activation) => {
            ensure!(
                activation.status == "paired",
                "VP3 device activation did not reach paired status"
            );
            ensure!(
                activation.device_public_id == request.device_public_id,
                "VP3 activation returned a different device identity"
            );
            activation.lease
        }
        Err(activation_error) => {
            // Recovery path for a response lost after the one-time enrollment code was consumed.
            match post_device::<LeaseEnvelope>(
                state,
                "lease.php",
                &request.credential,
                json!({
                    "device_public_id": request.device_public_id,
                    "request_id": request_id("activation-recovery-lease"),
                }),
            )
            .await
            {
                Ok(lease) => lease,
                Err(_) => {
                    let _ = delete_credential();
                    mark_error(state, "vp3_activation_rejected")?;
                    return Err(activation_error);
                }
            }
        }
    };

    let verified = verify_lease(
        state,
        request.account_id,
        &request.device_public_id,
        &request.device_fingerprint,
        lease,
    )?;
    persist_lease(state, &verified)?;
    heartbeat_with_credential(state, &request.credential).await?;
    request.credential.zeroize();
    request.enrollment_code.zeroize();
    Ok(())
}

async fn heartbeat(state: &AppState) -> Result<()> {
    let credential = load_credential()?;
    heartbeat_with_credential(state, &credential).await
}

async fn heartbeat_with_credential(state: &AppState, credential: &str) -> Result<()> {
    let identity = identity(state)?;
    let response = post_device::<HeartbeatEnvelope>(
        state,
        "heartbeat.php",
        credential,
        json!({
            "device_public_id": identity.device_public_id,
            "device_fingerprint": identity.device_fingerprint,
            "health": {
                "software_version": env!("CARGO_PKG_VERSION"),
                "mcp_version": env!("CARGO_PKG_VERSION"),
                "mcp_available": true,
                "pairing_available": true
            },
            "request_id": request_id("heartbeat"),
        }),
    )
    .await?;
    ensure!(
        response.software_authority == "vp3",
        "VP3 heartbeat did not assert VP3 software authority"
    );
    ensure!(
        matches!(response.status.as_str(), "online" | "degraded"),
        "VP3 heartbeat returned an invalid device state"
    );
    ensure!(
        matches!(response.update_channel.as_str(), "stable" | "security"),
        "VP3 heartbeat returned an invalid update channel"
    );
    let now = Utc::now().to_rfc3339();
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE vp3_authority_client_state SET license_public_id=?1,activation_state='active',last_heartbeat_at_utc=?2,last_error_code=NULL,updated_at_utc=?2 WHERE singleton_id=1",
        params![response.license_public_id, now],
    )?;
    transaction.execute(
        "UPDATE homeserver_software_authority SET current_authority='vp3',target_authority='vp3',cutover_state='active',vp3_device_id=?1,vp3_license_id=?2,update_eligible=1,allowed_update_channels_json=?3,last_vp3_heartbeat_at_utc=?4,last_error_code=NULL,updated_at_utc=?4 WHERE singleton_id=1",
        params![
            identity.device_public_id,
            response.license_public_id,
            serde_json::to_string(&vec![response.update_channel])?,
            now,
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

async fn refresh_lease_if_needed(state: &AppState) -> Result<()> {
    let identity = identity(state)?;
    let refresh = match identity.lease_expires_at_utc.as_deref() {
        Some(value) => parse_utc(value)? <= Utc::now() + ChronoDuration::minutes(10),
        None => true,
    };
    if refresh {
        refresh_lease(state).await?;
    }
    Ok(())
}

async fn refresh_lease(state: &AppState) -> Result<()> {
    let identity = identity(state)?;
    let credential = load_credential()?;
    let lease = post_device::<LeaseEnvelope>(
        state,
        "lease.php",
        &credential,
        json!({
            "device_public_id": identity.device_public_id,
            "request_id": request_id("lease"),
        }),
    )
    .await?;
    let verified = verify_lease(
        state,
        identity.account_id,
        &identity.device_public_id,
        &identity.device_fingerprint,
        lease,
    )?;
    persist_lease(state, &verified)
}

fn verify_lease(
    state: &AppState,
    account_id: i64,
    device_public_id: &str,
    device_fingerprint: &str,
    lease: LeaseEnvelope,
) -> Result<VerifiedLease> {
    ensure!(
        lease.key_id == state.config.vp3_lease_key_id,
        "VP3 entitlement lease signing key is not trusted"
    );
    let document = URL_SAFE_NO_PAD
        .decode(&lease.document)
        .context("VP3 entitlement lease document encoding is invalid")?;
    verify_ed25519(
        &state.config.vp3_lease_public_key_base64,
        &document,
        &lease.signature,
        "VP3 entitlement lease",
    )?;
    let claims: Value =
        serde_json::from_slice(&document).context("VP3 entitlement lease claims are invalid")?;
    ensure!(
        claims.get("iss").and_then(Value::as_str) == Some("vp3.me"),
        "VP3 entitlement lease issuer is invalid"
    );
    ensure!(
        claims.get("sub").and_then(Value::as_str) == Some(device_public_id),
        "VP3 entitlement lease device identity is invalid"
    );
    ensure!(
        claims.get("account_id").and_then(Value::as_i64) == Some(account_id),
        "VP3 entitlement lease account identity is invalid"
    );
    ensure!(
        claims.get("device_fingerprint").and_then(Value::as_str) == Some(device_fingerprint),
        "VP3 entitlement lease fingerprint is invalid"
    );
    let expires = claims
        .get("exp")
        .and_then(Value::as_i64)
        .context("VP3 entitlement lease expiration is missing")?;
    ensure!(
        expires > Utc::now().timestamp(),
        "VP3 entitlement lease is expired"
    );
    let response_expires = parse_database_or_rfc3339(&lease.expires_at)?;
    ensure!(
        response_expires.timestamp() == expires,
        "VP3 entitlement lease expiration evidence does not match"
    );
    let update_channel = claims
        .get("update_channel")
        .and_then(Value::as_str)
        .context("VP3 entitlement lease update channel is missing")?;
    ensure!(
        matches!(update_channel, "stable" | "security"),
        "VP3 entitlement lease update channel is invalid"
    );
    Ok(VerifiedLease {
        lease_public_id: lease.lease_public_id,
        expires_at_utc: response_expires.to_rfc3339(),
        update_channel: update_channel.to_owned(),
        document_hash: hex::encode(Sha256::digest(&document)),
        signature_hash: hex::encode(Sha256::digest(lease.signature.as_bytes())),
        key_id: lease.key_id,
    })
}

fn persist_lease(state: &AppState, lease: &VerifiedLease) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE vp3_authority_client_state SET activation_state='active',lease_public_id=?1,lease_key_id=?2,lease_document_hash=?3,lease_signature_hash=?4,lease_expires_at_utc=?5,last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![
            lease.lease_public_id,
            lease.key_id,
            lease.document_hash,
            lease.signature_hash,
            lease.expires_at_utc,
        ],
    )?;
    transaction.execute(
        "UPDATE homeserver_software_authority SET vp3_lease_id=?1,vp3_lease_expires_at_utc=?2,update_eligible=1,allowed_update_channels_json=?3,last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![
            lease.lease_public_id,
            lease.expires_at_utc,
            serde_json::to_string(&vec![lease.update_channel.clone()])?,
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

async fn check_for_update(state: &AppState) -> Result<UpdateActionResult> {
    refresh_lease_if_needed(state).await?;
    heartbeat(state).await?;
    update_store::begin_check(&state.connection()?)?;
    let response = fetch_manifest(state).await?;
    if !response.available {
        update_store::record_current(&state.connection()?)?;
        return Ok(UpdateActionResult {
            status: update_store::status(
                &state.connection()?,
                &format!(
                    "{}/api/homeserver/v1/manifest.php",
                    state.config.vp3_base_url
                ),
                state.config.update_plan_path().exists(),
            )?,
            message: "HomeServer is current under VP3 software authority.".to_owned(),
            restart_required: false,
        });
    }
    let release = verify_release(state, &response)?;
    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let target = Version::parse(&release.version)?;
    if target <= current {
        update_store::record_current(&state.connection()?)?;
        return Ok(UpdateActionResult {
            status: update_store::status(
                &state.connection()?,
                &format!(
                    "{}/api/homeserver/v1/manifest.php",
                    state.config.vp3_base_url
                ),
                state.config.update_plan_path().exists(),
            )?,
            message: "HomeServer is current under VP3 software authority.".to_owned(),
            restart_required: false,
        });
    }
    let derived = SignedUpdateManifest {
        key_id: format!("vp3-document:{}", release.signing_key_id),
        payload: UpdateManifestPayload {
            schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
            product: PRODUCT_NAME.to_owned(),
            channel: UpdateChannel::Stable,
            version: release.version.clone(),
            minimum_version: release.minimum_version.clone(),
            published_at_utc: Utc::now(),
            release_notes: format!(
                "VP3 signed release notes hash: {}",
                release.release_notes_hash
            ),
            installer: UpdateInstallerContract {
                url: format!(
                    "{}/api/homeserver/v1/installer-download.php",
                    state.config.vp3_base_url
                ),
                file_name: release.file_name.clone(),
                size_bytes: release.size_bytes,
                sha256: release.sha256.clone(),
                authenticode_thumbprint: release.authenticode_thumbprint.clone(),
            },
        },
        signature: release.manifest_signature.clone(),
    };
    let manifest_url = format!(
        "{}/api/homeserver/v1/manifest.php",
        state.config.vp3_base_url
    );
    update_store::save_available(
        &state.connection()?,
        &release.update_id,
        &manifest_url,
        &derived,
    )?;
    persist_release_binding(state, &release)?;
    Ok(UpdateActionResult {
        status: update_store::status(
            &state.connection()?,
            &manifest_url,
            state.config.update_plan_path().exists(),
        )?,
        message: format!(
            "HomeServer {} is available from verified VP3 software authority.",
            release.version
        ),
        restart_required: false,
    })
}

async fn fetch_manifest(state: &AppState) -> Result<ManifestEnvelope> {
    let identity = identity(state)?;
    let credential = load_credential()?;
    post_device::<ManifestEnvelope>(
        state,
        "manifest.php",
        &credential,
        json!({
            "device_public_id": identity.device_public_id,
            "current_version": env!("CARGO_PKG_VERSION"),
            "platform": "windows",
            "architecture": "x86_64",
            "request_id": request_id("manifest"),
        }),
    )
    .await
}

fn verify_release(state: &AppState, response: &ManifestEnvelope) -> Result<VerifiedRelease> {
    ensure!(
        response.available,
        "VP3 release response does not contain an update"
    );
    ensure!(
        response.signature_algorithm.as_deref() == Some("Ed25519"),
        "VP3 release signature algorithm is invalid"
    );
    let key_id = response
        .signing_key_id
        .as_deref()
        .context("VP3 release signing key ID is missing")?;
    ensure!(
        key_id == state.config.vp3_release_key_id,
        "VP3 release signing key is not trusted"
    );
    let document_encoded = response
        .manifest
        .as_deref()
        .context("VP3 signed release document is missing")?;
    let signature = response
        .signature
        .as_deref()
        .context("VP3 release signature is missing")?;
    let document = URL_SAFE_NO_PAD
        .decode(document_encoded)
        .context("VP3 signed release document encoding is invalid")?;
    verify_ed25519(
        &state.config.vp3_release_public_key_base64,
        &document,
        signature,
        "VP3 release manifest",
    )?;
    let hash = hex::encode(Sha256::digest(&document));
    ensure!(
        response.manifest_hash.as_deref() == Some(hash.as_str()),
        "VP3 release manifest hash does not match the signed document"
    );
    let manifest: Value =
        serde_json::from_slice(&document).context("VP3 release manifest JSON is invalid")?;
    ensure!(
        manifest.get("schema").and_then(Value::as_str) == Some("vp3.release-manifest.v1"),
        "VP3 release manifest schema is unsupported"
    );
    ensure!(
        manifest.get("target_type").and_then(Value::as_str) == Some("homeserver"),
        "VP3 release target type is invalid"
    );
    let release_public_id = manifest
        .get("release_public_id")
        .and_then(Value::as_str)
        .context("VP3 release identity is missing")?;
    ensure!(
        response.release_public_id.as_deref() == Some(release_public_id),
        "VP3 release wrapper identity does not match its signed document"
    );
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .context("VP3 release version is missing")?;
    Version::parse(version).context("VP3 release version is invalid")?;
    ensure!(
        response.version.as_deref() == Some(version),
        "VP3 release wrapper version does not match its signed document"
    );
    let channel = manifest
        .get("channel")
        .and_then(Value::as_str)
        .context("VP3 release channel is missing")?;
    ensure!(
        channel == "stable",
        "Phase 13 accepts stable VP3 releases only"
    );
    ensure!(
        response.channel.as_deref() == Some(channel),
        "VP3 release wrapper channel does not match its signed document"
    );
    ensure!(
        response.update_channel == "stable",
        "VP3 licensed update channel does not permit this release"
    );
    ensure!(
        response.emergency_override == manifest.get("emergency_override").and_then(Value::as_bool),
        "VP3 release emergency policy evidence does not match"
    );
    let release_notes_hash = manifest
        .get("release_notes_hash")
        .and_then(Value::as_str)
        .context("VP3 release notes hash is missing")?;
    ensure!(
        valid_sha256(release_notes_hash),
        "VP3 release notes hash is invalid"
    );
    let artifacts = manifest
        .get("artifacts")
        .and_then(Value::as_array)
        .context("VP3 release artifact list is missing")?;
    let artifact = artifacts
        .iter()
        .find(|artifact| {
            artifact.get("platform").and_then(Value::as_str) == Some("windows")
                && artifact.get("architecture").and_then(Value::as_str) == Some("x86_64")
        })
        .context("VP3 release has no Windows x86_64 artifact")?;
    let file_name = artifact
        .get("file_name")
        .and_then(Value::as_str)
        .context("VP3 release installer filename is missing")?;
    ensure!(
        file_name == "Microgifter-HomeServer-Setup.exe",
        "VP3 release installer filename is invalid"
    );
    let sha256 = artifact
        .get("sha256")
        .and_then(Value::as_str)
        .context("VP3 release installer SHA-256 is missing")?
        .to_lowercase();
    ensure!(
        valid_sha256(&sha256),
        "VP3 release installer SHA-256 is invalid"
    );
    let size_bytes =
        json_u64(artifact.get("size_bytes")).context("VP3 release installer size is missing")?;
    ensure!(
        (MIN_INSTALLER_BYTES..=MAX_INSTALLER_BYTES).contains(&size_bytes),
        "VP3 release installer size is outside the supported range"
    );
    let thumbprint = artifact
        .get("authenticode_thumbprint")
        .and_then(Value::as_str)
        .context("VP3 release Authenticode thumbprint is missing")?
        .to_uppercase();
    ensure!(
        valid_thumbprint(&thumbprint),
        "VP3 release Authenticode thumbprint is invalid"
    );
    let compatibility = manifest
        .get("compatibility")
        .cloned()
        .unwrap_or(Value::Null);
    let minimum_version = compatibility
        .get("minimum_current_version")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let Some(minimum) = minimum_version.as_deref() {
        ensure!(
            Version::parse(env!("CARGO_PKG_VERSION"))? >= Version::parse(minimum)?,
            "VP3 release requires a newer HomeServer baseline"
        );
    }
    if let Some(maximum) = compatibility
        .get("maximum_current_version")
        .and_then(Value::as_str)
    {
        ensure!(
            Version::parse(env!("CARGO_PKG_VERSION"))? <= Version::parse(maximum)?,
            "VP3 release does not support this HomeServer baseline"
        );
    }
    Ok(VerifiedRelease {
        update_id: format!("vp3:{}:{}", release_public_id, &sha256[..16]),
        release_public_id: release_public_id.to_owned(),
        version: version.to_owned(),
        channel: channel.to_owned(),
        manifest_document: document_encoded.to_owned(),
        manifest_signature: signature.to_owned(),
        signing_key_id: key_id.to_owned(),
        manifest_hash: hash,
        file_name: file_name.to_owned(),
        sha256,
        size_bytes,
        authenticode_thumbprint: thumbprint,
        minimum_version,
        release_notes_hash: release_notes_hash.to_owned(),
    })
}

fn persist_release_binding(state: &AppState, release: &VerifiedRelease) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    let connection = state.connection()?;
    connection.execute(
        "INSERT INTO vp3_update_bindings (update_id,release_public_id,version,channel,manifest_document,manifest_signature,signing_key_id,manifest_hash,installer_file_name,installer_sha256,installer_size_bytes,authenticode_thumbprint,checked_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?13,?13) ON CONFLICT(update_id) DO UPDATE SET release_public_id=excluded.release_public_id,version=excluded.version,channel=excluded.channel,manifest_document=excluded.manifest_document,manifest_signature=excluded.manifest_signature,signing_key_id=excluded.signing_key_id,manifest_hash=excluded.manifest_hash,installer_file_name=excluded.installer_file_name,installer_sha256=excluded.installer_sha256,installer_size_bytes=excluded.installer_size_bytes,authenticode_thumbprint=excluded.authenticode_thumbprint,checked_at_utc=excluded.checked_at_utc,updated_at_utc=excluded.updated_at_utc",
        params![
            release.update_id,
            release.release_public_id,
            release.version,
            release.channel,
            release.manifest_document,
            release.manifest_signature,
            release.signing_key_id,
            release.manifest_hash,
            release.file_name,
            release.sha256,
            release.size_bytes as i64,
            release.authenticode_thumbprint,
            now,
        ],
    )?;
    connection.execute(
        "UPDATE vp3_authority_client_state SET last_manifest_checked_at_utc=?1,last_error_code=NULL,updated_at_utc=?1 WHERE singleton_id=1",
        params![now],
    )?;
    Ok(())
}

async fn download_update(state: &AppState) -> Result<UpdateActionResult> {
    refresh_lease_if_needed(state).await?;
    let stored = {
        let connection = state.connection()?;
        let available = update_store::latest_in_state(&connection, UpdateState::Available)?;
        software_authority::ensure_update_download_allowed(
            &connection,
            &available.record.update_id,
        )?;
        update_store::mark_downloading(&connection, &available.record.update_id)?
    };
    let binding = binding(state, &stored.record.update_id)?;
    let fresh = fetch_manifest(state).await?;
    let release = verify_release(state, &fresh)?;
    ensure!(
        release.update_id == stored.record.update_id,
        "VP3 release changed before download authorization"
    );
    ensure!(
        release.release_public_id == binding.0,
        "VP3 release identity changed before download authorization"
    );
    ensure!(
        release.sha256 == stored.record.installer_sha256,
        "VP3 installer hash changed before download authorization"
    );
    ensure!(
        release.size_bytes == stored.record.installer_size_bytes,
        "VP3 installer size changed before download authorization"
    );
    ensure!(
        release.authenticode_thumbprint == stored.record.authenticode_thumbprint,
        "VP3 Authenticode signer changed before download authorization"
    );
    let authorization = fresh
        .installer_authorization
        .context("VP3 installer authorization is missing")?;
    ensure!(
        parse_database_or_rfc3339(&authorization.expires_at)? > Utc::now(),
        "VP3 installer authorization is expired"
    );
    ensure!(
        (32..=256).contains(&authorization.token.len()),
        "VP3 installer authorization token is invalid"
    );
    let url = authority_url(state, &authorization.download_path)?;
    ensure!(
        url.query_pairs()
            .any(|(key, value)| key == "grant" && value == authorization.token),
        "VP3 installer authorization is not bound to its grant token"
    );
    let destination = installer_destination(state, &stored.record.update_id, &release.file_name);
    let temporary = destination.with_extension("part");
    if temporary.exists() {
        tokio::fs::remove_file(&temporary).await?;
    }
    let response = client(Duration::from_secs(15 * 60))?
        .get(url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("unable to download the VP3-authorized HomeServer installer")?
        .error_for_status()
        .context("VP3-authorized HomeServer installer download was rejected")?;
    if let Some(length) = response.content_length() {
        ensure!(
            length == release.size_bytes,
            "VP3 installer response size does not match the signed release"
        );
    }
    let mut output = tokio::fs::File::create(&temporary).await?;
    let mut stream = response.bytes_stream();
    let mut size = 0_u64;
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("unable to read the VP3 installer response")?;
        size = size
            .checked_add(chunk.len() as u64)
            .context("VP3 installer size overflow")?;
        ensure!(
            size <= release.size_bytes && size <= MAX_INSTALLER_BYTES,
            "VP3 installer exceeds the signed size"
        );
        output.write_all(&chunk).await?;
        hasher.update(&chunk);
    }
    output.sync_all().await?;
    drop(output);
    ensure!(size == release.size_bytes, "VP3 installer is truncated");
    ensure!(
        hex::encode(hasher.finalize()).eq_ignore_ascii_case(&release.sha256),
        "VP3 installer SHA-256 does not match the signed release"
    );
    update::verify_authenticode(&temporary, &release.authenticode_thumbprint)?;
    if destination.exists() {
        tokio::fs::remove_file(&destination).await?;
    }
    tokio::fs::rename(&temporary, &destination).await?;
    let staged =
        update_store::mark_staged(&state.connection()?, &stored.record.update_id, &destination)?;
    queue_receipt(
        state,
        &stored.record.update_id,
        &stored.record.version,
        "downloaded",
        None,
    )?;
    submit_pending_receipts(state).await?;
    Ok(UpdateActionResult {
        status: update_store::status(
            &state.connection()?,
            &format!("{}/api/homeserver/v1/manifest.php", state.config.vp3_base_url),
            state.config.update_plan_path().exists(),
        )?,
        message: format!("HomeServer {} was authorized by VP3, downloaded, hash-verified, and Authenticode-verified.", staged.record.version),
        restart_required: false,
    })
}

fn binding(state: &AppState, update_id: &str) -> Result<(String, String)> {
    state
        .connection()?
        .query_row(
            "SELECT release_public_id,manifest_hash FROM vp3_update_bindings WHERE update_id=?1",
            params![update_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .context("VP3 update binding was not found")
}

fn queue_receipt(
    state: &AppState,
    update_id: &str,
    version: &str,
    disposition: &str,
    failure_code: Option<&str>,
) -> Result<()> {
    state.connection()?.execute(
        "INSERT INTO software_authority_receipts (receipt_id,authority_key,event_type,update_id,version,disposition,failure_code,submission_state,created_at_utc) VALUES (?1,'vp3','update.result',?2,?3,?4,?5,'pending_vp3_submission',?6)",
        params![
            Uuid::new_v4().to_string(),
            update_id,
            version,
            disposition,
            failure_code,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

async fn submit_pending_receipts(state: &AppState) -> Result<()> {
    let identity = identity(state)?;
    let credential = load_credential()?;
    let receipts = {
        let connection = state.connection()?;
        let mut statement = connection.prepare(
            "SELECT receipt_id,update_id,version,disposition,failure_code FROM software_authority_receipts WHERE authority_key='vp3' AND submission_state='pending_vp3_submission' ORDER BY created_at_utc,receipt_id LIMIT 50",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (receipt_id, update_id, version, local_disposition, failure_code) in receipts {
        let release_public_id = binding(state, &update_id).ok().map(|value| value.0);
        let disposition = match local_disposition.as_str() {
            "succeeded" | "installed" => "installed",
            "rolled_back" => "rolled_back",
            "failed" => "failed",
            "downloaded" => "downloaded",
            "staged" => "staged",
            other => {
                warn!(%other, %receipt_id, "unsupported VP3 receipt disposition retained for audit");
                continue;
            }
        };
        let receipt_hash = hex::encode(Sha256::digest(
            format!(
                "{receipt_id}|{update_id}|{version}|{disposition}|{}",
                failure_code.as_deref().unwrap_or("")
            )
            .as_bytes(),
        ));
        let _: Value = post_device(
            state,
            "update-receipt.php",
            &credential,
            json!({
                "device_public_id": identity.device_public_id,
                "request_id": safe_request_id(&receipt_id),
                "update_id": update_id,
                "release_public_id": release_public_id,
                "disposition": disposition,
                "failure_code": failure_code,
                "receipt_hash": receipt_hash,
                "metadata": {"version": version, "local_receipt_id": receipt_id}
            }),
        )
        .await?;
        state.connection()?.execute(
            "UPDATE software_authority_receipts SET submission_state='submitted',submitted_at_utc=?1 WHERE receipt_id=?2 AND submission_state='pending_vp3_submission'",
            params![Utc::now().to_rfc3339(), receipt_id],
        )?;
    }
    Ok(())
}

fn disconnect(state: &AppState) -> Result<()> {
    delete_credential()?;
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute("DELETE FROM vp3_update_bindings", [])?;
    transaction.execute(
        "UPDATE vp3_authority_client_state SET account_id=NULL,device_public_id=NULL,license_public_id=NULL,device_fingerprint=NULL,activation_state='unconfigured',lease_public_id=NULL,lease_key_id=NULL,lease_document_hash=NULL,lease_signature_hash=NULL,lease_expires_at_utc=NULL,last_heartbeat_at_utc=NULL,last_manifest_checked_at_utc=NULL,last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        [],
    )?;
    transaction.execute(
        "UPDATE homeserver_software_authority SET current_authority='microgifter_legacy',target_authority='vp3',cutover_state='awaiting_vp3_activation',vp3_device_id=NULL,vp3_license_id=NULL,vp3_lease_id=NULL,vp3_lease_expires_at_utc=NULL,update_eligible=0,allowed_update_channels_json='[]',last_vp3_heartbeat_at_utc=NULL,last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

fn identity(state: &AppState) -> Result<ClientIdentity> {
    state
        .connection()?
        .query_row(
            "SELECT account_id,device_public_id,license_public_id,device_fingerprint,lease_expires_at_utc FROM vp3_authority_client_state WHERE singleton_id=1 AND activation_state IN ('activating','active')",
            [],
            |row| {
                Ok(ClientIdentity {
                    account_id: row.get(0)?,
                    device_public_id: row.get(1)?,
                    license_public_id: row.get(2)?,
                    device_fingerprint: row.get(3)?,
                    lease_expires_at_utc: row.get(4)?,
                })
            },
        )
        .context("VP3 device activation is not configured")
}

fn mark_error(state: &AppState, code: &str) -> Result<()> {
    let code = bounded(code, 120);
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE vp3_authority_client_state SET activation_state='error',last_error_code=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![code],
    )?;
    transaction.execute(
        "UPDATE homeserver_software_authority SET last_error_code=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![code],
    )?;
    transaction.commit()?;
    Ok(())
}

async fn post_device<T: DeserializeOwned>(
    state: &AppState,
    endpoint: &str,
    credential: &str,
    body: Value,
) -> Result<T> {
    let url = authority_url(state, &format!("/api/homeserver/v1/{endpoint}"))?;
    let request_id = body
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("vp3-request");
    let response = client(Duration::from_secs(30))?
        .post(url)
        .bearer_auth(credential)
        .header("X-Request-ID", request_id)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&body)
        .send()
        .await
        .context("VP3 software-authority request failed")?;
    decode_envelope(response).await
}

async fn decode_envelope<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_JSON_BYTES as u64,
            "VP3 response exceeds the size limit"
        );
    }
    let bytes = response
        .bytes()
        .await
        .context("unable to read VP3 response")?;
    ensure!(
        bytes.len() <= MAX_JSON_BYTES,
        "VP3 response exceeds the size limit"
    );
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("VP3 request was rejected with HTTP {status}"));
        bail!(bounded(&message, 500));
    }
    let envelope: Envelope<T> =
        serde_json::from_slice(&bytes).context("VP3 response JSON is invalid")?;
    Ok(envelope.data)
}

fn client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!(
            "Microgifter-HomeServer/{} VP3-Authority/1",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("unable to create the VP3 software-authority client")
}

fn authority_url(state: &AppState, path: &str) -> Result<Url> {
    let base = Url::parse(&format!(
        "{}/",
        state.config.vp3_base_url.trim_end_matches('/')
    ))
    .context("VP3 base URL is invalid")?;
    let url = base
        .join(path.trim_start_matches('/'))
        .context("VP3 endpoint URL is invalid")?;
    ensure!(url.scheme() == "https", "VP3 endpoint must use HTTPS");
    ensure!(
        url.host_str() == base.host_str(),
        "VP3 endpoint escaped the configured authority host"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "VP3 endpoint cannot contain credentials"
    );
    Ok(url)
}

fn configured_verifying_key(value: &str, label: &str) -> Result<VerifyingKey> {
    let bytes = STANDARD
        .decode(value)
        .with_context(|| format!("{label} is not valid base64"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("{label} must decode to 32 bytes"))?;
    VerifyingKey::from_bytes(&bytes).with_context(|| format!("{label} is invalid"))
}

fn verify_ed25519(public_key: &str, document: &[u8], signature: &str, label: &str) -> Result<()> {
    let key = configured_verifying_key(public_key, &format!("{label} public key"))?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .with_context(|| format!("{label} signature encoding is invalid"))?;
    let signature = Signature::from_slice(&signature)
        .with_context(|| format!("{label} signature length is invalid"))?;
    key.verify(document, &signature)
        .with_context(|| format!("{label} signature verification failed"))
}

fn save_credential(credential: &str) -> Result<()> {
    Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_KEY)?
        .set_password(credential)
        .context("unable to save the VP3 device credential in the operating-system vault")
}

fn load_credential() -> Result<String> {
    Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_KEY)?
        .get_password()
        .context("VP3 device credential is unavailable from the operating-system vault")
}

fn delete_credential() -> Result<()> {
    match Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_KEY)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to remove the VP3 device credential"),
    }
}

fn installer_destination(state: &AppState, update_id: &str, file_name: &str) -> PathBuf {
    state.config.update_staging_dir.join(format!(
        "{}-{}",
        update_id.replace([':', '/', '\\'], "-"),
        file_name
    ))
}

fn request_id(operation: &str) -> String {
    format!("VP3-{}-{}", bounded(operation, 24), Uuid::new_v4().simple())
}

fn safe_request_id(receipt_id: &str) -> String {
    let cleaned: String = receipt_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || ". _:-".contains(*character))
        .filter(|character| *character != ' ')
        .take(52)
        .collect();
    format!("VP3-RCP-{cleaned}")
}

fn validate_public_id(value: &str, label: &str) -> Result<()> {
    ensure!(
        (3..=80).contains(&value.len())
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character)),
        "{label} is invalid"
    );
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn valid_thumbprint(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn json_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|text| text.parse::<u64>().ok()))
    })
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .context("VP3 timestamp is invalid")?
        .with_timezone(&Utc))
}

fn parse_database_or_rfc3339(value: &str) -> Result<DateTime<Utc>> {
    parse_utc(value).or_else(|_| {
        DateTime::parse_from_str(&format!("{value} +0000"), "%Y-%m-%d %H:%M:%S %z")
            .map(|value| value.with_timezone(&Utc))
            .context("VP3 database timestamp is invalid")
    })
}

fn bounded(value: &str, maximum: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(maximum)
        .collect()
}

fn action_error(code: &'static str, error: impl std::fmt::Display) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(ApiError {
            ok: false,
            error: code,
            message: bounded(&error.to_string(), 500),
        }),
    )
}

fn internal_error(
    code: &'static str,
    error: impl std::fmt::Display,
) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: bounded(&error.to_string(), 500),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_material_validators_are_strict() {
        assert!(valid_sha256(&"a".repeat(64)));
        assert!(!valid_sha256(&"a".repeat(63)));
        assert!(valid_thumbprint(&"B".repeat(40)));
        assert!(valid_thumbprint(&"C".repeat(64)));
        assert!(!valid_thumbprint("thumbprint"));
    }

    #[test]
    fn request_identifiers_are_bounded_and_safe() {
        let request_id = request_id("manifest");
        assert!((8..=64).contains(&request_id.len()));
        assert!(request_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character)));
    }
}
