use crate::AppState;
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, SecondsFormat, Utc};
use keyring::Entry;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use url::Url;
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0020_wrapper_identity_and_pairing.sql");
const MIGRATION_KEY: &str = "0020_wrapper_identity_and_pairing";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";
const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;
const MAX_WRAPPERS: i64 = 100;
const MAX_CONNECTIONS: i64 = 500;
const MAX_DEVICES: i64 = 500;
const MAX_PAIRING_ATTEMPTS: i64 = 5_000;
const MAX_EVENTS: i64 = 10_000;
const MAX_CAPABILITIES: usize = 128;

const WRAPPER_KINDS: &[&str] = &[
    "pod",
    "application",
    "commerce",
    "media",
    "service",
    "other",
];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrapperIdentitySummary {
    pub wrapper_id: String,
    pub wrapper_key: String,
    pub display_name: String,
    pub wrapper_kind: String,
    pub protocol_version: String,
    pub state: String,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub revoked_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrapperConnectionSummary {
    pub connection_id: String,
    pub wrapper_id: String,
    pub remote_connection_id: Option<String>,
    pub remote_origin: String,
    pub contract_version: String,
    pub lifecycle_state: String,
    pub credential_reference: String,
    pub grant_revision: u64,
    pub legacy_provider_key: Option<String>,
    pub legacy_connection_id: Option<String>,
    pub paired_at_utc: Option<String>,
    pub last_seen_at_utc: Option<String>,
    pub revoked_at_utc: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrapperDeviceSummary {
    pub wrapper_device_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub device_public_id: String,
    pub installation_id: String,
    pub public_key_base64: String,
    pub credential_reference: String,
    pub state: String,
    pub last_seen_at_utc: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub revoked_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WrapperPairingAttemptSummary {
    pub attempt_id: String,
    pub wrapper_id: String,
    pub request_id: String,
    pub remote_origin: String,
    pub device_display_name: String,
    pub requested_capabilities: Vec<String>,
    pub state: String,
    pub expires_at_utc: String,
    pub result_connection_id: Option<String>,
    pub error_code: Option<String>,
    pub created_at_utc: String,
    pub completed_at_utc: Option<String>,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WrapperRegistrySnapshot {
    pub schema: String,
    pub wrappers: Vec<WrapperIdentitySummary>,
    pub connections: Vec<WrapperConnectionSummary>,
    pub devices: Vec<WrapperDeviceSummary>,
    pub pairing_attempts: Vec<WrapperPairingAttemptSummary>,
    pub active_wrappers: u64,
    pub active_connections: u64,
    pub pending_pairings: u64,
    pub local_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterWrapperRequest {
    pub wrapper_key: String,
    pub display_name: String,
    pub wrapper_kind: String,
    pub protocol_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartWrapperPairingRequest {
    pub wrapper_id: String,
    pub request_id: String,
    pub remote_origin: String,
    pub device_display_name: String,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    pub expires_minutes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompleteWrapperPairingRequest {
    pub attempt_id: String,
    pub connection_id: String,
    pub remote_connection_id: Option<String>,
    pub contract_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevokeWrapperConnectionRequest {
    pub connection_id: String,
    pub confirmation: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
struct LegacyConnection {
    connection_id: String,
    provider_key: String,
    display_name: String,
    remote_origin: String,
    device_id: String,
    public_key_base64: String,
    credential_reference: String,
    state: String,
    paired_at_utc: String,
    last_seen_at_utc: Option<String>,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    backfill_legacy_connections(connection)?;
    expire_pairing_attempts(connection)?;
    maintain_history(connection)?;
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
        "wrapper identity and pairing migration is not registered exactly once"
    );

    for table in [
        "wrapper_identities",
        "wrapper_connections",
        "wrapper_devices",
        "wrapper_pairing_attempts",
        "wrapper_credential_references",
        "wrapper_events",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }

    let orphan_connections: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_connections c LEFT JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id WHERE w.wrapper_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        orphan_connections == 0,
        "wrapper connection identity binding is invalid"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    expire_pairing_attempts(connection)?;
    connection.execute(
        "DELETE FROM wrapper_pairing_attempts WHERE state IN ('completed','failed','expired','cancelled') AND attempt_id NOT IN (SELECT attempt_id FROM wrapper_pairing_attempts WHERE state IN ('completed','failed','expired','cancelled') ORDER BY updated_at_utc DESC,attempt_id DESC LIMIT ?1)",
        params![MAX_PAIRING_ATTEMPTS],
    )?;
    connection.execute(
        "DELETE FROM wrapper_events WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM wrapper_events WHERE event_id NOT IN (SELECT event_id FROM wrapper_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1)",
        params![MAX_EVENTS],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/wrappers", get(snapshot_handler))
        .route("/v1/wrappers/register", post(register_handler))
        .route("/v1/wrappers/pairing/start", post(start_pairing_handler))
        .route(
            "/v1/wrappers/pairing/complete",
            post(complete_pairing_handler),
        )
        .route(
            "/v1/wrappers/connections/revoke",
            post(revoke_connection_handler),
        )
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn snapshot_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<WrapperRegistrySnapshot> {
    tokio::task::spawn_blocking(move || snapshot(&state))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| api_error("wrapper_registry_snapshot_failed", error))
}

async fn register_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RegisterWrapperRequest>,
) -> ApiResult<WrapperRegistrySnapshot> {
    tokio::task::spawn_blocking(move || register_wrapper(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| api_error("wrapper_registration_rejected", error))
}

async fn start_pairing_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartWrapperPairingRequest>,
) -> ApiResult<WrapperRegistrySnapshot> {
    tokio::task::spawn_blocking(move || start_pairing(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| api_error("wrapper_pairing_start_rejected", error))
}

async fn complete_pairing_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CompleteWrapperPairingRequest>,
) -> ApiResult<WrapperRegistrySnapshot> {
    tokio::task::spawn_blocking(move || complete_pairing(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| api_error("wrapper_pairing_completion_rejected", error))
}

async fn revoke_connection_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RevokeWrapperConnectionRequest>,
) -> ApiResult<WrapperRegistrySnapshot> {
    tokio::task::spawn_blocking(move || revoke_connection(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| api_error("wrapper_connection_revocation_rejected", error))
}

fn snapshot(state: &AppState) -> Result<WrapperRegistrySnapshot> {
    let connection = state.connection()?;
    expire_pairing_attempts(&connection)?;
    snapshot_with_connection(&connection)
}

fn snapshot_with_connection(connection: &Connection) -> Result<WrapperRegistrySnapshot> {
    let wrappers = read_wrappers(connection)?;
    let connections = read_connections(connection)?;
    let devices = read_devices(connection)?;
    let pairing_attempts = read_pairing_attempts(connection)?;
    let active_wrappers = wrappers
        .iter()
        .filter(|item| item.state == "active")
        .count() as u64;
    let active_connections = connections
        .iter()
        .filter(|item| item.lifecycle_state == "active")
        .count() as u64;
    let pending_pairings = pairing_attempts
        .iter()
        .filter(|item| item.state == "pending")
        .count() as u64;
    Ok(WrapperRegistrySnapshot {
        schema: "homeserver.wrapper-registry.v1".to_owned(),
        wrappers,
        connections,
        devices,
        pairing_attempts,
        active_wrappers,
        active_connections,
        pending_pairings,
        local_only: true,
    })
}

fn register_wrapper(
    state: &AppState,
    request: RegisterWrapperRequest,
) -> Result<WrapperRegistrySnapshot> {
    let wrapper_key = validate_wrapper_key(&request.wrapper_key)?;
    let display_name = bounded_text(&request.display_name, 1, 120, "wrapper display name")?;
    let wrapper_kind = request.wrapper_kind.trim().to_ascii_lowercase();
    ensure!(
        WRAPPER_KINDS.contains(&wrapper_kind.as_str()),
        "wrapper kind is not supported"
    );
    let protocol_version = bounded_text(&request.protocol_version, 1, 40, "protocol version")?;
    let connection = state.connection()?;

    if let Some(existing) = wrapper_by_key(&connection, &wrapper_key)? {
        ensure!(
            existing.wrapper_kind == wrapper_kind
                && existing.protocol_version == protocol_version
                && existing.display_name == display_name,
            "wrapper key is already registered with different metadata"
        );
        return snapshot_with_connection(&connection);
    }

    let wrapper_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO wrapper_identities (wrapper_id,wrapper_key,display_name,wrapper_kind,protocol_version,state,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,'active',?6,?6)",
        params![wrapper_id, wrapper_key, display_name, wrapper_kind, protocol_version, now],
    )?;
    record_event(
        &transaction,
        &wrapper_id,
        None,
        "wrapper.registered",
        "success",
        None,
        json!({"wrapper_key": wrapper_key}),
    )?;
    transaction.commit()?;
    snapshot_with_connection(&connection)
}

fn start_pairing(
    state: &AppState,
    request: StartWrapperPairingRequest,
) -> Result<WrapperRegistrySnapshot> {
    let wrapper_id = validate_uuid(&request.wrapper_id, "wrapper ID")?;
    let request_id = validate_request_id(&request.request_id)?;
    let remote_origin = validate_remote_origin(&request.remote_origin)?;
    let device_display_name =
        bounded_text(&request.device_display_name, 1, 120, "device display name")?;
    let requested_capabilities = normalize_capabilities(request.requested_capabilities)?;
    let expires_minutes = request.expires_minutes.unwrap_or(15).clamp(1, 60);
    let now = Utc::now();
    let now_text = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let expires_at_utc = (now + Duration::minutes(i64::from(expires_minutes)))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let capabilities_json = serde_json::to_string(&requested_capabilities)?;
    let request_hash = hex::encode(Sha256::digest(
        format!(
            "{}\n{}\n{}\n{}\n{}",
            wrapper_id, request_id, remote_origin, device_display_name, capabilities_json
        )
        .as_bytes(),
    ));
    let connection = state.connection()?;
    let wrapper = wrapper_by_id(&connection, &wrapper_id)?;
    ensure!(wrapper.state == "active", "wrapper identity is not active");

    if let Some((stored_hash, state_value)) = connection
        .query_row(
            "SELECT request_hash,state FROM wrapper_pairing_attempts WHERE wrapper_id=?1 AND request_id=?2",
            params![wrapper_id, request_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    {
        ensure!(stored_hash == request_hash, "pairing request ID was reused with different data");
        ensure!(
            matches!(state_value.as_str(), "pending" | "completed"),
            "pairing request cannot be replayed from its current state"
        );
        return snapshot_with_connection(&connection);
    }

    let attempt_id = Uuid::new_v4().to_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO wrapper_pairing_attempts (attempt_id,wrapper_id,request_id,request_hash,remote_origin,device_display_name,requested_capabilities_json,state,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9,?9)",
        params![
            attempt_id,
            wrapper_id,
            request_id,
            request_hash,
            remote_origin,
            device_display_name,
            capabilities_json,
            expires_at_utc,
            now_text
        ],
    )?;
    record_event(
        &transaction,
        &wrapper_id,
        None,
        "wrapper.pairing.started",
        "success",
        Some(&request_id),
        json!({"attempt_id": attempt_id, "expires_at_utc": expires_at_utc}),
    )?;
    transaction.commit()?;
    snapshot_with_connection(&connection)
}

fn complete_pairing(
    state: &AppState,
    request: CompleteWrapperPairingRequest,
) -> Result<WrapperRegistrySnapshot> {
    let attempt_id = validate_uuid(&request.attempt_id, "pairing attempt ID")?;
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let contract_version = bounded_text(&request.contract_version, 1, 80, "contract version")?;
    let remote_connection_id = request
        .remote_connection_id
        .as_deref()
        .map(|value| bounded_text(value, 1, 160, "remote connection ID"))
        .transpose()?;
    let connection = state.connection()?;
    expire_pairing_attempts(&connection)?;

    let (wrapper_id, attempt_state, attempt_origin, expires_at_utc): (String, String, String, String) =
        connection
            .query_row(
                "SELECT wrapper_id,state,remote_origin,expires_at_utc FROM wrapper_pairing_attempts WHERE attempt_id=?1",
                params![attempt_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .context("pairing attempt was not found")?;
    ensure!(attempt_state == "pending", "pairing attempt is not pending");
    ensure!(expires_at_utc > now_utc(), "pairing attempt has expired");

    let legacy = legacy_connection_by_id(&connection, &connection_id)?;
    let wrapper = wrapper_by_id(&connection, &wrapper_id)?;
    ensure!(
        wrapper.wrapper_key == legacy.provider_key
            || (wrapper.wrapper_kind == "pod" && legacy.provider_key == "pod"),
        "paired connection provider does not match the wrapper identity"
    );
    ensure!(
        normalize_origin_for_compare(&attempt_origin)?
            == normalize_origin_for_compare(&legacy.remote_origin)?,
        "paired connection origin does not match the approved pairing attempt"
    );

    let transaction = connection.unchecked_transaction()?;
    upsert_connection_overlay(
        &transaction,
        &wrapper_id,
        &legacy,
        remote_connection_id.as_deref(),
        &contract_version,
    )?;
    transaction.execute(
        "UPDATE wrapper_pairing_attempts SET state='completed',result_connection_id=?1,completed_at_utc=?2,updated_at_utc=?2,error_code=NULL WHERE attempt_id=?3 AND state='pending'",
        params![connection_id, now_utc(), attempt_id],
    )?;
    record_event(
        &transaction,
        &wrapper_id,
        Some(&connection_id),
        "wrapper.pairing.completed",
        "success",
        None,
        json!({"attempt_id": attempt_id, "contract_version": contract_version}),
    )?;
    transaction.commit()?;
    snapshot_with_connection(&connection)
}

fn revoke_connection(
    state: &AppState,
    request: RevokeWrapperConnectionRequest,
) -> Result<WrapperRegistrySnapshot> {
    ensure!(
        request.confirmation.trim() == "REVOKE WRAPPER",
        "type REVOKE WRAPPER to revoke this connection"
    );
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let reason = request
        .reason
        .as_deref()
        .map(|value| bounded_text(value, 1, 500, "revocation reason"))
        .transpose()?;
    let connection = state.connection()?;
    let (wrapper_id, credential_reference): (String, String) = connection
        .query_row(
            "SELECT wrapper_id,credential_reference FROM wrapper_connections WHERE connection_id=?1",
            params![connection_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .context("wrapper connection was not found")?;

    delete_vault_credential(&credential_reference)?;
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE cloud_connections SET state='revoked',updated_at_utc=?1,last_error='revoked_by_owner' WHERE connection_id=?2",
        params![now, connection_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_connections SET lifecycle_state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE connection_id=?2",
        params![now, connection_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_devices SET state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE connection_id=?2",
        params![now, connection_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_credential_references SET state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE connection_id=?2",
        params![now, connection_id],
    )?;
    record_event(
        &transaction,
        &wrapper_id,
        Some(&connection_id),
        "wrapper.connection.revoked",
        "success",
        None,
        json!({"reason": reason}),
    )?;
    transaction.commit()?;
    snapshot_with_connection(&connection)
}

fn delete_vault_credential(credential_reference: &str) -> Result<()> {
    let entry = Entry::new(CREDENTIAL_SERVICE, credential_reference)
        .context("unable to access the HomeServer credential vault")?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to remove the wrapper credential from the vault"),
    }
}

fn backfill_legacy_connections(connection: &Connection) -> Result<()> {
    let legacy_connections = read_legacy_connections(connection)?;
    if legacy_connections.is_empty() {
        return Ok(());
    }
    let installation_id: String = connection.query_row(
        "SELECT setting_value FROM homeserver_settings WHERE setting_key='installation_id'",
        [],
        |row| row.get(0),
    )?;
    let transaction = connection.unchecked_transaction()?;
    let mut wrappers: HashMap<String, String> = HashMap::new();
    for legacy in &legacy_connections {
        let wrapper_id = if let Some(wrapper_id) = wrappers.get(&legacy.provider_key) {
            wrapper_id.clone()
        } else {
            let wrapper_id = ensure_legacy_wrapper(&transaction, legacy)?;
            wrappers.insert(legacy.provider_key.clone(), wrapper_id.clone());
            wrapper_id
        };
        upsert_connection_overlay(&transaction, &wrapper_id, legacy, None, "legacy-v1")?;
        ensure_device_overlay(&transaction, &wrapper_id, legacy, &installation_id)?;
    }
    transaction.commit()?;
    Ok(())
}

fn ensure_legacy_wrapper(connection: &Connection, legacy: &LegacyConnection) -> Result<String> {
    if let Some(wrapper_id) = connection
        .query_row(
            "SELECT wrapper_id FROM wrapper_identities WHERE wrapper_key=?1",
            params![legacy.provider_key],
            |row| row.get(0),
        )
        .optional()?
    {
        return Ok(wrapper_id);
    }
    let wrapper_id = Uuid::new_v4().to_string();
    let kind = match legacy.provider_key.as_str() {
        "microgifter" => "commerce",
        "pod" | "rss-pod" | "rss_pod" => "pod",
        _ => "application",
    };
    connection.execute(
        "INSERT INTO wrapper_identities (wrapper_id,wrapper_key,display_name,wrapper_kind,protocol_version,state) VALUES (?1,?2,?3,?4,'legacy-v1','active')",
        params![wrapper_id, legacy.provider_key, legacy.display_name, kind],
    )?;
    Ok(wrapper_id)
}

fn upsert_connection_overlay(
    connection: &Connection,
    wrapper_id: &str,
    legacy: &LegacyConnection,
    remote_connection_id: Option<&str>,
    contract_version: &str,
) -> Result<()> {
    let lifecycle_state = map_connection_state(&legacy.state);
    connection.execute(
        "INSERT INTO wrapper_connections (connection_id,wrapper_id,remote_connection_id,remote_origin,contract_version,lifecycle_state,credential_reference,legacy_provider_key,legacy_connection_id,paired_at_utc,last_seen_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?1,?9,?10) ON CONFLICT(connection_id) DO UPDATE SET wrapper_id=excluded.wrapper_id,remote_connection_id=COALESCE(excluded.remote_connection_id,wrapper_connections.remote_connection_id),remote_origin=excluded.remote_origin,contract_version=CASE WHEN excluded.contract_version='legacy-v1' THEN wrapper_connections.contract_version ELSE excluded.contract_version END,lifecycle_state=excluded.lifecycle_state,credential_reference=excluded.credential_reference,legacy_provider_key=excluded.legacy_provider_key,legacy_connection_id=excluded.legacy_connection_id,paired_at_utc=excluded.paired_at_utc,last_seen_at_utc=excluded.last_seen_at_utc,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![
            legacy.connection_id,
            wrapper_id,
            remote_connection_id,
            legacy.remote_origin,
            contract_version,
            lifecycle_state,
            legacy.credential_reference,
            legacy.provider_key,
            legacy.paired_at_utc,
            legacy.last_seen_at_utc
        ],
    )?;
    connection.execute(
        "INSERT INTO wrapper_credential_references (credential_reference,wrapper_id,connection_id,credential_kind,vault_service,vault_account,state) VALUES (?1,?2,?3,'connection_bundle',?4,?1,?5) ON CONFLICT(credential_reference) DO UPDATE SET wrapper_id=excluded.wrapper_id,connection_id=excluded.connection_id,state=excluded.state,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![
            legacy.credential_reference,
            wrapper_id,
            legacy.connection_id,
            CREDENTIAL_SERVICE,
            if legacy.state == "revoked" { "revoked" } else { "active" }
        ],
    )?;
    Ok(())
}

fn ensure_device_overlay(
    connection: &Connection,
    wrapper_id: &str,
    legacy: &LegacyConnection,
    installation_id: &str,
) -> Result<()> {
    let existing_id: Option<String> = connection
        .query_row(
            "SELECT wrapper_device_id FROM wrapper_devices WHERE connection_id=?1",
            params![legacy.connection_id],
            |row| row.get(0),
        )
        .optional()?;
    let wrapper_device_id = existing_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    connection.execute(
        "INSERT INTO wrapper_devices (wrapper_device_id,wrapper_id,connection_id,device_public_id,installation_id,public_key_base64,credential_reference,state,last_seen_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(connection_id) DO UPDATE SET wrapper_id=excluded.wrapper_id,device_public_id=excluded.device_public_id,installation_id=excluded.installation_id,public_key_base64=excluded.public_key_base64,credential_reference=excluded.credential_reference,state=excluded.state,last_seen_at_utc=excluded.last_seen_at_utc,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![
            wrapper_device_id,
            wrapper_id,
            legacy.connection_id,
            legacy.device_id,
            installation_id,
            legacy.public_key_base64,
            legacy.credential_reference,
            map_device_state(&legacy.state),
            legacy.last_seen_at_utc
        ],
    )?;
    Ok(())
}

fn expire_pairing_attempts(connection: &Connection) -> Result<()> {
    connection.execute(
        "UPDATE wrapper_pairing_attempts SET state='expired',error_code='pairing_expired',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='pending' AND expires_at_utc <= strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        [],
    )?;
    Ok(())
}

fn record_event(
    connection: &Connection,
    wrapper_id: &str,
    connection_id: Option<&str>,
    event_type: &str,
    outcome: &str,
    correlation_id: Option<&str>,
    metadata: Value,
) -> Result<()> {
    connection.execute(
        "INSERT INTO wrapper_events (event_id,wrapper_id,connection_id,event_type,outcome,correlation_id,visibility,metadata_json,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'security',?7,?8)",
        params![
            Uuid::new_v4().to_string(),
            wrapper_id,
            connection_id,
            event_type,
            outcome,
            correlation_id,
            serde_json::to_string(&metadata)?,
            now_utc()
        ],
    )?;
    Ok(())
}

fn read_legacy_connections(connection: &Connection) -> Result<Vec<LegacyConnection>> {
    let mut statement = connection.prepare(
        "SELECT connection_id,provider_key,display_name,cloud_base_url,device_id,public_key_base64,credential_key,state,paired_at_utc,last_success_utc FROM cloud_connections ORDER BY created_at_utc,connection_id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(LegacyConnection {
                connection_id: row.get(0)?,
                provider_key: row.get(1)?,
                display_name: row.get(2)?,
                remote_origin: row.get(3)?,
                device_id: row.get(4)?,
                public_key_base64: row.get(5)?,
                credential_reference: row.get(6)?,
                state: row.get(7)?,
                paired_at_utc: row.get(8)?,
                last_seen_at_utc: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn legacy_connection_by_id(
    connection: &Connection,
    connection_id: &str,
) -> Result<LegacyConnection> {
    connection
        .query_row(
            "SELECT connection_id,provider_key,display_name,cloud_base_url,device_id,public_key_base64,credential_key,state,paired_at_utc,last_success_utc FROM cloud_connections WHERE connection_id=?1",
            params![connection_id],
            |row| {
                Ok(LegacyConnection {
                    connection_id: row.get(0)?,
                    provider_key: row.get(1)?,
                    display_name: row.get(2)?,
                    remote_origin: row.get(3)?,
                    device_id: row.get(4)?,
                    public_key_base64: row.get(5)?,
                    credential_reference: row.get(6)?,
                    state: row.get(7)?,
                    paired_at_utc: row.get(8)?,
                    last_seen_at_utc: row.get(9)?,
                })
            },
        )
        .optional()?
        .context("paired cloud connection was not found")
}

fn read_wrappers(connection: &Connection) -> Result<Vec<WrapperIdentitySummary>> {
    let mut statement = connection.prepare(
        "SELECT wrapper_id,wrapper_key,display_name,wrapper_kind,protocol_version,state,created_at_utc,updated_at_utc,revoked_at_utc FROM wrapper_identities ORDER BY display_name,wrapper_id LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![MAX_WRAPPERS], |row| {
            Ok(WrapperIdentitySummary {
                wrapper_id: row.get(0)?,
                wrapper_key: row.get(1)?,
                display_name: row.get(2)?,
                wrapper_kind: row.get(3)?,
                protocol_version: row.get(4)?,
                state: row.get(5)?,
                created_at_utc: row.get(6)?,
                updated_at_utc: row.get(7)?,
                revoked_at_utc: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_connections(connection: &Connection) -> Result<Vec<WrapperConnectionSummary>> {
    let mut statement = connection.prepare(
        "SELECT connection_id,wrapper_id,remote_connection_id,remote_origin,contract_version,lifecycle_state,credential_reference,grant_revision,legacy_provider_key,legacy_connection_id,paired_at_utc,last_seen_at_utc,revoked_at_utc,created_at_utc,updated_at_utc FROM wrapper_connections ORDER BY updated_at_utc DESC,connection_id DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![MAX_CONNECTIONS], |row| {
            let grant_revision: i64 = row.get(7)?;
            Ok(WrapperConnectionSummary {
                connection_id: row.get(0)?,
                wrapper_id: row.get(1)?,
                remote_connection_id: row.get(2)?,
                remote_origin: row.get(3)?,
                contract_version: row.get(4)?,
                lifecycle_state: row.get(5)?,
                credential_reference: row.get(6)?,
                grant_revision: grant_revision.max(0) as u64,
                legacy_provider_key: row.get(8)?,
                legacy_connection_id: row.get(9)?,
                paired_at_utc: row.get(10)?,
                last_seen_at_utc: row.get(11)?,
                revoked_at_utc: row.get(12)?,
                created_at_utc: row.get(13)?,
                updated_at_utc: row.get(14)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_devices(connection: &Connection) -> Result<Vec<WrapperDeviceSummary>> {
    let mut statement = connection.prepare(
        "SELECT wrapper_device_id,wrapper_id,connection_id,device_public_id,installation_id,public_key_base64,credential_reference,state,last_seen_at_utc,created_at_utc,updated_at_utc,revoked_at_utc FROM wrapper_devices ORDER BY updated_at_utc DESC,wrapper_device_id DESC LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![MAX_DEVICES], |row| {
            Ok(WrapperDeviceSummary {
                wrapper_device_id: row.get(0)?,
                wrapper_id: row.get(1)?,
                connection_id: row.get(2)?,
                device_public_id: row.get(3)?,
                installation_id: row.get(4)?,
                public_key_base64: row.get(5)?,
                credential_reference: row.get(6)?,
                state: row.get(7)?,
                last_seen_at_utc: row.get(8)?,
                created_at_utc: row.get(9)?,
                updated_at_utc: row.get(10)?,
                revoked_at_utc: row.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_pairing_attempts(connection: &Connection) -> Result<Vec<WrapperPairingAttemptSummary>> {
    let mut statement = connection.prepare(
        "SELECT attempt_id,wrapper_id,request_id,remote_origin,device_display_name,requested_capabilities_json,state,expires_at_utc,result_connection_id,error_code,created_at_utc,completed_at_utc,updated_at_utc FROM wrapper_pairing_attempts ORDER BY created_at_utc DESC,attempt_id DESC LIMIT 250",
    )?;
    let rows = statement
        .query_map([], |row| {
            let capabilities_json: String = row.get(5)?;
            let requested_capabilities =
                serde_json::from_str::<Vec<String>>(&capabilities_json).unwrap_or_default();
            Ok(WrapperPairingAttemptSummary {
                attempt_id: row.get(0)?,
                wrapper_id: row.get(1)?,
                request_id: row.get(2)?,
                remote_origin: row.get(3)?,
                device_display_name: row.get(4)?,
                requested_capabilities,
                state: row.get(6)?,
                expires_at_utc: row.get(7)?,
                result_connection_id: row.get(8)?,
                error_code: row.get(9)?,
                created_at_utc: row.get(10)?,
                completed_at_utc: row.get(11)?,
                updated_at_utc: row.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn wrapper_by_key(
    connection: &Connection,
    wrapper_key: &str,
) -> Result<Option<WrapperIdentitySummary>> {
    connection
        .query_row(
            "SELECT wrapper_id,wrapper_key,display_name,wrapper_kind,protocol_version,state,created_at_utc,updated_at_utc,revoked_at_utc FROM wrapper_identities WHERE wrapper_key=?1",
            params![wrapper_key],
            wrapper_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn wrapper_by_id(connection: &Connection, wrapper_id: &str) -> Result<WrapperIdentitySummary> {
    connection
        .query_row(
            "SELECT wrapper_id,wrapper_key,display_name,wrapper_kind,protocol_version,state,created_at_utc,updated_at_utc,revoked_at_utc FROM wrapper_identities WHERE wrapper_id=?1",
            params![wrapper_id],
            wrapper_from_row,
        )
        .optional()?
        .context("wrapper identity was not found")
}

fn wrapper_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WrapperIdentitySummary> {
    Ok(WrapperIdentitySummary {
        wrapper_id: row.get(0)?,
        wrapper_key: row.get(1)?,
        display_name: row.get(2)?,
        wrapper_kind: row.get(3)?,
        protocol_version: row.get(4)?,
        state: row.get(5)?,
        created_at_utc: row.get(6)?,
        updated_at_utc: row.get(7)?,
        revoked_at_utc: row.get(8)?,
    })
}

fn validate_wrapper_key(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        (2..=40).contains(&value.len()),
        "wrapper key must contain 2 to 40 characters"
    );
    ensure!(
        value.chars().all(|character| character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | '-')),
        "wrapper key contains unsupported characters"
    );
    ensure!(
        value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase() || character.is_ascii_digit()),
        "wrapper key must begin with a letter or number"
    );
    Ok(value)
}

fn validate_request_id(value: &str) -> Result<String> {
    let value = bounded_text(value, 8, 160, "request ID")?;
    ensure!(
        value
            .chars()
            .all(|character| character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | ':' | '.')),
        "request ID contains unsupported characters"
    );
    Ok(value)
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    Uuid::parse_str(value).with_context(|| format!("{label} must be a UUID"))?;
    Ok(value.to_owned())
}

fn normalize_capabilities(values: Vec<String>) -> Result<Vec<String>> {
    ensure!(
        values.len() <= MAX_CAPABILITIES,
        "too many pairing capabilities were requested"
    );
    let mut normalized = values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    for capability in &normalized {
        ensure!(
            (3..=120).contains(&capability.len())
                && capability.chars().all(|character| {
                    character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || matches!(character, '.' | ':' | '_' | '-')
                }),
            "pairing capability identifier is invalid"
        );
    }
    Ok(normalized)
}

fn validate_remote_origin(value: &str) -> Result<String> {
    let value = bounded_text(value, 8, 500, "remote origin")?;
    let url = Url::parse(&value).context("remote origin is not a valid URL")?;
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "remote origin must not contain credentials"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "remote origin must not contain a query or fragment"
    );
    ensure!(
        url.path().is_empty() || url.path() == "/",
        "remote origin must not contain a path"
    );
    let host = url.host_str().context("remote origin requires a host")?;
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "remote origin must use HTTPS outside loopback tests"
    );
    let mut normalized = url;
    normalized.set_path("");
    Ok(normalized.to_string().trim_end_matches('/').to_owned())
}

fn normalize_origin_for_compare(value: &str) -> Result<String> {
    validate_remote_origin(value)
}

fn bounded_text(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        (minimum..=maximum).contains(&value.chars().count()),
        "{label} must contain between {minimum} and {maximum} characters"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} contains control characters"
    );
    Ok(value.to_owned())
}

fn map_connection_state(state: &str) -> &'static str {
    match state {
        "pairing" => "pairing_pending",
        "connected" => "active",
        "degraded" => "offline",
        "revoked" => "revoked",
        "disconnected" => "disconnected",
        _ => "error",
    }
}

fn map_device_state(state: &str) -> &'static str {
    match state {
        "pairing" => "pairing_pending",
        "connected" => "active",
        "degraded" => "offline",
        "revoked" => "revoked",
        "disconnected" => "suspended",
        _ => "error",
    }
}

fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    api_error(
        "wrapper_registry_task_failed",
        anyhow::anyhow!("wrapper registry task failed: {error}"),
    )
}

fn api_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(?error, code, "wrapper registry request rejected");
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_wrapper_keys() {
        assert_eq!(
            validate_wrapper_key("RSS-POD").expect("valid key"),
            "rss-pod"
        );
        assert!(validate_wrapper_key("../pod").is_err());
        assert!(validate_wrapper_key("x").is_err());
    }

    #[test]
    fn rejects_unsafe_remote_origins() {
        assert!(validate_remote_origin("https://pod.example.com").is_ok());
        assert!(validate_remote_origin("http://127.0.0.1:8080").is_ok());
        assert!(validate_remote_origin("http://pod.example.com").is_err());
        assert!(validate_remote_origin("https://user:pass@pod.example.com").is_err());
        assert!(validate_remote_origin("https://pod.example.com/path").is_err());
    }

    #[test]
    fn normalizes_capabilities_without_duplicates() {
        let values = normalize_capabilities(vec![
            "knowledge.search".to_owned(),
            "KNOWLEDGE.SEARCH".to_owned(),
            "agent.request".to_owned(),
        ])
        .expect("capabilities should be valid");
        assert_eq!(values, vec!["agent.request", "knowledge.search"]);
    }
}
