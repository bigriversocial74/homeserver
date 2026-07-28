use crate::{AppState, database};
use anyhow::{anyhow, bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer, SigningKey};
use keyring::Entry;
use rand::rngs::OsRng;
use reqwest::Method;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::{Path, PathBuf}, process::Stdio, sync::Arc, time::{Duration, Instant}};
use tokio::{fs, process::Command, sync::watch, time::timeout};
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

const MIGRATION: &str = include_str!("../../../../database/migrations/0015_pod_provider_voice_adapter.sql");
const MIGRATION_KEY: &str = "0015_pod_provider_voice_adapter";
const PROVIDER_KEY: &str = "pod";
const CONTRACT_VERSION: &str = "pod-homeserver-voice-1";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServerPodConnections";
const INSTALLATION_SERVICE: &str = "MicrogifterHomeServerPodIdentity";
const INSTALLATION_ACCOUNT: &str = "installation-id";
const LEASE_SERVICE: &str = "MicrogifterHomeServerPodJobLeases";
const PAIR_PATH: &str = "/api/homeserver/v1/pairing/exchange";
const HEARTBEAT_PATH: &str = "/api/homeserver/v1/devices/heartbeat";
const POLL_PATH: &str = "/api/homeserver/v1/voice/jobs/poll";
const COMPLETE_PATH: &str = "/api/homeserver/v1/voice/jobs/complete";
const FAIL_PATH: &str = "/api/homeserver/v1/voice/jobs/fail";
const ARTIFACT_PATH: &str = "/api/homeserver/v1/voice/artifacts/read";
const MAX_CONTROL_BODY_BYTES: usize = 128 * 1024;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const HEARTBEAT_INTERVAL_SECONDS: i64 = 45;
const WORKER_INTERVAL: Duration = Duration::from_secs(12);
const JOB_HISTORY_DAYS: i64 = 30;
const RECEIPT_HISTORY_DAYS: i64 = 90;
const CAPABILITIES: &[&str] = &[
    "pod.pairing.v1",
    "pod.device-heartbeat.v1",
    "pod.voice.jobs.v1",
    "pod.voice.transcription.v1",
    "pod.voice.synthesis.v1",
    "pod.voice.artifacts.v1",
    "pod.voice.receipts.v1",
    "pod.receptionist.context.v1",
];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecrets {
    bearer_token: String,
    signing_seed_base64: String,
    device_id: String,
    provider_connection_id: String,
}

impl Drop for StoredSecrets {
    fn drop(&mut self) {
        self.bearer_token.zeroize();
        self.signing_seed_base64.zeroize();
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PairRequest {
    pub display_name: String,
    pub pod_base_url: String,
    pub sync_code: String,
    #[serde(default)]
    pub make_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionRequest {
    pub connection_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeUpdateRequest {
    pub connection_id: String,
    #[serde(default)]
    pub transcription_enabled: bool,
    pub transcription_executable: Option<String>,
    #[serde(default)]
    pub transcription_arguments: Vec<String>,
    pub transcription_model: Option<String>,
    #[serde(default)]
    pub synthesis_enabled: bool,
    pub synthesis_executable: Option<String>,
    #[serde(default)]
    pub synthesis_arguments: Vec<String>,
    pub synthesis_model: Option<String>,
    pub synthesis_voice: Option<String>,
    #[serde(default = "default_timeout")]
    pub execution_timeout_seconds: u64,
    #[serde(default = "default_maximum_bytes")]
    pub maximum_input_bytes: usize,
    #[serde(default = "default_maximum_bytes")]
    pub maximum_output_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeProfile {
    pub connection_id: String,
    pub transcription_enabled: bool,
    pub transcription_executable: Option<String>,
    pub transcription_arguments: Vec<String>,
    pub transcription_model: Option<String>,
    pub synthesis_enabled: bool,
    pub synthesis_executable: Option<String>,
    pub synthesis_arguments: Vec<String>,
    pub synthesis_model: Option<String>,
    pub synthesis_voice: Option<String>,
    pub execution_timeout_seconds: u64,
    pub maximum_input_bytes: usize,
    pub maximum_output_bytes: usize,
    pub runtime_state: String,
    pub runtime_health_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionSummary {
    pub connection_id: String,
    pub display_name: String,
    pub pod_base_url: String,
    pub provider_connection_id: String,
    pub provider_identity_id: String,
    pub provider_display_name: String,
    pub device_id: String,
    pub state: String,
    pub lifecycle_state: String,
    pub granted_capabilities: Vec<String>,
    pub runtime: RuntimeProfile,
    pub last_heartbeat_at_utc: Option<String>,
    pub last_poll_at_utc: Option<String>,
    pub last_job_completed_at_utc: Option<String>,
    pub last_error: Option<String>,
    pub queued_jobs: u64,
    pub active_jobs: u64,
    pub failed_jobs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub local_job_id: String,
    pub connection_id: String,
    pub remote_job_uuid: String,
    pub job_type: String,
    pub state: String,
    pub attempt_count: u32,
    pub maximum_attempts: u32,
    pub model_name: Option<String>,
    pub processing_ms: Option<u64>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub leased_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReceiptSummary {
    pub receipt_id: String,
    pub connection_id: String,
    pub local_job_id: Option<String>,
    pub event_type: String,
    pub outcome: String,
    pub detail_code: Option<String>,
    pub metadata: Value,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusSnapshot {
    pub local_operation_available: bool,
    pub provider_key: &'static str,
    pub contract_version: &'static str,
    pub connector_version: &'static str,
    pub worker_enabled: bool,
    pub supported_capabilities: Vec<String>,
    pub connections: Vec<ConnectionSummary>,
    pub recent_jobs: Vec<JobSummary>,
    pub recent_receipts: Vec<ReceiptSummary>,
    pub privacy_boundary: Vec<String>,
}

#[derive(Debug, Clone)]
struct ConnectionRecord {
    connection_id: String,
    display_name: String,
    base_url: String,
    provider_connection_id: String,
    provider_identity_id: String,
    provider_display_name: String,
    device_id: String,
    credential_key: String,
    state: String,
    lifecycle_state: String,
    capabilities: Vec<String>,
    last_heartbeat_at_utc: Option<String>,
}

#[derive(Debug, Serialize)]
struct PairingPayload<'a> {
    schema_version: u32,
    provider_key: &'a str,
    sync_code: &'a str,
    request_id: &'a str,
    installation_id: &'a str,
    device_display_name: &'a str,
    homeserver_version: &'a str,
    device_public_key: &'a str,
    requested_capabilities: &'a [String],
}

#[derive(Debug, Deserialize)]
struct ProviderEnvelope<T> {
    ok: bool,
    message: String,
    data: T,
}

#[derive(Debug, Deserialize)]
struct PairingData {
    provider_id: String,
    provider_connection_id: String,
    provider_identity_id: String,
    provider_display_name: String,
    device_id: String,
    device_token: String,
    #[serde(default)]
    granted_capabilities: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HeartbeatPayload {
    homeserver_version: String,
    supported_capabilities: Vec<String>,
    voice_runtime_health: String,
    active_voice_jobs: u64,
}

#[derive(Debug, Deserialize)]
struct HeartbeatData {
    receipt_id: String,
    connection_state: String,
    queued_voice_jobs: u64,
}

#[derive(Debug, Deserialize)]
struct PollData {
    job: Option<RemoteJob>,
}

#[derive(Debug, Deserialize)]
struct RemoteJob {
    job_uuid: String,
    job_type: String,
    priority: String,
    payload: Value,
    payload_hash: String,
    input_artifact: Option<ArtifactReference>,
    lease_token: String,
    lease_expires_at: String,
    attempt_count: u32,
    max_attempts: u32,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactReference {
    artifact_uuid: String,
    mime_type: String,
    plaintext_bytes: usize,
    content_hash: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactData {
    artifact_uuid: String,
    mime_type: String,
    content_hash: String,
    plaintext_bytes: usize,
    content_base64: String,
}

#[derive(Debug, Deserialize)]
struct RuntimeResult {
    transcript: Option<String>,
    language: Option<String>,
    confidence: Option<f64>,
    model: Option<String>,
    processing_ms: Option<u64>,
    audio_path: Option<String>,
    mime_type: Option<String>,
    details: Option<String>,
}

struct ProviderClient {
    record: ConnectionRecord,
    bearer_token: String,
    signing_key: SigningKey,
    client: reqwest::Client,
}

impl Drop for ProviderClient {
    fn drop(&mut self) {
        self.bearer_token.zeroize();
    }
}

fn default_timeout() -> u64 { 120 }
fn default_maximum_bytes() -> usize { 8 * 1024 * 1024 }

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(count == 1, "POD provider migration is not registered exactly once");
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    for table in [
        "pod_provider_connections",
        "pod_provider_runtime_profiles",
        "pod_provider_voice_jobs",
        "pod_provider_runtime_receipts",
        "pod_provider_worker_state",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let worker: i64 = connection.query_row(
        "SELECT COUNT(*) FROM pod_provider_worker_state WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    ensure!(worker == 1, "POD provider worker state is unavailable");
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM pod_provider_runtime_receipts WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now',?1)",
        params![format!("-{RECEIPT_HISTORY_DAYS} days")],
    )?;
    connection.execute(
        "DELETE FROM pod_provider_voice_jobs WHERE state IN ('completed','failed','cancelled') AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now',?1)",
        params![format!("-{JOB_HISTORY_DAYS} days")],
    )?;
    connection.execute(
        "DELETE FROM provider_pairing_attempts WHERE provider_key='pod' AND state IN ('completed','failed','expired') AND COALESCE(completed_at_utc,started_at_utc) < strftime('%Y-%m-%dT%H:%M:%fZ','now','-30 days')",
        [],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/providers/pod/status", get(status_handler))
        .route("/v1/providers/pod/connect", post(connect_handler))
        .route("/v1/providers/pod/runtime", post(runtime_handler))
        .route("/v1/providers/pod/poll", post(poll_handler))
        .route("/v1/providers/pod/disconnect", post(disconnect_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(WORKER_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(error) = run_cycle(state.clone()).await {
                    warn!(?error, "POD provider worker cycle failed");
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    info!("POD provider voice worker stopped");
                    return;
                }
            }
        }
    }
}

async fn status_handler(State(state): State<Arc<AppState>>) -> ApiResult<StatusSnapshot> {
    tokio::task::spawn_blocking(move || status_snapshot(&*state.connection()?))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("pod_status_failed", error))
}

async fn connect_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PairRequest>,
) -> ApiResult<ConnectionSummary> {
    connect(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("pod_pairing_failed", error))
}

async fn runtime_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RuntimeUpdateRequest>,
) -> ApiResult<RuntimeProfile> {
    tokio::task::spawn_blocking(move || save_runtime(&*state.connection()?, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("pod_runtime_update_failed", error))
}

async fn poll_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionRequest>,
) -> ApiResult<Value> {
    run_connection(state, request.connection_id.trim())
        .await
        .map(|processed| Json(json!({"ok": true, "processed_jobs": processed})))
        .map_err(|error| action_error("pod_poll_failed", error))
}

async fn disconnect_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionRequest>,
) -> ApiResult<Value> {
    tokio::task::spawn_blocking(move || disconnect(&*state.connection()?, request.connection_id.trim()))
        .await
        .map_err(task_error)?
        .map(|_| Json(json!({"ok": true})))
        .map_err(|error| action_error("pod_disconnect_failed", error))
}

async fn run_cycle(state: Arc<AppState>) -> Result<()> {
    let connection_ids = {
        let connection = state.connection()?;
        connection.prepare(
            "SELECT connection_id FROM cloud_connections WHERE provider_key='pod' AND state IN ('connected','degraded') ORDER BY paired_at_utc,connection_id"
        )?.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    {
        let connection = state.connection()?;
        connection.execute(
            "UPDATE pod_provider_worker_state SET last_cycle_started_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_connection_count=?1,last_job_count=0,last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
            params![connection_ids.len() as i64],
        )?;
    }
    let mut processed = 0_i64;
    let mut last_error: Option<String> = None;
    for connection_id in connection_ids {
        match run_connection(state.clone(), &connection_id).await {
            Ok(count) => processed += count as i64,
            Err(error) => {
                let message = bounded(&error.to_string(), 500);
                last_error = Some(message.clone());
                let connection = state.connection()?;
                mark_connection_error(&connection, &connection_id, "pod_worker_cycle_failed", &message)?;
            }
        }
    }
    let connection = state.connection()?;
    connection.execute(
        "UPDATE pod_provider_worker_state SET last_cycle_completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_job_count=?1,last_error=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![processed, last_error],
    )?;
    maintain_history(&connection)
}

async fn connect(state: Arc<AppState>, request: PairRequest) -> Result<ConnectionSummary> {
    let display_name = required(&request.display_name, 120, "device display name")?;
    let base_url = normalize_url(&request.pod_base_url)?;
    let sync_code = request.sync_code.trim().to_ascii_uppercase();
    ensure!(valid_sync_code(&sync_code), "enter a valid POD Sync Code");
    let installation_id = installation_id()?;
    let connection_id = Uuid::new_v4().to_string();
    let request_id = format!("pod-pair-{}", Uuid::new_v4());
    let credential_key = format!("pod-connection-{connection_id}");
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().to_bytes());
    let requested_capabilities = CAPABILITIES.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>();
    let payload = PairingPayload {
        schema_version: 1,
        provider_key: PROVIDER_KEY,
        sync_code: &sync_code,
        request_id: &request_id,
        installation_id: &installation_id,
        device_display_name: &display_name,
        homeserver_version: env!("CARGO_PKG_VERSION"),
        device_public_key: &public_key,
        requested_capabilities: &requested_capabilities,
    };
    {
        let connection = state.connection()?;
        connection.execute(
            "INSERT INTO provider_pairing_attempts (attempt_id,provider_key,request_id,cloud_base_url,device_display_name,state,started_at_utc) VALUES (?1,'pod',?2,?3,?4,'pending',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![Uuid::new_v4().to_string(), request_id, base_url.as_str(), display_name],
        )?;
    }
    let envelope: ProviderEnvelope<PairingData> = unsigned_request(&base_url, PAIR_PATH, &payload).await?;
    ensure!(envelope.ok, "POD pairing was rejected: {}", envelope.message);
    let paired = envelope.data;
    ensure!(paired.provider_id == PROVIDER_KEY, "POD pairing returned an unexpected provider");
    ensure!(paired.device_token.len() == 64, "POD pairing returned an invalid bearer credential");
    ensure!(!paired.provider_connection_id.is_empty() && !paired.device_id.is_empty(), "POD pairing response is incomplete");
    ensure!(paired.granted_capabilities.iter().all(|value| CAPABILITIES.contains(&value.as_str())), "POD pairing returned an unsupported capability");
    let secrets = StoredSecrets {
        bearer_token: paired.device_token,
        signing_seed_base64: URL_SAFE_NO_PAD.encode(signing_key.to_bytes()),
        device_id: paired.device_id.clone(),
        provider_connection_id: paired.provider_connection_id.clone(),
    };
    save_secrets(&credential_key, &secrets)?;
    let now = now();
    let mut connection = state.connection()?;
    let tx = connection.transaction()?;
    if request.make_default { tx.execute("UPDATE cloud_connections SET is_default=0", [])?; }
    tx.execute(
        "INSERT INTO cloud_connections (connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,public_key_base64,credential_key,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error,created_at_utc,updated_at_utc) VALUES (?1,'pod',?2,?3,NULL,NULL,?4,?5,?6,'connected',?7,?8,?9,?9,NULL,?9,?9)",
        params![connection_id, display_name, base_url.as_str(), paired.device_id, public_key, credential_key, serde_json::to_string(&paired.granted_capabilities)?, if request.make_default {1} else {0}, now],
    )?;
    tx.execute(
        "INSERT INTO provider_connection_profiles (connection_id,provider_key,provider_connection_id,contract_version,lifecycle_state,owner_account_id,device_display_name,connector_version,capability_registry_version,subscription_state,update_eligible,last_heartbeat_at_utc,created_at_utc,updated_at_utc) VALUES (?1,'pod',?2,?3,'active',?4,?5,?6,'1','unknown',0,?7,?7,?7)",
        params![connection_id, paired.provider_connection_id, CONTRACT_VERSION, paired.provider_identity_id, display_name, env!("CARGO_PKG_VERSION"), now],
    )?;
    tx.execute(
        "INSERT INTO pod_provider_connections (connection_id,provider_connection_id,provider_identity_id,provider_display_name,contract_version,device_signing_key_name,runtime_state,last_heartbeat_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'unconfigured',?7,?7,?7)",
        params![connection_id, paired.provider_connection_id, paired.provider_identity_id, paired.provider_display_name, CONTRACT_VERSION, credential_key, now],
    )?;
    tx.execute("INSERT INTO pod_provider_runtime_profiles (connection_id) VALUES (?1)", params![connection_id])?;
    tx.execute(
        "UPDATE provider_pairing_attempts SET state='completed',connection_id=?1,completed_at_utc=?2,error_code=NULL WHERE provider_key='pod' AND request_id=?3",
        params![connection_id, now, request_id],
    )?;
    for capability in &paired.granted_capabilities {
        tx.execute(
            "INSERT INTO provider_connection_capabilities (connection_id,capability_id,grant_state,source,expires_at_utc,updated_at_utc) VALUES (?1,?2,'granted','server',NULL,?3) ON CONFLICT(connection_id,capability_id,source) DO UPDATE SET grant_state='granted',updated_at_utc=excluded.updated_at_utc",
            params![connection_id, capability, now],
        )?;
    }
    receipt_tx(&tx, &connection_id, None, "pod.paired", "success", Some("paired"), &json!({"provider_connection_id": paired.provider_connection_id, "provider_identity_id": paired.provider_identity_id, "device_id": paired.device_id}))?;
    tx.commit()?;
    info!(%connection_id, pod_base_url = %base_url, "paired POD provider connection");
    connection_summary(&state.connection()?, &connection_id)
}

fn save_runtime(connection: &Connection, request: RuntimeUpdateRequest) -> Result<RuntimeProfile> {
    let connection_id = required(&request.connection_id, 64, "connection ID")?;
    let provider: Option<String> = connection.query_row(
        "SELECT provider_key FROM cloud_connections WHERE connection_id=?1",
        params![connection_id],
        |row| row.get(0),
    ).optional()?;
    ensure!(provider.as_deref() == Some(PROVIDER_KEY), "POD connection was not found");
    validate_arguments(&request.transcription_arguments)?;
    validate_arguments(&request.synthesis_arguments)?;
    let transcription_executable = validate_executable(request.transcription_enabled, request.transcription_executable, "transcription")?;
    let synthesis_executable = validate_executable(request.synthesis_enabled, request.synthesis_executable, "synthesis")?;
    ensure!((5..=1800).contains(&request.execution_timeout_seconds), "runtime timeout is outside the supported range");
    ensure!((262_144..=16_777_216).contains(&request.maximum_input_bytes), "maximum input bytes are outside the supported range");
    ensure!((262_144..=16_777_216).contains(&request.maximum_output_bytes), "maximum output bytes are outside the supported range");
    let state = runtime_state(request.transcription_enabled, transcription_executable.as_deref(), request.synthesis_enabled, synthesis_executable.as_deref());
    let health = runtime_health(request.transcription_enabled, transcription_executable.as_deref(), request.synthesis_enabled, synthesis_executable.as_deref());
    connection.execute(
        "INSERT INTO pod_provider_runtime_profiles (connection_id,transcription_enabled,transcription_executable,transcription_arguments_json,transcription_model,synthesis_enabled,synthesis_executable,synthesis_arguments_json,synthesis_model,synthesis_voice,execution_timeout_seconds,maximum_input_bytes,maximum_output_bytes,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(connection_id) DO UPDATE SET transcription_enabled=excluded.transcription_enabled,transcription_executable=excluded.transcription_executable,transcription_arguments_json=excluded.transcription_arguments_json,transcription_model=excluded.transcription_model,synthesis_enabled=excluded.synthesis_enabled,synthesis_executable=excluded.synthesis_executable,synthesis_arguments_json=excluded.synthesis_arguments_json,synthesis_model=excluded.synthesis_model,synthesis_voice=excluded.synthesis_voice,execution_timeout_seconds=excluded.execution_timeout_seconds,maximum_input_bytes=excluded.maximum_input_bytes,maximum_output_bytes=excluded.maximum_output_bytes,updated_at_utc=excluded.updated_at_utc",
        params![connection_id, if request.transcription_enabled {1} else {0}, transcription_executable, serde_json::to_string(&request.transcription_arguments)?, optional(request.transcription_model,190), if request.synthesis_enabled {1} else {0}, synthesis_executable, serde_json::to_string(&request.synthesis_arguments)?, optional(request.synthesis_model,190), optional(request.synthesis_voice,190), request.execution_timeout_seconds as i64, request.maximum_input_bytes as i64, request.maximum_output_bytes as i64],
    )?;
    connection.execute(
        "UPDATE pod_provider_connections SET runtime_state=?1,runtime_health_message=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?3",
        params![state, health, connection_id],
    )?;
    receipt(connection, &connection_id, None, "pod.runtime.updated", "success", Some(&state), &json!({"transcription_enabled": request.transcription_enabled, "synthesis_enabled": request.synthesis_enabled}))?;
    runtime_profile(connection, &connection_id)
}

async fn run_connection(state: Arc<AppState>, connection_id: &str) -> Result<u64> {
    let record = {
        let connection = state.connection()?;
        connection_record(&connection, connection_id)?
    };
    ensure!(record.state != "disconnected" && record.lifecycle_state != "revoked", "POD connection is inactive");
    let client = ProviderClient::new(record.clone(), load_secrets(&record.credential_key)?)?;
    let heartbeat_due = record.last_heartbeat_at_utc.as_deref().and_then(parse_time).map(|value| Utc::now().timestamp() - value >= HEARTBEAT_INTERVAL_SECONDS).unwrap_or(true);
    if heartbeat_due {
        let runtime = { let connection = state.connection()?; runtime_profile(&connection, connection_id)? };
        let active = { let connection = state.connection()?; count_jobs(&connection, connection_id, &["leased","processing","retrying"])? };
        let payload = HeartbeatPayload {
            homeserver_version: env!("CARGO_PKG_VERSION").to_owned(),
            supported_capabilities: CAPABILITIES.iter().map(|value| (*value).to_owned()).collect(),
            voice_runtime_health: runtime.runtime_state,
            active_voice_jobs: active,
        };
        let envelope: ProviderEnvelope<HeartbeatData> = client.signed(Method::POST, HEARTBEAT_PATH, &payload).await?;
        ensure!(envelope.ok, "POD heartbeat was rejected: {}", envelope.message);
        let connection = state.connection()?;
        connection.execute("UPDATE cloud_connections SET state='connected',last_success_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1", params![connection_id])?;
        connection.execute("UPDATE provider_connection_profiles SET lifecycle_state='active',last_heartbeat_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1", params![connection_id])?;
        connection.execute("UPDATE pod_provider_connections SET last_heartbeat_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1", params![connection_id])?;
        receipt(&connection, connection_id, None, "pod.heartbeat", "success", Some(&envelope.data.connection_state), &json!({"receipt_id": envelope.data.receipt_id, "queued_voice_jobs": envelope.data.queued_voice_jobs}))?;
    }
    let envelope: ProviderEnvelope<PollData> = client.signed(Method::POST, POLL_PATH, &json!({})).await?;
    ensure!(envelope.ok, "POD job poll was rejected: {}", envelope.message);
    { let connection = state.connection()?; connection.execute("UPDATE pod_provider_connections SET last_poll_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1", params![connection_id])?; }
    let Some(job) = envelope.data.job else { return Ok(0); };
    process_job(state, &client, job).await?;
    Ok(1)
}

async fn process_job(state: Arc<AppState>, client: &ProviderClient, job: RemoteJob) -> Result<()> {
    validate_job(&client.record, &job)?;
    let local_job_id = Uuid::new_v4().to_string();
    let lease_key = format!("pod-job-lease-{local_job_id}");
    save_lease(&lease_key, &job.lease_token)?;
    {
        let connection = state.connection()?;
        connection.execute(
            "INSERT INTO pod_provider_voice_jobs (local_job_id,connection_id,remote_job_uuid,job_type,state,lease_credential_key,lease_token_hint,payload_hash,attempt_count,maximum_attempts,lease_expires_at_utc,remote_expires_at_utc,leased_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,'leased',?5,?6,?7,?8,?9,?10,?11,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(connection_id,remote_job_uuid) DO UPDATE SET state='leased',lease_credential_key=excluded.lease_credential_key,lease_token_hint=excluded.lease_token_hint,payload_hash=excluded.payload_hash,attempt_count=excluded.attempt_count,maximum_attempts=excluded.maximum_attempts,lease_expires_at_utc=excluded.lease_expires_at_utc,remote_expires_at_utc=excluded.remote_expires_at_utc,updated_at_utc=excluded.updated_at_utc",
            params![local_job_id, client.record.connection_id, job.job_uuid, job.job_type, lease_key, hint(&job.lease_token), job.payload_hash, job.attempt_count as i64, job.max_attempts as i64, job.lease_expires_at, job.expires_at],
        )?;
        receipt(&connection, &client.record.connection_id, Some(&local_job_id), "pod.voice.job.leased", "success", Some(&job.job_type), &json!({"priority": job.priority, "attempt_count": job.attempt_count}))?;
    }
    let result = execute_job(&state, client, &local_job_id, &job).await;
    match result {
        Ok((provider_result, model, processing_ms)) => {
            let payload = json!({"job_uuid": job.job_uuid, "lease_token": load_lease(&lease_key)?, "result": provider_result});
            let envelope: ProviderEnvelope<Value> = client.signed(Method::POST, COMPLETE_PATH, &payload).await?;
            ensure!(envelope.ok, "POD completion was rejected: {}", envelope.message);
            let result_hash = envelope.data.get("result_hash").and_then(Value::as_str).map(ToOwned::to_owned);
            let connection = state.connection()?;
            connection.execute("UPDATE pod_provider_voice_jobs SET state='completed',result_hash=?1,model_name=?2,processing_ms=?3,completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),failure_code=NULL,failure_message=NULL WHERE local_job_id=?4", params![result_hash, model, processing_ms.map(|value| value as i64), local_job_id])?;
            connection.execute("UPDATE pod_provider_connections SET last_job_completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1", params![client.record.connection_id])?;
            receipt(&connection, &client.record.connection_id, Some(&local_job_id), "pod.voice.job.completed", "success", Some(&job.job_type), &json!({"result_hash": result_hash, "processing_ms": processing_ms}))?;
            delete_lease(&lease_key);
        }
        Err(error) => {
            let message = bounded(&error.to_string(), 500);
            let code = error_code(&message);
            let retryable = matches!(code.as_str(), "pod_runtime_timeout" | "pod_runtime_failed") && job.attempt_count < job.max_attempts;
            let payload = json!({"job_uuid": job.job_uuid, "lease_token": load_lease(&lease_key)?, "failure_code": code, "failure_message": message, "retryable": retryable});
            let envelope: ProviderEnvelope<Value> = client.signed(Method::POST, FAIL_PATH, &payload).await?;
            ensure!(envelope.ok, "POD failure receipt was rejected: {}", envelope.message);
            let remote_state = envelope.data.get("status").and_then(Value::as_str).unwrap_or(if retryable {"queued"} else {"failed"});
            let local_state = if remote_state == "queued" {"retrying"} else {"failed"};
            let connection = state.connection()?;
            connection.execute("UPDATE pod_provider_voice_jobs SET state=?1,failure_code=?2,failure_message=?3,completed_at_utc=CASE WHEN ?1='failed' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE completed_at_utc END,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE local_job_id=?4", params![local_state, code, message, local_job_id])?;
            receipt(&connection, &client.record.connection_id, Some(&local_job_id), "pod.voice.job.failed", if retryable {"warning"} else {"error"}, Some(&code), &json!({"retryable": retryable}))?;
            delete_lease(&lease_key);
        }
    }
    Ok(())
}

async fn execute_job(state: &Arc<AppState>, client: &ProviderClient, local_job_id: &str, job: &RemoteJob) -> Result<(Value, Option<String>, Option<u64>)> {
    let runtime = { let connection = state.connection()?; runtime_profile(&connection, &client.record.connection_id)? };
    { let connection = state.connection()?; connection.execute("UPDATE pod_provider_voice_jobs SET state='processing',started_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE local_job_id=?1", params![local_job_id])?; }
    let started = Instant::now();
    match job.job_type.as_str() {
        "capability_test" => Ok((json!({
            "runtime": "homeserver-local-command-v1",
            "models": [runtime.transcription_model.clone(), runtime.synthesis_model.clone()].into_iter().flatten().collect::<Vec<_>>(),
            "transcription_ready": runtime.transcription_enabled && executable_ready(runtime.transcription_executable.as_deref()),
            "synthesis_ready": runtime.synthesis_enabled && executable_ready(runtime.synthesis_executable.as_deref()),
            "details": runtime.runtime_health_message,
        }), None, Some(started.elapsed().as_millis() as u64))),
        "speech_to_text" => {
            ensure!(runtime.transcription_enabled, "POD transcription runtime is disabled");
            let executable = runtime.transcription_executable.clone().ok_or_else(|| anyhow!("POD transcription runtime is not configured"))?;
            let artifact = job.input_artifact.as_ref().ok_or_else(|| anyhow!("POD transcription job has no input artifact"))?;
            ensure!(artifact.plaintext_bytes <= runtime.maximum_input_bytes, "POD transcription input exceeds the local runtime limit");
            let payload = json!({"job_uuid": job.job_uuid, "lease_token": load_lease_for_job(state, local_job_id)?, "artifact_uuid": artifact.artifact_uuid});
            let envelope: ProviderEnvelope<ArtifactData> = client.signed(Method::POST, ARTIFACT_PATH, &payload).await?;
            ensure!(envelope.ok, "POD artifact read was rejected: {}", envelope.message);
            let data = envelope.data;
            ensure!(data.artifact_uuid == artifact.artifact_uuid, "POD artifact identity mismatch");
            ensure!(data.mime_type == artifact.mime_type && data.content_hash == artifact.content_hash, "POD artifact metadata mismatch");
            let bytes = STANDARD.decode(&data.content_base64).context("POD artifact base64 is invalid")?;
            ensure!(bytes.len() == data.plaintext_bytes && bytes.len() <= runtime.maximum_input_bytes, "POD artifact size mismatch");
            ensure!(sha256(&bytes) == data.content_hash, "POD artifact hash mismatch");
            let work = work_directory(state, local_job_id).await?;
            let input = work.join(format!("input.{}", extension(&data.mime_type).unwrap_or("audio")));
            let output = work.join("result.json");
            fs::write(&input, bytes).await?;
            let mut args = runtime.transcription_arguments.clone();
            args.extend(["--input".to_owned(), input.to_string_lossy().to_string(), "--output".to_owned(), output.to_string_lossy().to_string(), "--job-id".to_owned(), job.job_uuid.clone(), "--language".to_owned(), job.payload.get("language").and_then(Value::as_str).unwrap_or("en-US").to_owned()]);
            let result = run_command(&executable, &args, runtime.execution_timeout_seconds, runtime.maximum_output_bytes, &output).await?;
            let transcript = result.transcript.unwrap_or_default().trim().to_owned();
            ensure!(!transcript.is_empty() && transcript.len() <= 12_000, "local transcription result is empty or too large");
            let processing_ms = result.processing_ms.unwrap_or(started.elapsed().as_millis() as u64);
            let model = result.model.clone().or(runtime.transcription_model);
            cleanup(&work).await;
            Ok((json!({"transcript": transcript, "language": result.language.unwrap_or_else(|| "en-US".to_owned()), "confidence": result.confidence.unwrap_or(0.0).clamp(0.0,1.0), "model": model.clone().unwrap_or_else(|| "local-runtime".to_owned()), "processing_ms": processing_ms}), model, Some(processing_ms)))
        }
        "text_to_speech" => {
            ensure!(runtime.synthesis_enabled, "POD synthesis runtime is disabled");
            let executable = runtime.synthesis_executable.clone().ok_or_else(|| anyhow!("POD synthesis runtime is not configured"))?;
            let text = job.payload.get("text").and_then(Value::as_str).unwrap_or("").trim();
            ensure!(!text.is_empty() && text.len() <= 6_000, "POD synthesis text is empty or too large");
            let format = job.payload.get("audio_format").and_then(Value::as_str).unwrap_or("mp3");
            ensure!(["mp3","wav","ogg","webm"].contains(&format), "POD synthesis format is unsupported");
            let work = work_directory(state, local_job_id).await?;
            let input = work.join("request.json");
            let audio = work.join(format!("output.{format}"));
            let output = work.join("result.json");
            fs::write(&input, serde_json::to_vec(&json!({"text": text, "language": job.payload.get("language").and_then(Value::as_str).unwrap_or("en-US"), "voice": job.payload.get("voice").and_then(Value::as_str).filter(|value| !value.is_empty()).or(runtime.synthesis_voice.as_deref()), "model": runtime.synthesis_model, "audio_format": format, "job_uuid": job.job_uuid}))?).await?;
            let mut args = runtime.synthesis_arguments.clone();
            args.extend(["--input-json".to_owned(), input.to_string_lossy().to_string(), "--output-audio".to_owned(), audio.to_string_lossy().to_string(), "--output-json".to_owned(), output.to_string_lossy().to_string(), "--job-id".to_owned(), job.job_uuid.clone()]);
            let result = run_command(&executable, &args, runtime.execution_timeout_seconds, runtime.maximum_output_bytes, &output).await?;
            let audio_path = result.audio_path.as_deref().map(PathBuf::from).unwrap_or(audio);
            ensure!(audio_path.starts_with(&work), "local synthesis returned an unsafe output path");
            let bytes = fs::read(&audio_path).await?;
            ensure!(!bytes.is_empty() && bytes.len() <= runtime.maximum_output_bytes, "local synthesis output is empty or too large");
            let mime = result.mime_type.unwrap_or_else(|| mime_type(format).to_owned());
            ensure!(["audio/mpeg","audio/wav","audio/ogg","audio/webm"].contains(&mime.as_str()), "local synthesis MIME type is unsupported");
            let processing_ms = result.processing_ms.unwrap_or(started.elapsed().as_millis() as u64);
            let model = result.model.clone().or(runtime.synthesis_model);
            let encoded = STANDARD.encode(bytes);
            cleanup(&work).await;
            Ok((json!({"audio_base64": encoded, "mime_type": mime, "model": model.clone().unwrap_or_else(|| "local-runtime".to_owned()), "processing_ms": processing_ms}), model, Some(processing_ms)))
        }
        _ => bail!("unsupported POD job type"),
    }
}

async fn run_command(executable: &str, arguments: &[String], timeout_seconds: u64, maximum_output_bytes: usize, result_path: &Path) -> Result<RuntimeResult> {
    let path = Path::new(executable);
    ensure!(path.is_absolute() && path.is_file(), "local voice runtime executable is unavailable");
    let output = timeout(Duration::from_secs(timeout_seconds), Command::new(path).args(arguments).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).output()).await.map_err(|_| anyhow!("local voice runtime timed out"))??;
    let stderr = bounded(&String::from_utf8_lossy(&output.stderr), 500);
    ensure!(output.status.success(), "local voice runtime failed: {stderr}");
    let metadata = fs::metadata(result_path).await.context("local voice runtime did not create result JSON")?;
    ensure!(metadata.len() as usize <= maximum_output_bytes.min(1024 * 1024), "local voice runtime result JSON is too large");
    let bytes = fs::read(result_path).await?;
    serde_json::from_slice(&bytes).context("local voice runtime result JSON is invalid")
}

fn status_snapshot(connection: &Connection) -> Result<StatusSnapshot> {
    let enabled = connection.query_row("SELECT enabled FROM pod_provider_worker_state WHERE singleton_id=1", [], |row| row.get::<_, i64>(0))? != 0;
    let ids = connection.prepare("SELECT connection_id FROM cloud_connections WHERE provider_key='pod' ORDER BY is_default DESC,paired_at_utc DESC")?.query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let connections = ids.iter().map(|id| connection_summary(connection,id)).collect::<Result<Vec<_>>>()?;
    Ok(StatusSnapshot {
        local_operation_available: true,
        provider_key: PROVIDER_KEY,
        contract_version: CONTRACT_VERSION,
        connector_version: env!("CARGO_PKG_VERSION"),
        worker_enabled: enabled,
        supported_capabilities: CAPABILITIES.iter().map(|value| (*value).to_owned()).collect(),
        connections,
        recent_jobs: recent_jobs(connection,100)?,
        recent_receipts: recent_receipts(connection,100)?,
        privacy_boundary: vec![
            "No Knowledge Vault contents are sent to the POD provider".to_owned(),
            "No unrelated wrapper or provider data is exposed".to_owned(),
            "POD bearer, signing seed, and job leases stay in the operating-system credential vault".to_owned(),
            "Local runtime commands execute without a shell and only from absolute configured paths".to_owned(),
        ],
    })
}

fn connection_summary(connection: &Connection, id: &str) -> Result<ConnectionSummary> {
    let record = connection_record(connection,id)?;
    Ok(ConnectionSummary {
        connection_id: record.connection_id.clone(),
        display_name: record.display_name,
        pod_base_url: record.base_url,
        provider_connection_id: record.provider_connection_id,
        provider_identity_id: record.provider_identity_id,
        provider_display_name: record.provider_display_name,
        device_id: record.device_id,
        state: record.state,
        lifecycle_state: record.lifecycle_state,
        granted_capabilities: record.capabilities,
        runtime: runtime_profile(connection,id)?,
        last_heartbeat_at_utc: record.last_heartbeat_at_utc,
        last_poll_at_utc: connection.query_row("SELECT last_poll_at_utc FROM pod_provider_connections WHERE connection_id=?1", params![id], |row| row.get(0)).optional()?.flatten(),
        last_job_completed_at_utc: connection.query_row("SELECT last_job_completed_at_utc FROM pod_provider_connections WHERE connection_id=?1", params![id], |row| row.get(0)).optional()?.flatten(),
        last_error: connection.query_row("SELECT last_error FROM cloud_connections WHERE connection_id=?1", params![id], |row| row.get(0)).optional()?.flatten(),
        queued_jobs: count_jobs(connection,id,&["retrying"] )?,
        active_jobs: count_jobs(connection,id,&["leased","processing"] )?,
        failed_jobs: count_jobs(connection,id,&["failed"] )?,
    })
}

fn connection_record(connection: &Connection, id: &str) -> Result<ConnectionRecord> {
    let row = connection.query_row(
        "SELECT c.connection_id,c.display_name,c.cloud_base_url,p.provider_connection_id,p.provider_identity_id,p.provider_display_name,c.device_id,c.credential_key,c.state,pp.lifecycle_state,c.scopes_json,p.last_heartbeat_at_utc FROM cloud_connections c JOIN provider_connection_profiles pp ON pp.connection_id=c.connection_id JOIN pod_provider_connections p ON p.connection_id=c.connection_id WHERE c.connection_id=?1 AND c.provider_key='pod'",
        params![id],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,String>(7)?,row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?,row.get::<_,Option<String>>(11)?)),
    ).optional()?.ok_or_else(|| anyhow!("POD connection was not found"))?;
    Ok(ConnectionRecord { connection_id: row.0, display_name: row.1, base_url: row.2, provider_connection_id: row.3, provider_identity_id: row.4, provider_display_name: row.5, device_id: row.6, credential_key: row.7, state: row.8, lifecycle_state: row.9, capabilities: serde_json::from_str(&row.10)?, last_heartbeat_at_utc: row.11 })
}

fn runtime_profile(connection: &Connection, id: &str) -> Result<RuntimeProfile> {
    connection.query_row(
        "SELECT r.connection_id,r.transcription_enabled,r.transcription_executable,r.transcription_arguments_json,r.transcription_model,r.synthesis_enabled,r.synthesis_executable,r.synthesis_arguments_json,r.synthesis_model,r.synthesis_voice,r.execution_timeout_seconds,r.maximum_input_bytes,r.maximum_output_bytes,p.runtime_state,p.runtime_health_message FROM pod_provider_runtime_profiles r JOIN pod_provider_connections p ON p.connection_id=r.connection_id WHERE r.connection_id=?1",
        params![id],
        |row| {
            let trans_args: String = row.get(3)?; let synth_args: String = row.get(7)?;
            Ok(RuntimeProfile { connection_id: row.get(0)?, transcription_enabled: row.get::<_,i64>(1)? != 0, transcription_executable: row.get(2)?, transcription_arguments: serde_json::from_str(&trans_args).unwrap_or_default(), transcription_model: row.get(4)?, synthesis_enabled: row.get::<_,i64>(5)? != 0, synthesis_executable: row.get(6)?, synthesis_arguments: serde_json::from_str(&synth_args).unwrap_or_default(), synthesis_model: row.get(8)?, synthesis_voice: row.get(9)?, execution_timeout_seconds: row.get::<_,i64>(10)?.max(5) as u64, maximum_input_bytes: row.get::<_,i64>(11)?.max(262_144) as usize, maximum_output_bytes: row.get::<_,i64>(12)?.max(262_144) as usize, runtime_state: row.get(13)?, runtime_health_message: row.get(14)? })
        }
    ).map_err(Into::into)
}

fn recent_jobs(connection: &Connection, limit: usize) -> Result<Vec<JobSummary>> {
    let mut statement = connection.prepare("SELECT local_job_id,connection_id,remote_job_uuid,job_type,state,attempt_count,maximum_attempts,model_name,processing_ms,failure_code,failure_message,leased_at_utc,completed_at_utc FROM pod_provider_voice_jobs ORDER BY updated_at_utc DESC,local_job_id DESC LIMIT ?1")?;
    Ok(statement.query_map(params![limit.clamp(1,250) as i64], |row| Ok(JobSummary { local_job_id: row.get(0)?, connection_id: row.get(1)?, remote_job_uuid: row.get(2)?, job_type: row.get(3)?, state: row.get(4)?, attempt_count: row.get::<_,i64>(5)?.max(0) as u32, maximum_attempts: row.get::<_,i64>(6)?.max(1) as u32, model_name: row.get(7)?, processing_ms: row.get::<_,Option<i64>>(8)?.map(|value| value.max(0) as u64), failure_code: row.get(9)?, failure_message: row.get(10)?, leased_at_utc: row.get(11)?, completed_at_utc: row.get(12)? }))?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn recent_receipts(connection: &Connection, limit: usize) -> Result<Vec<ReceiptSummary>> {
    let mut statement = connection.prepare("SELECT receipt_id,connection_id,local_job_id,event_type,outcome,detail_code,metadata_json,created_at_utc FROM pod_provider_runtime_receipts ORDER BY created_at_utc DESC,receipt_id DESC LIMIT ?1")?;
    Ok(statement.query_map(params![limit.clamp(1,250) as i64], |row| { let metadata: String = row.get(6)?; Ok(ReceiptSummary { receipt_id: row.get(0)?, connection_id: row.get(1)?, local_job_id: row.get(2)?, event_type: row.get(3)?, outcome: row.get(4)?, detail_code: row.get(5)?, metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})), created_at_utc: row.get(7)? }) })?.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn disconnect(connection: &Connection, id: &str) -> Result<()> {
    let id = required(id,64,"connection ID")?;
    let record = connection_record(connection,&id)?;
    connection.execute("UPDATE cloud_connections SET state='disconnected',is_default=0,last_error='Disconnected by the owner',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1", params![id])?;
    connection.execute("UPDATE provider_connection_profiles SET lifecycle_state='revoked',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1", params![id])?;
    connection.execute("UPDATE pod_provider_voice_jobs SET state='cancelled',failure_code='connection_disconnected',failure_message='POD provider connection was disconnected',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1 AND state IN ('leased','processing','retrying')", params![id])?;
    let keys = connection.prepare("SELECT lease_credential_key FROM pod_provider_voice_jobs WHERE connection_id=?1")?.query_map(params![id], |row| row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for key in keys { delete_lease(&key); }
    delete_secrets(&record.credential_key);
    receipt(connection,&id,None,"pod.disconnected","success",Some("owner_disconnected"),&json!({}))
}

impl ProviderClient {
    fn new(record: ConnectionRecord, secrets: StoredSecrets) -> Result<Self> {
        ensure!(secrets.device_id == record.device_id && secrets.provider_connection_id == record.provider_connection_id, "POD credential identity mismatch");
        let seed = URL_SAFE_NO_PAD.decode(secrets.signing_seed_base64.as_bytes())?;
        let seed: [u8;32] = seed.try_into().map_err(|_| anyhow!("POD signing seed has an invalid length"))?;
        Ok(Self { record, bearer_token: secrets.bearer_token, signing_key: SigningKey::from_bytes(&seed), client: reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(45)).build()? })
    }
    async fn signed<T: Serialize + ?Sized, R: DeserializeOwned>(&self, method: Method, path: &str, body: &T) -> Result<R> {
        let bytes = serde_json::to_vec(body)?;
        ensure!(bytes.len() <= MAX_PROVIDER_RESPONSE_BYTES, "POD request is too large");
        let timestamp = Utc::now().timestamp().to_string();
        let nonce = Uuid::new_v4().to_string();
        let canonical = canonical(&method,path,&timestamp,&nonce,&bytes);
        let signature = URL_SAFE_NO_PAD.encode(self.signing_key.sign(canonical.as_bytes()).to_bytes());
        let response = self.client.request(method, endpoint(&self.record.base_url,path)?).bearer_auth(&self.bearer_token).header("Content-Type","application/json").header("Accept","application/json").header("X-POD-Homeserver-ID",&self.record.device_id).header("X-POD-Connection-ID",&self.record.provider_connection_id).header("X-POD-Timestamp",timestamp).header("X-POD-Nonce",nonce).header("X-POD-Signature",signature).header("X-POD-Homeserver-Version",env!("CARGO_PKG_VERSION")).body(bytes).send().await?;
        parse_response(response).await
    }
}

async fn unsigned_request<T: Serialize + ?Sized, R: DeserializeOwned>(base: &Url, path: &str, body: &T) -> Result<R> {
    let response = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(30)).build()?.post(endpoint(base.as_str(),path)?).header("Content-Type","application/json").header("Accept","application/json").json(body).send().await?;
    parse_response(response).await
}

async fn parse_response<R: DeserializeOwned>(response: reqwest::Response) -> Result<R> {
    let status = response.status(); let bytes = response.bytes().await?;
    ensure!(bytes.len() <= MAX_PROVIDER_RESPONSE_BYTES, "POD response is too large");
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes).ok().and_then(|value| value.get("message").and_then(Value::as_str).map(ToOwned::to_owned)).unwrap_or_else(|| format!("POD provider returned HTTP {status}"));
        bail!("{}", bounded(&message,500));
    }
    serde_json::from_slice(&bytes).context("POD response JSON is invalid")
}

fn normalize_url(value: &str) -> Result<Url> {
    let mut url = Url::parse(value.trim()).context("enter a valid POD URL")?;
    ensure!(url.username().is_empty() && url.password().is_none(), "POD URL must not contain credentials");
    ensure!(url.query().is_none() && url.fragment().is_none(), "POD URL must not contain query or fragment");
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let local = matches!(host.as_str(),"localhost"|"127.0.0.1"|"::1");
    ensure!(url.scheme()=="https" || (url.scheme()=="http" && local), "POD URL must use HTTPS outside localhost tests");
    url.set_path(""); url.set_query(None); url.set_fragment(None); Ok(url)
}

fn endpoint(base: &str, path: &str) -> Result<Url> { let mut url = normalize_url(base)?; url.set_path(path); Ok(url) }
fn canonical(method: &Method,path: &str,timestamp: &str,nonce: &str,body: &[u8]) -> String { format!("{}\n{}\n{}\n{}\n{}",method.as_str(),path,timestamp,nonce,sha256(body)) }
fn sha256(bytes: &[u8]) -> String { format!("{:x}",Sha256::digest(bytes)) }
fn now() -> String { Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true) }
fn parse_time(value: &str) -> Option<i64> { DateTime::parse_from_rfc3339(value).ok().map(|value| value.timestamp()) }
fn valid_sync_code(value: &str) -> bool { let parts=value.split('-').collect::<Vec<_>>(); parts.len()==7 && parts[0]=="POD" && parts[1..].iter().all(|part| part.len()==4 && part.chars().all(|c| c.is_ascii_hexdigit())) }
fn required(value: &str,max: usize,label: &str)->Result<String>{ let value=value.trim(); ensure!(!value.is_empty(),"{label} is required"); ensure!(value.len()<=max,"{label} is too long"); Ok(value.to_owned()) }
fn optional(value: Option<String>,max: usize)->Option<String>{ value.map(|value| value.trim().chars().take(max).collect::<String>()).filter(|value| !value.is_empty()) }
fn bounded(value: &str,max: usize)->String{ value.trim().chars().take(max).collect() }
fn hint(value: &str)->String{ if value.len()<12 {"hidden".to_owned()} else {format!("{}…{}",&value[..6],&value[value.len()-4..])} }
fn mime_type(format:&str)->&'static str{ match format {"wav"=>"audio/wav","ogg"=>"audio/ogg","webm"=>"audio/webm",_=>"audio/mpeg"} }
fn extension(mime:&str)->Option<&'static str>{ match mime {"audio/mpeg"=>Some("mp3"),"audio/wav"|"audio/x-wav"=>Some("wav"),"audio/ogg"=>Some("ogg"),"audio/webm"=>Some("webm"),"audio/mp4"=>Some("m4a"),_=>None} }
fn error_code(message:&str)->String{ let value=message.to_ascii_lowercase(); if value.contains("not configured")||value.contains("disabled"){"pod_runtime_unconfigured".to_owned()} else if value.contains("timed out"){"pod_runtime_timeout".to_owned()} else if value.contains("artifact"){"pod_artifact_invalid".to_owned()} else if value.contains("too large"){"pod_runtime_output_too_large".to_owned()} else {"pod_runtime_failed".to_owned()} }
fn count_jobs(connection:&Connection,id:&str,states:&[&str])->Result<u64>{ if states.is_empty(){return Ok(0)} let placeholders=(1..=states.len()).map(|index|format!("?{}",index+1)).collect::<Vec<_>>().join(","); let sql=format!("SELECT COUNT(*) FROM pod_provider_voice_jobs WHERE connection_id=?1 AND state IN ({placeholders})"); let mut values:Vec<&dyn rusqlite::ToSql>=vec![&id]; for state in states {values.push(state)} let count:i64=connection.query_row(&sql,values.as_slice(),|row|row.get(0))?; Ok(count.max(0) as u64) }
fn validate_job(record:&ConnectionRecord,job:&RemoteJob)->Result<()>{ ensure!(Uuid::parse_str(&job.job_uuid).is_ok(),"POD job UUID is invalid"); ensure!(["speech_to_text","text_to_speech","capability_test"].contains(&job.job_type.as_str()),"POD job type is unsupported"); ensure!(job.lease_token.len()==64&&job.lease_token.chars().all(|c|c.is_ascii_hexdigit()),"POD lease token is invalid"); ensure!(sha256(&serde_json::to_vec(&job.payload)?)==job.payload_hash,"POD payload hash mismatch"); let capability=match job.job_type.as_str(){"speech_to_text"=>"pod.voice.transcription.v1","text_to_speech"=>"pod.voice.synthesis.v1",_=>"pod.voice.jobs.v1"}; ensure!(record.capabilities.iter().any(|value|value==capability),"POD capability is not granted"); Ok(()) }
fn validate_arguments(values:&[String])->Result<()>{ ensure!(values.len()<=64,"runtime argument list is too large"); for value in values {ensure!(value.len()<=500&&!value.contains('\0'),"runtime argument is invalid")} Ok(()) }
fn validate_executable(enabled:bool,value:Option<String>,label:&str)->Result<Option<String>>{ let value=value.map(|value|value.trim().to_owned()).filter(|value|!value.is_empty()); if enabled {let path=value.as_deref().map(Path::new).ok_or_else(||anyhow!("{label} executable is required"))?; ensure!(path.is_absolute()&&path.is_file(),"{label} executable is unavailable")} Ok(value) }
fn executable_ready(value:Option<&str>)->bool{ value.map(Path::new).is_some_and(|path|path.is_absolute()&&path.is_file()) }
fn runtime_state(te:bool,tp:Option<&str>,se:bool,sp:Option<&str>)->String{ if !te&&!se {"unconfigured".to_owned()} else if (!te||executable_ready(tp))&&(!se||executable_ready(sp)){"ready".to_owned()} else {"degraded".to_owned()} }
fn runtime_health(te:bool,tp:Option<&str>,se:bool,sp:Option<&str>)->Option<String>{ let mut issues=Vec::new(); if te&&!executable_ready(tp){issues.push("transcription executable unavailable")} if se&&!executable_ready(sp){issues.push("synthesis executable unavailable")} Some(if issues.is_empty(){if te||se{"configured local voice runtimes are available".to_owned()}else{"no local voice runtime is configured".to_owned()}}else{issues.join("; ")}) }

async fn work_directory(state:&Arc<AppState>,id:&str)->Result<PathBuf>{ ensure!(id.chars().all(|c|c.is_ascii_alphanumeric()||c=='-'),"invalid local job ID"); let path=state.config.data_dir.join("pod-provider-runtime").join("jobs").join(id); fs::create_dir_all(&path).await?; Ok(path) }
async fn cleanup(path:&Path){ if let Err(error)=fs::remove_dir_all(path).await {warn!(?error,path=%path.display(),"unable to remove POD job directory")} }

fn installation_id()->Result<String>{ let entry=Entry::new(INSTALLATION_SERVICE,INSTALLATION_ACCOUNT)?; match entry.get_password(){Ok(value) if Uuid::parse_str(value.trim()).is_ok()=>Ok(value.trim().to_owned()),_=>{let value=Uuid::new_v4().to_string(); entry.set_password(&value)?; Ok(value)}} }
fn secret_entry(key:&str)->Result<Entry>{ Entry::new(CREDENTIAL_SERVICE,key).map_err(Into::into) }
fn save_secrets(key:&str,value:&StoredSecrets)->Result<()>{ secret_entry(key)?.set_password(&serde_json::to_string(value)?)?; Ok(()) }
fn load_secrets(key:&str)->Result<StoredSecrets>{ serde_json::from_str(&secret_entry(key)?.get_password()?).map_err(Into::into) }
fn delete_secrets(key:&str){ if let Ok(entry)=secret_entry(key){let _=entry.delete_credential();} }
fn lease_entry(key:&str)->Result<Entry>{ Entry::new(LEASE_SERVICE,key).map_err(Into::into) }
fn save_lease(key:&str,value:&str)->Result<()>{ lease_entry(key)?.set_password(value)?; Ok(()) }
fn load_lease(key:&str)->Result<String>{ lease_entry(key)?.get_password().map_err(Into::into) }
fn delete_lease(key:&str){ if let Ok(entry)=lease_entry(key){let _=entry.delete_credential();} }
fn load_lease_for_job(state:&Arc<AppState>,id:&str)->Result<String>{ let connection=state.connection()?; let key:String=connection.query_row("SELECT lease_credential_key FROM pod_provider_voice_jobs WHERE local_job_id=?1",params![id],|row|row.get(0))?; load_lease(&key) }

fn receipt(connection:&Connection,connection_id:&str,job_id:Option<&str>,event:&str,outcome:&str,code:Option<&str>,metadata:&Value)->Result<()>{ connection.execute("INSERT INTO pod_provider_runtime_receipts (receipt_id,connection_id,local_job_id,event_type,outcome,detail_code,metadata_json,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![Uuid::new_v4().to_string(),connection_id,job_id,event,outcome,code.map(|value|bounded(value,100)),serde_json::to_string(metadata)?])?; Ok(()) }
fn receipt_tx(tx:&rusqlite::Transaction<'_>,connection_id:&str,job_id:Option<&str>,event:&str,outcome:&str,code:Option<&str>,metadata:&Value)->Result<()>{ tx.execute("INSERT INTO pod_provider_runtime_receipts (receipt_id,connection_id,local_job_id,event_type,outcome,detail_code,metadata_json,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",params![Uuid::new_v4().to_string(),connection_id,job_id,event,outcome,code.map(|value|bounded(value,100)),serde_json::to_string(metadata)?])?; Ok(()) }
fn mark_connection_error(connection:&Connection,id:&str,code:&str,message:&str)->Result<()>{ connection.execute("UPDATE cloud_connections SET state='degraded',last_error=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?2 AND state!='disconnected'",params![bounded(message,500),id])?; connection.execute("UPDATE provider_connection_profiles SET lifecycle_state='error',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",params![id])?; connection.execute("UPDATE pod_provider_connections SET last_error_code=?1,last_error_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?2",params![bounded(code,100),id])?; receipt(connection,id,None,"pod.worker.error","warning",Some(code),&json!({"message":bounded(message,500)})) }

fn internal_error(code:&'static str,error:anyhow::Error)->(StatusCode,Json<ApiError>){(StatusCode::INTERNAL_SERVER_ERROR,Json(ApiError{ok:false,error:code,message:bounded(&error.to_string(),500)}))}
fn action_error(code:&'static str,error:anyhow::Error)->(StatusCode,Json<ApiError>){(StatusCode::UNPROCESSABLE_ENTITY,Json(ApiError{ok:false,error:code,message:bounded(&error.to_string(),500)}))}
fn task_error(error:tokio::task::JoinError)->(StatusCode,Json<ApiError>){internal_error("pod_task_failed",anyhow!(error))}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_contract_matches_pod_server(){ let body=br#"{"voice_runtime_health":"healthy"}"#; assert_eq!(canonical(&Method::POST,HEARTBEAT_PATH,"1785273600","fixture-nonce",body),format!("POST\n{}\n1785273600\nfixture-nonce\n{}",HEARTBEAT_PATH,sha256(body))); }
    #[test]
    fn sync_code_is_strict(){ assert!(valid_sync_code("POD-1111-2222-3333-4444-5555-6666")); assert!(!valid_sync_code("POD-1111-2222")); }
    #[test]
    fn provider_url_requires_https(){ assert!(normalize_url("https://pod.example/path").is_ok()); assert!(normalize_url("http://localhost:8080").is_ok()); assert!(normalize_url("http://pod.example").is_err()); }
}
