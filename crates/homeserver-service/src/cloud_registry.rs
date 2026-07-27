use crate::{database, AppState};
use anyhow::{anyhow, bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode as HttpStatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::StreamExt;
use keyring::Entry;
use rand::rngs::OsRng;
use reqwest::Method;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

const REGISTRY_MIGRATION: &str =
    include_str!("../../../database/migrations/0010_multi_cloud_connections.sql");
const REGISTRY_MIGRATION_KEY: &str = "0010_multi_cloud_connections";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";
const PAIR_PATH: &str = "/api/homeserver/pair.php";
const STATUS_PATH: &str = "/api/homeserver/status.php";
const SYNC_PATH: &str = "/api/homeserver/sync.php";
const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;
const MAX_SYNC_PAYLOAD_BYTES: usize = 48 * 1024;
const MAX_CLOUD_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CONNECTIONS: i64 = 50;
const MAX_PENDING_SYNC_OPERATIONS_PER_CONNECTION: u64 = 5_000;
const MAX_SYNC_ATTEMPTS: u32 = 12;
const SYNC_INTERVAL: Duration = Duration::from_secs(60);
const ALLOWED_PROVIDERS: &[&str] = &["microgifter"];
const ALLOWED_LOCAL_OPERATIONS: &[&str] = &[
    "device.heartbeat",
    "local.settings.snapshot",
    "cache.refresh.request",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudRegistryConnectionState {
    Pairing,
    Connected,
    Degraded,
    Revoked,
    Disconnected,
}

impl CloudRegistryConnectionState {
    fn from_database(value: &str) -> Self {
        match value {
            "pairing" => Self::Pairing,
            "connected" => Self::Connected,
            "degraded" => Self::Degraded,
            "revoked" => Self::Revoked,
            "disconnected" => Self::Disconnected,
            _ => Self::Degraded,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudConnectionSummary {
    pub connection_id: String,
    pub provider_key: String,
    pub display_name: String,
    pub cloud_base_url: String,
    pub tenant_id: Option<String>,
    pub site_id: Option<String>,
    pub device_id: String,
    pub state: CloudRegistryConnectionState,
    pub scopes: Vec<String>,
    pub is_default: bool,
    pub paired_at_utc: String,
    pub last_success_utc: Option<String>,
    pub last_error: Option<String>,
    pub pending_sync: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudConnectionsSnapshot {
    pub connections: Vec<CloudConnectionSummary>,
    pub active_connections: u64,
    pub pending_sync: u64,
    pub default_connection_id: Option<String>,
    pub supported_providers: Vec<String>,
    pub local_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCloudConnectionRequest {
    pub provider_key: String,
    pub display_name: String,
    pub cloud_base_url: String,
    pub pairing_code: String,
    pub tenant_id: Option<String>,
    pub site_id: Option<String>,
    pub make_default: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConnectionRequest {
    pub connection_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnqueueConnectionSyncRequest {
    pub connection_id: String,
    pub operation_type: String,
    #[serde(default)]
    pub payload: Value,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionSyncRunSnapshot {
    pub connection_id: String,
    pub processed: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub review: u64,
    pub pending: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AllConnectionsSyncSnapshot {
    pub runs: Vec<ConnectionSyncRunSnapshot>,
    pub processed: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub review: u64,
    pub pending: u64,
}

#[derive(Debug, Serialize)]
struct EnqueueResult {
    connection_id: String,
    idempotency_key: String,
}

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (HttpStatusCode, Json<ApiError>)>;

#[derive(Debug)]
struct DeviceSecrets {
    device_token: String,
    signing_key_base64: String,
}

impl Drop for DeviceSecrets {
    fn drop(&mut self) {
        self.device_token.zeroize();
        self.signing_key_base64.zeroize();
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredDeviceSecrets {
    device_token: String,
    signing_key_base64: String,
}

#[derive(Debug, Clone)]
struct CloudConnectionRecord {
    summary: CloudConnectionSummary,
    credential_key: String,
}

#[derive(Debug, Clone)]
struct QueuedOperation {
    queue_id: i64,
    idempotency_key: String,
    operation_type: String,
    payload: Value,
    attempts: u32,
}

#[derive(Debug, Clone)]
struct ReceiptRecord {
    receipt_id: String,
    idempotency_key: String,
    operation_type: String,
    disposition: String,
    reason_code: Option<String>,
    response: Value,
}

#[derive(Debug)]
struct PairingOutcome {
    cloud_base_url: String,
    device_id: String,
    scopes: Vec<String>,
    public_key_base64: String,
    secrets: DeviceSecrets,
}

#[derive(Debug, Serialize)]
struct PairingPayload<'a> {
    pairing_code: &'a str,
    installation_id: &'a str,
    server_name: &'a str,
    version: &'a str,
    public_key: &'a str,
}

#[derive(Debug, Deserialize)]
struct PairingData {
    device_id: String,
    device_token: String,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SyncPayload<'a> {
    operations: &'a [SyncOperation],
}

#[derive(Debug, Serialize)]
struct SyncOperation {
    idempotency_key: String,
    operation_type: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
struct SyncData {
    receipts: Vec<SyncReceipt>,
}

#[derive(Debug, Deserialize)]
struct SyncReceipt {
    receipt_id: Option<String>,
    idempotency_key: String,
    operation_type: String,
    disposition: String,
    reason_code: Option<String>,
    response: Value,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    ok: bool,
    message: String,
    data: Option<T>,
}

#[derive(Clone)]
struct MicrogifterCloudClient {
    client: reqwest::Client,
}

impl MicrogifterCloudClient {
    fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(20))
                .user_agent(format!(
                    "Microgifter-HomeServer/{}",
                    env!("CARGO_PKG_VERSION")
                ))
                .build()?,
        })
    }

    async fn pair(
        &self,
        cloud_base_url: &str,
        pairing_code: &str,
        installation_id: &str,
        server_name: &str,
    ) -> Result<PairingOutcome> {
        let cloud_base_url = normalize_cloud_base_url(cloud_base_url)?;
        let pairing_code = pairing_code.trim();
        if !(20..=80).contains(&pairing_code.len()) {
            bail!("pairing code must contain between 20 and 80 characters");
        }

        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key_base64 = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().as_bytes());
        let payload = PairingPayload {
            pairing_code,
            installation_id,
            server_name,
            version: env!("CARGO_PKG_VERSION"),
            public_key: &public_key_base64,
        };
        let response = self
            .client
            .post(format!("{cloud_base_url}{PAIR_PATH}"))
            .json(&payload)
            .send()
            .await
            .context("unable to reach the Microgifter pairing service")?;
        let data: PairingData = decode_response(response).await?;
        if Uuid::parse_str(&data.device_id).is_err() {
            bail!("Microgifter returned an invalid HomeServer device identity");
        }
        if data.device_token.len() < 32 || data.scopes.is_empty() {
            bail!("Microgifter returned incomplete HomeServer credentials");
        }

        Ok(PairingOutcome {
            cloud_base_url,
            device_id: data.device_id,
            scopes: data.scopes,
            public_key_base64,
            secrets: DeviceSecrets {
                device_token: data.device_token,
                signing_key_base64: URL_SAFE_NO_PAD.encode(signing_key.to_bytes()),
            },
        })
    }

    async fn status(&self, record: &CloudConnectionRecord, secrets: &DeviceSecrets) -> Result<()> {
        let _: Value = self
            .signed_request(Method::GET, STATUS_PATH, "", record, secrets)
            .await?;
        Ok(())
    }

    async fn sync(
        &self,
        record: &CloudConnectionRecord,
        secrets: &DeviceSecrets,
        operations: &[QueuedOperation],
    ) -> Result<Vec<ReceiptRecord>> {
        if operations.is_empty() {
            return Ok(Vec::new());
        }
        let sync_operations = operations
            .iter()
            .map(|operation| SyncOperation {
                idempotency_key: operation.idempotency_key.clone(),
                operation_type: operation.operation_type.clone(),
                payload: operation.payload.clone(),
            })
            .collect::<Vec<_>>();
        let body = serde_json::to_string(&SyncPayload {
            operations: &sync_operations,
        })?;
        let data: SyncData = self
            .signed_request(Method::POST, SYNC_PATH, &body, record, secrets)
            .await?;
        Ok(data
            .receipts
            .into_iter()
            .map(|receipt| ReceiptRecord {
                receipt_id: receipt
                    .receipt_id
                    .unwrap_or_else(|| format!("local-rejected:{}", receipt.idempotency_key)),
                idempotency_key: receipt.idempotency_key,
                operation_type: receipt.operation_type,
                disposition: receipt.disposition,
                reason_code: receipt.reason_code,
                response: receipt.response,
            })
            .collect())
    }

    async fn signed_request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &str,
        record: &CloudConnectionRecord,
        secrets: &DeviceSecrets,
    ) -> Result<T> {
        let timestamp = Utc::now().timestamp().to_string();
        let nonce = Uuid::new_v4().simple().to_string();
        let canonical = canonical_request(&method, path, &timestamp, &nonce, body);
        let signing_bytes = URL_SAFE_NO_PAD
            .decode(&secrets.signing_key_base64)
            .context("HomeServer signing key is invalid")?;
        let signing_array: [u8; 32] = signing_bytes
            .try_into()
            .map_err(|_| anyhow!("HomeServer signing key has an invalid length"))?;
        let signing_key = SigningKey::from_bytes(&signing_array);
        let signature = URL_SAFE_NO_PAD.encode(signing_key.sign(canonical.as_bytes()).to_bytes());

        let mut request = self
            .client
            .request(method, format!("{}{}", record.summary.cloud_base_url, path))
            .bearer_auth(&secrets.device_token)
            .header("X-MG-Homeserver-ID", &record.summary.device_id)
            .header("X-MG-Timestamp", timestamp)
            .header("X-MG-Nonce", nonce)
            .header("X-MG-Signature", signature)
            .header("X-MG-Homeserver-Version", env!("CARGO_PKG_VERSION"));
        if !body.is_empty() {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_owned());
        }
        decode_response(
            request
                .send()
                .await
                .context("Microgifter cloud request failed")?,
        )
        .await
    }
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(REGISTRY_MIGRATION)?;
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![REGISTRY_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "cloud registry migration is not registered exactly once"
    );
    migrate_legacy_connection(connection)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    for table in [
        "cloud_connections",
        "cloud_sync_queue",
        "cloud_sync_receipts",
        "cloud_connection_events",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/cloud/connections", get(connections_snapshot))
        .route("/v1/cloud/connections/pair", post(pair_connection))
        .route(
            "/v1/cloud/connections/disconnect",
            post(disconnect_connection),
        )
        .route("/v1/cloud/connections/enqueue", post(enqueue_sync))
        .route("/v1/cloud/connections/sync", post(sync_connection))
        .route("/v1/cloud/connections/sync-all", post(sync_all_connections))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(SYNC_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    info!("multi-connection synchronization worker stopped");
                    return;
                }
            }
            _ = interval.tick() => {
                let connection_ids = match state.active_cloud_connection_ids() {
                    Ok(ids) => ids,
                    Err(error) => {
                        warn!(?error, "unable to inspect cloud connection registry");
                        continue;
                    }
                };
                for connection_id in connection_ids {
                    if let Err(error) = state.enqueue_connection_heartbeat(&connection_id) {
                        warn!(?error, %connection_id, "unable to queue connection heartbeat");
                    }
                    if let Err(error) = state.sync_cloud_connection(&connection_id).await {
                        warn!(?error, %connection_id, "cloud connection synchronization failed");
                    }
                }
            }
        }
    }
}

impl AppState {
    pub(crate) fn cloud_connections_snapshot(&self) -> Result<CloudConnectionsSnapshot> {
        registry_snapshot(&self.connection()?)
    }

    fn active_cloud_connection_ids(&self) -> Result<Vec<String>> {
        active_connection_ids(&self.connection()?)
    }

    async fn pair_cloud_connection(
        &self,
        request: PairCloudConnectionRequest,
    ) -> Result<CloudConnectionSummary> {
        let provider_key = normalize_provider_key(&request.provider_key)?;
        let display_name =
            sanitize_optional_text(Some(&request.display_name), 120, "display name")?
                .context("connection display name is required")?;
        let tenant_id = sanitize_optional_text(request.tenant_id.as_deref(), 120, "tenant id")?;
        let site_id = sanitize_optional_text(request.site_id.as_deref(), 120, "site id")?;
        let installation_id = database::installation_id(&self.connection()?)?;
        let connection_id = Uuid::new_v4().to_string();
        let credential_key = format!("{installation_id}:cloud:{connection_id}");
        let client = provider_client(&provider_key)?;
        let outcome = client
            .pair(
                &request.cloud_base_url,
                &request.pairing_code,
                &installation_id,
                &self.config.server_name,
            )
            .await?;

        save_secrets(&credential_key, &outcome.secrets)?;
        let save_result = save_connection(
            &self.connection()?,
            NewConnection {
                connection_id: &connection_id,
                provider_key: &provider_key,
                display_name: &display_name,
                cloud_base_url: &outcome.cloud_base_url,
                tenant_id: tenant_id.as_deref(),
                site_id: site_id.as_deref(),
                device_id: &outcome.device_id,
                public_key_base64: &outcome.public_key_base64,
                credential_key: &credential_key,
                scopes: &outcome.scopes,
                make_default: request.make_default.unwrap_or(false),
            },
        );
        if let Err(error) = save_result {
            let _ = delete_secrets(&credential_key);
            return Err(error).context("unable to persist cloud connection");
        }

        let record = connection_record(&self.connection()?, &connection_id)?;
        if let Err(error) = client.status(&record, &outcome.secrets).await {
            mark_connection_error(
                &self.connection()?,
                &connection_id,
                &public_cloud_error(&error),
                authentication_failed(&error),
            )?;
            return Err(error).context("pairing completed but signed cloud verification failed");
        }
        mark_connection_success(&self.connection()?, &connection_id)?;
        self.enqueue_connection_heartbeat(&connection_id)?;
        connection_summary(&self.connection()?, &connection_id)
    }

    fn disconnect_cloud_connection(&self, connection_id: &str) -> Result<CloudConnectionsSnapshot> {
        validate_connection_id(connection_id)?;
        let record = connection_record(&self.connection()?, connection_id)?;
        delete_secrets(&record.credential_key)?;
        let connection = self.connection()?;
        connection.execute(
            "UPDATE cloud_connections SET state='disconnected',is_default=0,last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
            params![connection_id],
        )?;
        ensure_default_connection(&connection)?;
        record_event(
            &connection,
            connection_id,
            "connection.disconnected",
            "success",
            None,
            &json!({}),
        )?;
        registry_snapshot(&connection)
    }

    fn enqueue_connection_sync(&self, request: EnqueueConnectionSyncRequest) -> Result<String> {
        validate_connection_id(&request.connection_id)?;
        let operation_type = request.operation_type.trim().to_lowercase();
        if !ALLOWED_LOCAL_OPERATIONS.contains(&operation_type.as_str()) {
            bail!("synchronization operation is not enabled for HomeServer v1");
        }
        let idempotency_key = request.idempotency_key.unwrap_or_else(|| {
            format!(
                "homeserver:{}:{}",
                request.connection_id,
                Uuid::new_v4().simple()
            )
        });
        validate_idempotency_key(&idempotency_key)?;
        enqueue_operation(
            &self.connection()?,
            &request.connection_id,
            &idempotency_key,
            &operation_type,
            &request.payload,
        )?;
        Ok(idempotency_key)
    }

    fn enqueue_connection_heartbeat(&self, connection_id: &str) -> Result<String> {
        validate_connection_id(connection_id)?;
        let connection = self.connection()?;
        let summary = connection_summary(&connection, connection_id)?;
        if let Some(existing) = connection
            .query_row(
                "SELECT idempotency_key FROM cloud_sync_queue WHERE connection_id=?1 AND operation_type='device.heartbeat' AND state IN ('pending','processing') ORDER BY queue_id LIMIT 1",
                params![connection_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(existing);
        }
        let installation_id = database::installation_id(&connection)?;
        let bucket = Utc::now().timestamp() / 300;
        let key = format!("heartbeat:{connection_id}:{bucket}");
        enqueue_operation(
            &connection,
            connection_id,
            &key,
            "device.heartbeat",
            &json!({
                "installation_id": installation_id,
                "connection_id": connection_id,
                "provider_key": summary.provider_key,
                "tenant_id": summary.tenant_id,
                "site_id": summary.site_id,
                "server_name": &self.config.server_name,
                "version": env!("CARGO_PKG_VERSION"),
            }),
        )?;
        Ok(key)
    }

    async fn sync_cloud_connection(
        &self,
        connection_id: &str,
    ) -> Result<ConnectionSyncRunSnapshot> {
        validate_connection_id(connection_id)?;
        let (record, operations) = {
            let mut connection = self.connection()?;
            let record = connection_record(&connection, connection_id)?;
            if matches!(
                record.summary.state,
                CloudRegistryConnectionState::Revoked | CloudRegistryConnectionState::Disconnected
            ) {
                bail!("cloud connection is inactive; pair it again before synchronizing");
            }
            let operations = claim_due_sync(&mut connection, connection_id, 25)?;
            (record, operations)
        };
        let secrets = match load_secrets(&record.credential_key) {
            Ok(secrets) => secrets,
            Err(error) => {
                mark_connection_error(
                    &self.connection()?,
                    connection_id,
                    "credential_vault_unavailable",
                    false,
                )?;
                return Err(error);
            }
        };
        let client = provider_client(&record.summary.provider_key)?;

        if operations.is_empty() {
            match client.status(&record, &secrets).await {
                Ok(()) => mark_connection_success(&self.connection()?, connection_id)?,
                Err(error) => {
                    mark_connection_error(
                        &self.connection()?,
                        connection_id,
                        &public_cloud_error(&error),
                        authentication_failed(&error),
                    )?;
                    return Err(error);
                }
            }
            return Ok(ConnectionSyncRunSnapshot {
                connection_id: connection_id.to_owned(),
                processed: 0,
                accepted: 0,
                rejected: 0,
                review: 0,
                pending: pending_sync_count(&self.connection()?, connection_id)?,
            });
        }

        let receipts = match client.sync(&record, &secrets, &operations).await {
            Ok(receipts) => receipts,
            Err(error) => {
                let connection = self.connection()?;
                retry_operations(&connection, connection_id, &operations)?;
                mark_connection_error(
                    &connection,
                    connection_id,
                    &public_cloud_error(&error),
                    authentication_failed(&error),
                )?;
                return Err(error);
            }
        };
        if let Err(error) = validate_receipts(&operations, &receipts) {
            let connection = self.connection()?;
            retry_operations(&connection, connection_id, &operations)?;
            mark_connection_error(&connection, connection_id, "invalid_receipt_set", false)?;
            return Err(error);
        }

        let mut accepted = 0;
        let mut rejected = 0;
        let mut review = 0;
        for receipt in &receipts {
            match receipt.disposition.as_str() {
                "accepted" => accepted += 1,
                "rejected" => rejected += 1,
                "review" => review += 1,
                other => bail!("unsupported cloud receipt disposition '{other}'"),
            }
        }
        let mut connection = self.connection()?;
        apply_receipts(&mut connection, connection_id, &receipts)?;
        mark_connection_success(&connection, connection_id)?;
        let pending = pending_sync_count(&connection, connection_id)?;
        Ok(ConnectionSyncRunSnapshot {
            connection_id: connection_id.to_owned(),
            processed: receipts.len() as u64,
            accepted,
            rejected,
            review,
            pending,
        })
    }

    async fn sync_all_cloud_connections(&self) -> Result<AllConnectionsSyncSnapshot> {
        let connection_ids = self.active_cloud_connection_ids()?;
        let mut runs = Vec::with_capacity(connection_ids.len());
        for connection_id in connection_ids {
            self.enqueue_connection_heartbeat(&connection_id)?;
            runs.push(self.sync_cloud_connection(&connection_id).await?);
        }
        Ok(AllConnectionsSyncSnapshot {
            processed: runs.iter().map(|run| run.processed).sum(),
            accepted: runs.iter().map(|run| run.accepted).sum(),
            rejected: runs.iter().map(|run| run.rejected).sum(),
            review: runs.iter().map(|run| run.review).sum(),
            pending: runs.iter().map(|run| run.pending).sum(),
            runs,
        })
    }
}

async fn connections_snapshot(
    State(state): State<Arc<AppState>>,
) -> ApiResult<CloudConnectionsSnapshot> {
    tokio::task::spawn_blocking(move || state.cloud_connections_snapshot())
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("cloud_connections_failed", error))
}

async fn pair_connection(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PairCloudConnectionRequest>,
) -> ApiResult<CloudConnectionSummary> {
    state
        .pair_cloud_connection(request)
        .await
        .map(Json)
        .map_err(|error| action_error("cloud_connection_pairing_failed", error))
}

async fn disconnect_connection(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CloudConnectionRequest>,
) -> ApiResult<CloudConnectionsSnapshot> {
    tokio::task::spawn_blocking(move || state.disconnect_cloud_connection(&request.connection_id))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("cloud_connection_disconnect_failed", error))
}

async fn enqueue_sync(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EnqueueConnectionSyncRequest>,
) -> ApiResult<EnqueueResult> {
    let connection_id = request.connection_id.clone();
    tokio::task::spawn_blocking(move || state.enqueue_connection_sync(request))
        .await
        .map_err(task_error)?
        .map(|idempotency_key| {
            Json(EnqueueResult {
                connection_id,
                idempotency_key,
            })
        })
        .map_err(|error| action_error("cloud_connection_enqueue_failed", error))
}

async fn sync_connection(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CloudConnectionRequest>,
) -> ApiResult<ConnectionSyncRunSnapshot> {
    state
        .sync_cloud_connection(&request.connection_id)
        .await
        .map(Json)
        .map_err(|error| action_error("cloud_connection_sync_failed", error))
}

async fn sync_all_connections(
    State(state): State<Arc<AppState>>,
) -> ApiResult<AllConnectionsSyncSnapshot> {
    state
        .sync_all_cloud_connections()
        .await
        .map(Json)
        .map_err(|error| action_error("cloud_connections_sync_failed", error))
}

fn provider_client(provider_key: &str) -> Result<MicrogifterCloudClient> {
    if provider_key != "microgifter" {
        bail!("cloud provider adapter is not installed");
    }
    MicrogifterCloudClient::new()
}

fn normalize_provider_key(value: &str) -> Result<String> {
    let value = value.trim().to_lowercase();
    if !ALLOWED_PROVIDERS.contains(&value.as_str()) {
        bail!("cloud provider adapter is not installed");
    }
    Ok(value)
}

fn sanitize_optional_text(value: Option<&str>, max: usize, label: &str) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    ensure!(
        value.chars().count() <= max && value.chars().all(|character| !character.is_control()),
        "{label} is invalid"
    );
    Ok(Some(value.to_owned()))
}

fn validate_connection_id(value: &str) -> Result<()> {
    ensure!(
        Uuid::parse_str(value).is_ok(),
        "cloud connection identity is invalid"
    );
    Ok(())
}

struct NewConnection<'a> {
    connection_id: &'a str,
    provider_key: &'a str,
    display_name: &'a str,
    cloud_base_url: &'a str,
    tenant_id: Option<&'a str>,
    site_id: Option<&'a str>,
    device_id: &'a str,
    public_key_base64: &'a str,
    credential_key: &'a str,
    scopes: &'a [String],
    make_default: bool,
}

fn save_connection(connection: &Connection, value: NewConnection<'_>) -> Result<()> {
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM cloud_connections", [], |row| {
        row.get(0)
    })?;
    ensure!(count < MAX_CONNECTIONS, "cloud connection limit reached");
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
        "INSERT INTO cloud_connections (connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,public_key_base64,credential_key,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'connected',?10,?11,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            value.connection_id,
            value.provider_key,
            value.display_name,
            value.cloud_base_url,
            value.tenant_id,
            value.site_id,
            value.device_id,
            value.public_key_base64,
            value.credential_key,
            serde_json::to_string(value.scopes)?,
            i64::from(make_default),
        ],
    )?;
    transaction.commit()?;
    record_event(
        connection,
        value.connection_id,
        "connection.paired",
        "success",
        None,
        &json!({"provider_key": value.provider_key}),
    )?;
    Ok(())
}

fn migrate_legacy_connection(connection: &Connection) -> Result<()> {
    let registry_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM cloud_connections", [], |row| {
            row.get(0)
        })?;
    if registry_count > 0 {
        return Ok(());
    }
    let legacy = connection
        .query_row(
            "SELECT cloud_base_url,device_id,public_key_base64,state,scopes_json,paired_at_utc,last_success_utc,last_error FROM cloud_connection WHERE singleton_id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((base_url, device_id, public_key, state, scopes, paired_at, last_success, last_error)) =
        legacy
    else {
        return Ok(());
    };
    let installation_id = database::installation_id(connection)?;
    let connection_id = Uuid::new_v4().to_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO cloud_connections (connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,public_key_base64,credential_key,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error,created_at_utc,updated_at_utc) VALUES (?1,'microgifter','Microgifter Cloud',?2,NULL,NULL,?3,?4,?5,?6,?7,1,?8,?9,?10,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            connection_id,
            base_url,
            device_id,
            public_key,
            installation_id,
            state,
            scopes,
            paired_at,
            last_success,
            last_error,
        ],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO cloud_sync_queue (connection_id,idempotency_key,operation_type,payload_json,state,attempts,available_at_utc,created_at_utc,updated_at_utc) SELECT ?1,idempotency_key,operation_type,payload_json,state,attempts,available_at_utc,created_at_utc,updated_at_utc FROM sync_queue",
        params![connection_id],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO cloud_sync_receipts (receipt_id,connection_id,idempotency_key,operation_type,disposition,reason_code,response_json,received_at_utc) SELECT receipt_id,?1,idempotency_key,operation_type,disposition,reason_code,response_json,received_at_utc FROM sync_receipts",
        params![connection_id],
    )?;
    transaction.commit()?;
    record_event(
        connection,
        &connection_id,
        "connection.migrated",
        "success",
        None,
        &json!({"source": "legacy_singleton"}),
    )?;
    Ok(())
}

fn registry_snapshot(connection: &Connection) -> Result<CloudConnectionsSnapshot> {
    let mut statement = connection.prepare(
        "SELECT connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error FROM cloud_connections ORDER BY is_default DESC,display_name,connection_id",
    )?;
    let mut connections = statement
        .query_map([], |row| summary_from_row(connection, row))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for summary in &mut connections {
        summary.pending_sync = pending_sync_count(connection, &summary.connection_id)?;
    }
    let active_connections = connections
        .iter()
        .filter(|summary| {
            matches!(
                summary.state,
                CloudRegistryConnectionState::Connected
                    | CloudRegistryConnectionState::Degraded
                    | CloudRegistryConnectionState::Pairing
            )
        })
        .count() as u64;
    let pending_sync = connections.iter().map(|summary| summary.pending_sync).sum();
    let default_connection_id = connections
        .iter()
        .find(|summary| summary.is_default)
        .map(|summary| summary.connection_id.clone());
    Ok(CloudConnectionsSnapshot {
        local_only: active_connections == 0,
        connections,
        active_connections,
        pending_sync,
        default_connection_id,
        supported_providers: ALLOWED_PROVIDERS
            .iter()
            .map(|provider| (*provider).to_owned())
            .collect(),
    })
}

fn summary_from_row(
    connection: &Connection,
    row: &Row<'_>,
) -> rusqlite::Result<CloudConnectionSummary> {
    let connection_id = row.get::<_, String>(0)?;
    let scopes_json = row.get::<_, String>(8)?;
    let pending_sync = pending_sync_count(connection, &connection_id).unwrap_or(0);
    Ok(CloudConnectionSummary {
        connection_id,
        provider_key: row.get(1)?,
        display_name: row.get(2)?,
        cloud_base_url: row.get(3)?,
        tenant_id: row.get(4)?,
        site_id: row.get(5)?,
        device_id: row.get(6)?,
        state: CloudRegistryConnectionState::from_database(&row.get::<_, String>(7)?),
        scopes: serde_json::from_str(&scopes_json).unwrap_or_default(),
        is_default: row.get::<_, i64>(9)? == 1,
        paired_at_utc: row.get(10)?,
        last_success_utc: row.get(11)?,
        last_error: row.get(12)?,
        pending_sync,
    })
}

fn connection_summary(
    connection: &Connection,
    connection_id: &str,
) -> Result<CloudConnectionSummary> {
    connection
        .query_row(
            "SELECT connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error FROM cloud_connections WHERE connection_id=?1",
            params![connection_id],
            |row| summary_from_row(connection, row),
        )
        .optional()?
        .context("cloud connection was not found")
}

fn connection_record(
    connection: &Connection,
    connection_id: &str,
) -> Result<CloudConnectionRecord> {
    let summary = connection_summary(connection, connection_id)?;
    let credential_key: String = connection.query_row(
        "SELECT credential_key FROM cloud_connections WHERE connection_id=?1",
        params![connection_id],
        |row| row.get(0),
    )?;
    Ok(CloudConnectionRecord {
        summary,
        credential_key,
    })
}

fn active_connection_ids(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT connection_id FROM cloud_connections WHERE state IN ('pairing','connected','degraded') ORDER BY is_default DESC,connection_id",
    )?;
    Ok(statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

fn ensure_default_connection(connection: &Connection) -> Result<()> {
    let default_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM cloud_connections WHERE is_default=1 AND state!='disconnected'",
        [],
        |row| row.get(0),
    )?;
    if default_count == 0 {
        connection.execute(
            "UPDATE cloud_connections SET is_default=1 WHERE connection_id=(SELECT connection_id FROM cloud_connections WHERE state!='disconnected' ORDER BY paired_at_utc,connection_id LIMIT 1)",
            [],
        )?;
    }
    Ok(())
}

fn mark_connection_success(connection: &Connection, connection_id: &str) -> Result<()> {
    connection.execute(
        "UPDATE cloud_connections SET state='connected',last_success_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
        params![connection_id],
    )?;
    record_event(
        connection,
        connection_id,
        "connection.sync",
        "success",
        None,
        &json!({}),
    )
}

fn mark_connection_error(
    connection: &Connection,
    connection_id: &str,
    reason: &str,
    revoked: bool,
) -> Result<()> {
    let state = if revoked { "revoked" } else { "degraded" };
    let reason = reason.chars().take(500).collect::<String>();
    connection.execute(
        "UPDATE cloud_connections SET state=?1,last_error=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?3",
        params![state, reason, connection_id],
    )?;
    record_event(
        connection,
        connection_id,
        "connection.sync",
        if revoked { "error" } else { "warning" },
        Some(&reason),
        &json!({}),
    )
}

fn record_event(
    connection: &Connection,
    connection_id: &str,
    event_type: &str,
    outcome: &str,
    detail_code: Option<&str>,
    metadata: &Value,
) -> Result<()> {
    connection.execute(
        "INSERT INTO cloud_connection_events (event_id,connection_id,event_type,outcome,detail_code,metadata_json,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            Uuid::new_v4().to_string(),
            connection_id,
            event_type,
            outcome,
            detail_code,
            serde_json::to_string(metadata)?,
        ],
    )?;
    Ok(())
}

fn enqueue_operation(
    connection: &Connection,
    connection_id: &str,
    idempotency_key: &str,
    operation_type: &str,
    payload: &Value,
) -> Result<i64> {
    let record = connection_summary(connection, connection_id)?;
    ensure!(
        !matches!(record.state, CloudRegistryConnectionState::Disconnected),
        "cloud connection is disconnected"
    );
    let payload_json = serde_json::to_string(payload)?;
    ensure!(
        payload_json.len() <= MAX_SYNC_PAYLOAD_BYTES,
        "synchronization payload exceeds the HomeServer size limit"
    );
    let existing = connection
        .query_row(
            "SELECT queue_id,operation_type,payload_json FROM cloud_sync_queue WHERE connection_id=?1 AND idempotency_key=?2",
            params![connection_id, idempotency_key],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    if let Some((queue_id, existing_type, existing_payload)) = existing {
        if existing_type != operation_type || existing_payload != payload_json {
            bail!("idempotency key is already bound to different synchronization work");
        }
        return Ok(queue_id);
    }
    ensure!(
        pending_sync_count(connection, connection_id)? < MAX_PENDING_SYNC_OPERATIONS_PER_CONNECTION,
        "synchronization queue has reached its safety limit"
    );
    connection.execute(
        "INSERT INTO cloud_sync_queue (connection_id,idempotency_key,operation_type,payload_json,state,attempts,available_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,'pending',0,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![connection_id, idempotency_key, operation_type, payload_json],
    )?;
    Ok(connection.last_insert_rowid())
}

fn pending_sync_count(connection: &Connection, connection_id: &str) -> Result<u64> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM cloud_sync_queue WHERE connection_id=?1 AND state IN ('pending','processing')",
        params![connection_id],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

fn claim_due_sync(
    connection: &mut Connection,
    connection_id: &str,
    limit: usize,
) -> Result<Vec<QueuedOperation>> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "UPDATE cloud_sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1 AND state='processing' AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-5 minutes')",
        params![connection_id],
    )?;
    let mut statement = transaction.prepare(
        "SELECT queue_id,idempotency_key,operation_type,payload_json,attempts FROM cloud_sync_queue WHERE connection_id=?1 AND state='pending' AND available_at_utc<=strftime('%Y-%m-%dT%H:%M:%fZ','now') ORDER BY queue_id LIMIT ?2",
    )?;
    let rows = statement
        .query_map(params![connection_id, limit.max(1) as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    let mut operations = Vec::with_capacity(rows.len());
    for (queue_id, idempotency_key, operation_type, payload_json, attempts) in rows {
        transaction.execute(
            "UPDATE cloud_sync_queue SET state='processing',attempts=attempts+1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE queue_id=?1 AND connection_id=?2 AND state='pending'",
            params![queue_id, connection_id],
        )?;
        operations.push(QueuedOperation {
            queue_id,
            idempotency_key,
            operation_type,
            payload: serde_json::from_str(&payload_json)
                .with_context(|| format!("sync queue payload {queue_id} is invalid"))?,
            attempts: attempts.max(0) as u32 + 1,
        });
    }
    transaction.commit()?;
    Ok(operations)
}

fn apply_receipts(
    connection: &mut Connection,
    connection_id: &str,
    receipts: &[ReceiptRecord],
) -> Result<()> {
    let transaction = connection.transaction()?;
    for receipt in receipts {
        let state = match receipt.disposition.as_str() {
            "accepted" => "accepted",
            "rejected" => "rejected",
            "review" => "review",
            other => bail!("unsupported cloud receipt disposition '{other}'"),
        };
        transaction.execute(
            "UPDATE cloud_sync_queue SET state=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?2 AND idempotency_key=?3",
            params![state, connection_id, receipt.idempotency_key],
        )?;
        transaction.execute(
            "INSERT INTO cloud_sync_receipts (receipt_id,connection_id,idempotency_key,operation_type,disposition,reason_code,response_json,received_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(connection_id,idempotency_key) DO UPDATE SET receipt_id=excluded.receipt_id,operation_type=excluded.operation_type,disposition=excluded.disposition,reason_code=excluded.reason_code,response_json=excluded.response_json,received_at_utc=excluded.received_at_utc",
            params![
                receipt.receipt_id,
                connection_id,
                receipt.idempotency_key,
                receipt.operation_type,
                receipt.disposition,
                receipt.reason_code,
                serde_json::to_string(&receipt.response)?,
            ],
        )?;
    }
    transaction.commit()?;
    maintain_history(connection)
}

fn retry_operations(
    connection: &Connection,
    connection_id: &str,
    operations: &[QueuedOperation],
) -> Result<()> {
    for operation in operations {
        if operation.attempts >= MAX_SYNC_ATTEMPTS {
            connection.execute(
                "UPDATE cloud_sync_queue SET state='rejected',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE queue_id=?1 AND connection_id=?2 AND state='processing'",
                params![operation.queue_id, connection_id],
            )?;
            continue;
        }
        let delay_seconds = (2_u64.pow(operation.attempts.min(8)) * 5).min(1_800);
        let modifier = format!("+{delay_seconds} seconds");
        connection.execute(
            "UPDATE cloud_sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now',?1),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE queue_id=?2 AND connection_id=?3 AND state='processing'",
            params![modifier, operation.queue_id, connection_id],
        )?;
    }
    Ok(())
}

fn maintain_history(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM cloud_sync_receipts WHERE received_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-90 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM cloud_sync_queue WHERE state IN ('accepted','rejected','review') AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-90 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM cloud_connection_events WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-90 days')",
        [],
    )?;
    Ok(())
}

fn credential_entry(credential_key: &str) -> Result<Entry> {
    Entry::new(CREDENTIAL_SERVICE, credential_key)
        .context("unable to open the cloud connection credential vault")
}

fn save_secrets(credential_key: &str, secrets: &DeviceSecrets) -> Result<()> {
    let payload = serde_json::to_string(&StoredDeviceSecrets {
        device_token: secrets.device_token.clone(),
        signing_key_base64: secrets.signing_key_base64.clone(),
    })?;
    credential_entry(credential_key)?
        .set_password(&payload)
        .context("unable to save cloud connection credentials")
}

fn load_secrets(credential_key: &str) -> Result<DeviceSecrets> {
    let payload = credential_entry(credential_key)?
        .get_password()
        .context("cloud connection credentials are unavailable")?;
    let stored: StoredDeviceSecrets =
        serde_json::from_str(&payload).context("cloud connection credentials are invalid")?;
    Ok(DeviceSecrets {
        device_token: stored.device_token,
        signing_key_base64: stored.signing_key_base64,
    })
}

fn delete_secrets(credential_key: &str) -> Result<()> {
    match credential_entry(credential_key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to delete cloud connection credentials"),
    }
}

fn validate_idempotency_key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 190
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.:-".contains(character))
    {
        bail!("idempotency key is invalid");
    }
    Ok(())
}

fn validate_receipts(operations: &[QueuedOperation], receipts: &[ReceiptRecord]) -> Result<()> {
    if operations.len() != receipts.len() {
        bail!("cloud provider returned an incomplete synchronization receipt set");
    }
    for operation in operations {
        let matching = receipts
            .iter()
            .filter(|receipt| {
                receipt.idempotency_key == operation.idempotency_key
                    && receipt.operation_type == operation.operation_type
            })
            .count();
        if matching != 1 {
            bail!("cloud provider returned an invalid synchronization receipt set");
        }
    }
    for receipt in receipts {
        ensure!(
            matches!(
                receipt.disposition.as_str(),
                "accepted" | "rejected" | "review"
            ),
            "cloud provider returned an unsupported synchronization disposition"
        );
        validate_idempotency_key(&receipt.idempotency_key)?;
        ensure!(
            receipt.receipt_id.len() <= 190
                && receipt.receipt_id.chars().all(
                    |character| character.is_ascii_alphanumeric() || "_.:-".contains(character)
                ),
            "cloud provider returned an invalid synchronization receipt identity"
        );
        ensure!(
            receipt
                .reason_code
                .as_deref()
                .is_none_or(|value| value.len() <= 120),
            "cloud provider returned an oversized synchronization reason"
        );
        ensure!(
            serde_json::to_vec(&receipt.response)?.len() <= MAX_SYNC_PAYLOAD_BYTES,
            "cloud provider returned an oversized synchronization receipt"
        );
    }
    Ok(())
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

async fn decode_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_CLOUD_RESPONSE_BYTES as u64,
            "cloud provider response exceeds the HomeServer size limit"
        );
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("unable to read cloud provider response")?;
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .context("cloud provider response size overflow")?;
        ensure!(
            next_len <= MAX_CLOUD_RESPONSE_BYTES,
            "cloud provider response exceeds the HomeServer size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let envelope: ApiEnvelope<T> = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "cloud provider returned an invalid response with HTTP {}",
            status.as_u16()
        )
    })?;
    if !status.is_success() || !envelope.ok {
        bail!(
            "cloud provider request failed with HTTP {}: {}",
            status.as_u16(),
            envelope.message.chars().take(500).collect::<String>()
        );
    }
    envelope
        .data
        .context("cloud provider response did not contain data")
}

fn normalize_cloud_base_url(value: &str) -> Result<String> {
    let url = Url::parse(value.trim()).context("cloud URL is invalid")?;
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "cloud URL cannot contain credentials"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "cloud URL cannot contain a query or fragment"
    );
    let host = url.host_str().context("cloud URL host is required")?;
    let loopback = host
        .parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or_else(|_| host.eq_ignore_ascii_case("localhost"));
    ensure!(
        url.scheme() == "https" || (loopback && url.scheme() == "http"),
        "cloud URL must use HTTPS"
    );
    ensure!(
        url.path() == "/" || url.path().is_empty(),
        "cloud URL cannot contain a path"
    );
    let mut normalized = url;
    normalized.set_path("");
    Ok(normalized.as_str().trim_end_matches('/').to_owned())
}

fn authentication_failed(error: &anyhow::Error) -> bool {
    let text = error.to_string().to_lowercase();
    text.contains("http 401") || text.contains("http 403") || text.contains("revoked")
}

fn public_cloud_error(error: &anyhow::Error) -> String {
    let text = error.to_string().to_lowercase();
    if authentication_failed(error) {
        "cloud_credentials_rejected".to_owned()
    } else if text.contains("timed out") || text.contains("timeout") {
        "cloud_timeout".to_owned()
    } else if text.contains("dns") || text.contains("connect") || text.contains("reach") {
        "cloud_unreachable".to_owned()
    } else if text.contains("invalid response") || text.contains("receipt") {
        "cloud_response_invalid".to_owned()
    } else {
        "cloud_request_failed".to_owned()
    }
}

fn task_error(error: tokio::task::JoinError) -> (HttpStatusCode, Json<ApiError>) {
    internal_error("cloud_connection_task_failed", error.into())
}

fn action_error(code: &'static str, error: anyhow::Error) -> (HttpStatusCode, Json<ApiError>) {
    let text = error.to_string().to_lowercase();
    let status = if text.contains("pairing code")
        || text.contains("cloud url")
        || text.contains("idempotency")
        || text.contains("not enabled")
        || text.contains("not installed")
        || text.contains("invalid")
    {
        HttpStatusCode::UNPROCESSABLE_ENTITY
    } else if text.contains("not found") {
        HttpStatusCode::NOT_FOUND
    } else if text.contains("inactive") || text.contains("revoked") || text.contains("disconnected")
    {
        HttpStatusCode::CONFLICT
    } else if text.contains("cloud")
        || text.contains("microgifter")
        || text.contains("pairing service")
    {
        HttpStatusCode::BAD_GATEWAY
    } else {
        HttpStatusCode::INTERNAL_SERVER_ERROR
    };
    api_error(status, code, error)
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (HttpStatusCode, Json<ApiError>) {
    api_error(HttpStatusCode::INTERNAL_SERVER_ERROR, code, error)
}

fn api_error(
    status: HttpStatusCode,
    code: &'static str,
    error: anyhow::Error,
) -> (HttpStatusCode, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string().chars().take(500).collect(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn insert_connection(connection: &Connection, connection_id: &str) {
        save_connection(
            connection,
            NewConnection {
                connection_id,
                provider_key: "microgifter",
                display_name: "Test Site",
                cloud_base_url: "https://microgifter.com",
                tenant_id: Some("tenant-a"),
                site_id: Some("site-a"),
                device_id: "2c1aa5b0-00a4-4d06-b773-4e27ae331d6f",
                public_key_base64: "public-key",
                credential_key: "test-credential",
                scopes: &[
                    "homeserver.status".to_owned(),
                    "homeserver.sync.write".to_owned(),
                ],
                make_default: true,
            },
        )
        .unwrap();
    }

    #[test]
    fn multi_connection_queue_is_namespaced() {
        let temp = tempdir().unwrap();
        let connection = database::initialize(&temp.path().join("registry.sqlite3")).unwrap();
        initialize(&connection).unwrap();
        let first = Uuid::new_v4().to_string();
        let second = Uuid::new_v4().to_string();
        insert_connection(&connection, &first);
        save_connection(
            &connection,
            NewConnection {
                connection_id: &second,
                provider_key: "microgifter",
                display_name: "Second Site",
                cloud_base_url: "https://microgifter.com",
                tenant_id: None,
                site_id: Some("site-b"),
                device_id: "ff23256e-ee4b-42bf-a201-0bef291286e7",
                public_key_base64: "public-key-two",
                credential_key: "test-credential-two",
                scopes: &["homeserver.sync.write".to_owned()],
                make_default: false,
            },
        )
        .unwrap();
        enqueue_operation(
            &connection,
            &first,
            "same-key",
            "device.heartbeat",
            &json!({"site": "a"}),
        )
        .unwrap();
        enqueue_operation(
            &connection,
            &second,
            "same-key",
            "device.heartbeat",
            &json!({"site": "b"}),
        )
        .unwrap();
        assert_eq!(pending_sync_count(&connection, &first).unwrap(), 1);
        assert_eq!(pending_sync_count(&connection, &second).unwrap(), 1);
        let snapshot = registry_snapshot(&connection).unwrap();
        assert_eq!(snapshot.connections.len(), 2);
        assert_eq!(snapshot.pending_sync, 2);
    }

    #[test]
    fn unsupported_provider_and_commerce_are_rejected() {
        assert!(normalize_provider_key("other-crm").is_err());
        assert!(!ALLOWED_LOCAL_OPERATIONS.contains(&"commerce.order.create"));
    }

    #[test]
    fn cloud_urls_require_https_except_loopback() {
        assert_eq!(
            normalize_cloud_base_url("https://microgifter.com/").unwrap(),
            "https://microgifter.com"
        );
        assert!(normalize_cloud_base_url("http://microgifter.com").is_err());
        assert_eq!(
            normalize_cloud_base_url("http://127.0.0.1:49001/").unwrap(),
            "http://127.0.0.1:49001"
        );
    }
}
