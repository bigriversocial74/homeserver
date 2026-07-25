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
use keyring::Entry;
use rand::rngs::OsRng;
use reqwest::{Method, StatusCode as CloudStatusCode};
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

const CLOUD_MIGRATION: &str =
    include_str!("../../../database/migrations/0004_cloud_pairing_sync.sql");
const CLOUD_MIGRATION_KEY: &str = "0004_cloud_pairing_sync";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";
const PAIR_PATH: &str = "/api/homeserver/pair.php";
const STATUS_PATH: &str = "/api/homeserver/status.php";
const SYNC_PATH: &str = "/api/homeserver/sync.php";
const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;
const SYNC_INTERVAL: Duration = Duration::from_secs(60);
const ALLOWED_LOCAL_OPERATIONS: &[&str] = &[
    "device.heartbeat",
    "local.settings.snapshot",
    "cache.refresh.request",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudConnectionState {
    NotPaired,
    Pairing,
    Connected,
    Degraded,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudConnectionSnapshot {
    pub state: CloudConnectionState,
    pub cloud_base_url: Option<String>,
    pub device_id: Option<String>,
    pub scopes: Vec<String>,
    pub paired_at_utc: Option<String>,
    pub last_success_utc: Option<String>,
    pub last_error: Option<String>,
    pub pending_sync: u64,
}

impl Default for CloudConnectionSnapshot {
    fn default() -> Self {
        Self {
            state: CloudConnectionState::NotPaired,
            cloud_base_url: None,
            device_id: None,
            scopes: Vec::new(),
            paired_at_utc: None,
            last_success_utc: None,
            last_error: None,
            pending_sync: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairCloudRequest {
    pub cloud_base_url: String,
    pub pairing_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnqueueSyncRequest {
    pub operation_type: String,
    #[serde(default)]
    pub payload: Value,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncRunSnapshot {
    pub processed: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub review: u64,
    pub pending: u64,
}

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct ActionMessage {
    ok: bool,
    message: String,
}

#[derive(Debug, Serialize)]
struct EnqueueResult {
    idempotency_key: String,
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
struct QueuedOperation {
    queue_id: i64,
    idempotency_key: String,
    operation_type: String,
    payload: Value,
    attempts: u32,
}

#[derive(Debug, Clone)]
struct CloudConnectionRecord {
    snapshot: CloudConnectionSnapshot,
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
struct CloudClient {
    client: reqwest::Client,
}

impl CloudClient {
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
        let base_url = record
            .snapshot
            .cloud_base_url
            .as_deref()
            .context("HomeServer is not paired")?;
        let _: Value = self
            .signed_request(Method::GET, base_url, STATUS_PATH, "", record, secrets)
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
        let base_url = record
            .snapshot
            .cloud_base_url
            .as_deref()
            .context("HomeServer is not paired")?;
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
            .signed_request(Method::POST, base_url, SYNC_PATH, &body, record, secrets)
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
        base_url: &str,
        path: &str,
        body: &str,
        record: &CloudConnectionRecord,
        secrets: &DeviceSecrets,
    ) -> Result<T> {
        let device_id = record
            .snapshot
            .device_id
            .as_deref()
            .context("HomeServer device identity is unavailable")?;
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
            .request(method, format!("{base_url}{path}"))
            .bearer_auth(&secrets.device_token)
            .header("X-MG-Homeserver-ID", device_id)
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
    connection.execute_batch(CLOUD_MIGRATION)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![CLOUD_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        count == 1,
        "migration '{CLOUD_MIGRATION_KEY}' is not registered exactly once"
    );
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/cloud", get(cloud_status))
        .route("/v1/cloud/pair", post(pair_cloud))
        .route("/v1/cloud/disconnect", post(disconnect_cloud))
        .route("/v1/cloud/vault-self-test", post(vault_self_test_handler))
        .route("/v1/cloud/enqueue", post(enqueue_sync))
        .route("/v1/cloud/sync", post(sync_once))
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
                    info!("HomeServer synchronization worker stopped");
                    return;
                }
            }
            _ = interval.tick() => {
                let connection = match state.cloud_snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        warn!(?error, "unable to inspect HomeServer cloud state");
                        continue;
                    }
                };
                if matches!(connection.state, CloudConnectionState::NotPaired | CloudConnectionState::Revoked) {
                    continue;
                }
                if let Err(error) = state.enqueue_heartbeat() {
                    warn!(?error, "unable to queue HomeServer heartbeat");
                }
                if let Err(error) = state.sync_once().await {
                    warn!(?error, "HomeServer synchronization attempt failed");
                }
            }
        }
    }
}

impl AppState {
    fn cloud_snapshot(&self) -> Result<CloudConnectionSnapshot> {
        cloud_connection(&*self.connection()?)
    }

    async fn pair_cloud(&self, request: PairCloudRequest) -> Result<CloudConnectionSnapshot> {
        let installation_id = database::installation_id(&*self.connection()?)?;
        let client = CloudClient::new()?;
        let outcome = client
            .pair(
                &request.cloud_base_url,
                &request.pairing_code,
                &installation_id,
                &self.config.server_name,
            )
            .await?;

        save_secrets(&installation_id, &outcome.secrets)?;
        if let Err(error) = save_cloud_connection(
            &*self.connection()?,
            &outcome.cloud_base_url,
            &outcome.device_id,
            &outcome.public_key_base64,
            &outcome.scopes,
        ) {
            let _ = delete_secrets(&installation_id);
            return Err(error).context("unable to persist HomeServer cloud pairing state");
        }

        let record = cloud_connection_record(&*self.connection()?)?;
        if let Err(error) = client.status(&record, &outcome.secrets).await {
            mark_cloud_error(
                &*self.connection()?,
                &public_cloud_error(&error),
                authentication_failed(&error),
            )?;
            return Err(error).context("pairing completed but signed cloud verification failed");
        }
        mark_cloud_success(&*self.connection()?)?;
        self.enqueue_heartbeat()?;
        self.cloud_snapshot()
    }

    fn disconnect_cloud(&self) -> Result<CloudConnectionSnapshot> {
        let installation_id = database::installation_id(&*self.connection()?)?;
        delete_secrets(&installation_id)?;
        clear_cloud_connection(&*self.connection()?)?;
        Ok(CloudConnectionSnapshot::default())
    }

    fn credential_vault_self_test(&self) -> Result<()> {
        let installation_id = database::installation_id(&*self.connection()?)?;
        credential_vault_self_test_entry(&installation_id)
    }

    fn enqueue_sync(&self, request: EnqueueSyncRequest) -> Result<String> {
        let operation_type = request.operation_type.trim().to_lowercase();
        if !ALLOWED_LOCAL_OPERATIONS.contains(&operation_type.as_str()) {
            bail!("synchronization operation is not enabled for HomeServer v1");
        }
        let idempotency_key = request
            .idempotency_key
            .unwrap_or_else(|| format!("homeserver:{}", Uuid::new_v4().simple()));
        validate_idempotency_key(&idempotency_key)?;
        enqueue_operation(
            &*self.connection()?,
            &idempotency_key,
            &operation_type,
            &request.payload,
        )?;
        Ok(idempotency_key)
    }

    fn enqueue_heartbeat(&self) -> Result<String> {
        let connection = self.connection()?;
        let installation_id = database::installation_id(&connection)?;
        let bucket = Utc::now().timestamp() / 300;
        let key = format!("heartbeat:{installation_id}:{bucket}");
        enqueue_operation(
            &connection,
            &key,
            "device.heartbeat",
            &json!({
                "installation_id": installation_id,
                "server_name": &self.config.server_name,
                "version": env!("CARGO_PKG_VERSION"),
            }),
        )?;
        Ok(key)
    }

    async fn sync_once(&self) -> Result<SyncRunSnapshot> {
        let (record, installation_id, operations) = {
            let mut connection = self.connection()?;
            let record = cloud_connection_record(&connection)?;
            match record.snapshot.state {
                CloudConnectionState::NotPaired => {
                    let pending = cloud_pending_sync_count(&connection)?;
                    return Ok(SyncRunSnapshot {
                        processed: 0,
                        accepted: 0,
                        rejected: 0,
                        review: 0,
                        pending,
                    });
                }
                CloudConnectionState::Revoked => {
                    bail!("HomeServer cloud credentials were revoked; pair the device again");
                }
                _ => {}
            }
            let installation_id = database::installation_id(&connection)?;
            let operations = claim_due_sync(&mut connection, 25)?;
            (record, installation_id, operations)
        };

        let secrets = match load_secrets(&installation_id) {
            Ok(secrets) => secrets,
            Err(error) => {
                mark_cloud_error(&*self.connection()?, "credential_vault_unavailable", false)?;
                return Err(error);
            }
        };
        let client = CloudClient::new()?;

        if operations.is_empty() {
            match client.status(&record, &secrets).await {
                Ok(()) => mark_cloud_success(&*self.connection()?)?,
                Err(error) => {
                    mark_cloud_error(
                        &*self.connection()?,
                        &public_cloud_error(&error),
                        authentication_failed(&error),
                    )?;
                    return Err(error);
                }
            }
            return Ok(SyncRunSnapshot {
                processed: 0,
                accepted: 0,
                rejected: 0,
                review: 0,
                pending: cloud_pending_sync_count(&*self.connection()?)?,
            });
        }

        let receipts = match client.sync(&record, &secrets, &operations).await {
            Ok(receipts) => receipts,
            Err(error) => {
                let connection = self.connection()?;
                retry_operations(&connection, &operations)?;
                mark_cloud_error(
                    &connection,
                    &public_cloud_error(&error),
                    authentication_failed(&error),
                )?;
                return Err(error);
            }
        };
        if let Err(error) = validate_receipts(&operations, &receipts) {
            let connection = self.connection()?;
            retry_operations(&connection, &operations)?;
            mark_cloud_error(&connection, "invalid_receipt_set", false)?;
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
                _ => unreachable!("receipt validation rejects unknown dispositions"),
            }
        }
        let mut connection = self.connection()?;
        apply_receipts(&mut connection, &receipts)?;
        mark_cloud_success(&connection)?;
        let pending = cloud_pending_sync_count(&connection)?;

        Ok(SyncRunSnapshot {
            processed: receipts.len() as u64,
            accepted,
            rejected,
            review,
            pending,
        })
    }
}

async fn cloud_status(State(state): State<Arc<AppState>>) -> ApiResult<CloudConnectionSnapshot> {
    tokio::task::spawn_blocking(move || state.cloud_snapshot())
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("cloud_status_failed", error))
}

async fn pair_cloud(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PairCloudRequest>,
) -> ApiResult<CloudConnectionSnapshot> {
    state
        .pair_cloud(request)
        .await
        .map(Json)
        .map_err(|error| action_error("cloud_pairing_failed", error))
}

async fn disconnect_cloud(
    State(state): State<Arc<AppState>>,
) -> ApiResult<CloudConnectionSnapshot> {
    tokio::task::spawn_blocking(move || state.disconnect_cloud())
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("cloud_disconnect_failed", error))
}

async fn vault_self_test_handler(State(state): State<Arc<AppState>>) -> ApiResult<ActionMessage> {
    tokio::task::spawn_blocking(move || state.credential_vault_self_test())
        .await
        .map_err(task_error)?
        .map(|()| {
            Json(ActionMessage {
                ok: true,
                message: "Operating-system credential vault passed its write/read/delete test."
                    .to_owned(),
            })
        })
        .map_err(|error| action_error("credential_vault_test_failed", error))
}

async fn enqueue_sync(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EnqueueSyncRequest>,
) -> ApiResult<EnqueueResult> {
    tokio::task::spawn_blocking(move || state.enqueue_sync(request))
        .await
        .map_err(task_error)?
        .map(|idempotency_key| Json(EnqueueResult { idempotency_key }))
        .map_err(|error| action_error("cloud_enqueue_failed", error))
}

async fn sync_once(State(state): State<Arc<AppState>>) -> ApiResult<SyncRunSnapshot> {
    state
        .sync_once()
        .await
        .map(Json)
        .map_err(|error| action_error("cloud_sync_failed", error))
}

fn entry(installation_id: &str) -> Result<Entry> {
    Entry::new(CREDENTIAL_SERVICE, installation_id)
        .context("unable to open the HomeServer operating-system credential vault")
}

fn save_secrets(installation_id: &str, secrets: &DeviceSecrets) -> Result<()> {
    let payload = serde_json::to_string(&StoredDeviceSecrets {
        device_token: secrets.device_token.clone(),
        signing_key_base64: secrets.signing_key_base64.clone(),
    })?;
    entry(installation_id)?
        .set_password(&payload)
        .context("unable to save HomeServer cloud credentials")
}

fn load_secrets(installation_id: &str) -> Result<DeviceSecrets> {
    let payload = entry(installation_id)?
        .get_password()
        .context("HomeServer cloud credentials are unavailable")?;
    let stored: StoredDeviceSecrets =
        serde_json::from_str(&payload).context("HomeServer cloud credentials are invalid")?;
    Ok(DeviceSecrets {
        device_token: stored.device_token,
        signing_key_base64: stored.signing_key_base64,
    })
}

fn delete_secrets(installation_id: &str) -> Result<()> {
    match entry(installation_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to delete HomeServer cloud credentials"),
    }
}

fn credential_vault_self_test_entry(installation_id: &str) -> Result<()> {
    let diagnostic_user = format!("{installation_id}:diagnostic:{}", Uuid::new_v4().simple());
    let diagnostic_entry = Entry::new(CREDENTIAL_SERVICE, &diagnostic_user)
        .context("unable to open a diagnostic operating-system credential entry")?;
    let mut secret = format!("homeserver-vault-test:{}", Uuid::new_v4().simple());

    let result = diagnostic_entry
        .set_password(&secret)
        .context("unable to write a diagnostic operating-system credential")
        .and_then(|()| {
            let stored = diagnostic_entry
                .get_password()
                .context("unable to read the diagnostic operating-system credential")?;
            if stored != secret {
                bail!("operating-system credential vault returned mismatched data");
            }
            Ok(())
        });
    let delete_result = match diagnostic_entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to delete the diagnostic credential"),
    };
    secret.zeroize();
    result.and(delete_result)
}

fn cloud_connection(connection: &Connection) -> Result<CloudConnectionSnapshot> {
    Ok(cloud_connection_record(connection)?.snapshot)
}

fn cloud_connection_record(connection: &Connection) -> Result<CloudConnectionRecord> {
    let row = connection
        .query_row(
            "SELECT cloud_base_url,device_id,state,scopes_json,paired_at_utc,last_success_utc,last_error FROM cloud_connection WHERE singleton_id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .optional()?;

    let pending_sync = cloud_pending_sync_count(connection)?;
    let Some((base_url, device_id, state, scopes_json, paired_at, last_success, last_error)) = row
    else {
        return Ok(CloudConnectionRecord {
            snapshot: CloudConnectionSnapshot {
                pending_sync,
                ..CloudConnectionSnapshot::default()
            },
        });
    };
    let state = match state.as_str() {
        "pairing" => CloudConnectionState::Pairing,
        "connected" => CloudConnectionState::Connected,
        "degraded" => CloudConnectionState::Degraded,
        "revoked" => CloudConnectionState::Revoked,
        _ => CloudConnectionState::Degraded,
    };
    let scopes = serde_json::from_str::<Vec<String>>(&scopes_json).unwrap_or_default();
    Ok(CloudConnectionRecord {
        snapshot: CloudConnectionSnapshot {
            state,
            cloud_base_url: Some(base_url),
            device_id: Some(device_id),
            scopes,
            paired_at_utc: Some(paired_at),
            last_success_utc: last_success,
            last_error,
            pending_sync,
        },
    })
}

fn save_cloud_connection(
    connection: &Connection,
    cloud_base_url: &str,
    device_id: &str,
    public_key_base64: &str,
    scopes: &[String],
) -> Result<()> {
    connection.execute(
        "INSERT INTO cloud_connection (singleton_id,cloud_base_url,device_id,public_key_base64,state,scopes_json,paired_at_utc,last_success_utc,last_error,updated_at_utc)
         VALUES (1,?1,?2,?3,'connected',?4,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(singleton_id) DO UPDATE SET cloud_base_url=excluded.cloud_base_url,device_id=excluded.device_id,public_key_base64=excluded.public_key_base64,state='connected',scopes_json=excluded.scopes_json,paired_at_utc=excluded.paired_at_utc,last_success_utc=excluded.last_success_utc,last_error=NULL,updated_at_utc=excluded.updated_at_utc",
        params![cloud_base_url, device_id, public_key_base64, serde_json::to_string(scopes)?],
    )?;
    Ok(())
}

fn mark_cloud_success(connection: &Connection) -> Result<()> {
    connection.execute(
        "UPDATE cloud_connection SET state='connected',last_success_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        [],
    )?;
    Ok(())
}

fn mark_cloud_error(connection: &Connection, reason: &str, revoked: bool) -> Result<()> {
    let state = if revoked { "revoked" } else { "degraded" };
    connection.execute(
        "UPDATE cloud_connection SET state=?1,last_error=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![state, reason.chars().take(500).collect::<String>()],
    )?;
    Ok(())
}

fn clear_cloud_connection(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM cloud_connection WHERE singleton_id=1", [])?;
    connection.execute(
        "UPDATE sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='processing'",
        [],
    )?;
    Ok(())
}

fn enqueue_operation(
    connection: &Connection,
    idempotency_key: &str,
    operation_type: &str,
    payload: &Value,
) -> Result<i64> {
    let payload_json = serde_json::to_string(payload)?;
    let existing = connection
        .query_row(
            "SELECT queue_id,operation_type,payload_json FROM sync_queue WHERE idempotency_key=?1",
            params![idempotency_key],
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
    connection.execute(
        "INSERT INTO sync_queue (idempotency_key,operation_type,payload_json,state,attempts,available_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,'pending',0,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![idempotency_key, operation_type, payload_json],
    )?;
    Ok(connection.last_insert_rowid())
}

fn cloud_pending_sync_count(connection: &Connection) -> Result<u64> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sync_queue WHERE state IN ('pending','processing')",
        [],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

fn claim_due_sync(connection: &mut Connection, limit: usize) -> Result<Vec<QueuedOperation>> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "UPDATE sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='processing' AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-5 minutes')",
        [],
    )?;
    let mut statement = transaction.prepare(
        "SELECT queue_id,idempotency_key,operation_type,payload_json,attempts FROM sync_queue WHERE state='pending' AND available_at_utc<=strftime('%Y-%m-%dT%H:%M:%fZ','now') ORDER BY queue_id LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![limit.max(1) as i64], |row| {
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
            "UPDATE sync_queue SET state='processing',attempts=attempts+1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE queue_id=?1 AND state='pending'",
            params![queue_id],
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

fn apply_receipts(connection: &mut Connection, receipts: &[ReceiptRecord]) -> Result<()> {
    let transaction = connection.transaction()?;
    for receipt in receipts {
        let state = match receipt.disposition.as_str() {
            "accepted" => "accepted",
            "rejected" => "rejected",
            "review" => "review",
            other => bail!("unsupported cloud receipt disposition '{other}'"),
        };
        transaction.execute(
            "UPDATE sync_queue SET state=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE idempotency_key=?2",
            params![state, receipt.idempotency_key],
        )?;
        transaction.execute(
            "INSERT INTO sync_receipts (receipt_id,idempotency_key,operation_type,disposition,reason_code,response_json,received_at_utc) VALUES (?1,?2,?3,?4,?5,?6,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(idempotency_key) DO UPDATE SET receipt_id=excluded.receipt_id,operation_type=excluded.operation_type,disposition=excluded.disposition,reason_code=excluded.reason_code,response_json=excluded.response_json,received_at_utc=excluded.received_at_utc",
            params![receipt.receipt_id, receipt.idempotency_key, receipt.operation_type, receipt.disposition, receipt.reason_code, serde_json::to_string(&receipt.response)?],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn retry_operations(connection: &Connection, operations: &[QueuedOperation]) -> Result<()> {
    for operation in operations {
        let delay_seconds = (2_u64.pow(operation.attempts.min(8)) * 5).min(1_800);
        let modifier = format!("+{delay_seconds} seconds");
        connection.execute(
            "UPDATE sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now',?1),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE queue_id=?2 AND state='processing'",
            params![modifier, operation.queue_id],
        )?;
    }
    Ok(())
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
        bail!("Microgifter returned an incomplete synchronization receipt set");
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
            bail!("Microgifter returned an invalid synchronization receipt set");
        }
    }
    for receipt in receipts {
        if !matches!(
            receipt.disposition.as_str(),
            "accepted" | "rejected" | "review"
        ) {
            bail!("Microgifter returned an unsupported synchronization disposition");
        }
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
    let text = response
        .text()
        .await
        .context("unable to read Microgifter response")?;
    let envelope: ApiEnvelope<T> = serde_json::from_str(&text).with_context(|| {
        format!(
            "Microgifter returned an invalid response ({})",
            status.as_u16()
        )
    })?;
    if !status.is_success() || !envelope.ok {
        let prefix = if matches!(
            status,
            CloudStatusCode::UNAUTHORIZED | CloudStatusCode::FORBIDDEN
        ) {
            "cloud_authentication_failed"
        } else {
            "cloud_request_rejected"
        };
        bail!("{prefix}: {}", envelope.message);
    }
    envelope
        .data
        .ok_or_else(|| anyhow!("Microgifter response did not include data"))
}

fn normalize_cloud_base_url(value: &str) -> Result<String> {
    let mut url = Url::parse(value.trim()).context("cloud URL is invalid")?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("cloud URL cannot include credentials, a query, or a fragment");
    }
    let host = url.host_str().context("cloud URL host is required")?;
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => bail!("cloud URL must use HTTPS unless it is a loopback test server"),
    }
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn authentication_failed(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .starts_with("cloud_authentication_failed:")
}

fn public_cloud_error(error: &anyhow::Error) -> String {
    let text = error.to_string();
    if text.starts_with("cloud_authentication_failed:") {
        "authentication_failed".to_owned()
    } else if text.starts_with("cloud_request_rejected:") {
        "request_rejected".to_owned()
    } else {
        "cloud_unavailable".to_owned()
    }
}

fn task_error(error: tokio::task::JoinError) -> (HttpStatusCode, Json<ApiError>) {
    api_error(
        HttpStatusCode::INTERNAL_SERVER_ERROR,
        "homeserver_task_failed",
        anyhow!(error),
    )
}

fn action_error(code: &'static str, error: anyhow::Error) -> (HttpStatusCode, Json<ApiError>) {
    let text = error.to_string().to_lowercase();
    let status = if text.contains("pairing code")
        || text.contains("cloud url")
        || text.contains("idempotency")
        || text.contains("not enabled")
        || text.contains("invalid")
    {
        HttpStatusCode::UNPROCESSABLE_ENTITY
    } else if text.contains("revoked") {
        HttpStatusCode::CONFLICT
    } else if text.contains("cloud_")
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
    use ed25519_dalek::Verifier;
    use tempfile::tempdir;

    #[test]
    fn cloud_url_requires_https_except_loopback() {
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

    #[test]
    fn canonical_signature_is_verifiable() {
        let key = SigningKey::generate(&mut OsRng);
        let canonical = canonical_request(
            &Method::POST,
            SYNC_PATH,
            "100",
            "nonce-value-1234",
            "{\"x\":1}",
        );
        let signature = key.sign(canonical.as_bytes());
        key.verifying_key()
            .verify(canonical.as_bytes(), &signature)
            .unwrap();
        assert!(
            canonical.ends_with("5041bf1f713df204784353e82f6a4a535931cb64f1f4b4a5aeaffcb720918b22")
        );
    }

    #[test]
    fn cloud_migration_and_queue_contract_are_durable() {
        let temp = tempdir().unwrap();
        let database_path = temp.path().join("homeserver.db");
        let connection = database::initialize(&database_path).unwrap();
        initialize(&connection).unwrap();
        let id = enqueue_operation(
            &connection,
            "local.settings:test",
            "local.settings.snapshot",
            &json!({"enabled": true}),
        )
        .unwrap();
        assert!(id > 0);
        assert_eq!(cloud_pending_sync_count(&connection).unwrap(), 1);
        assert_eq!(
            cloud_connection(&connection).unwrap().state,
            CloudConnectionState::NotPaired
        );
    }

    #[test]
    fn commerce_work_is_not_locally_enabled() {
        assert!(ALLOWED_LOCAL_OPERATIONS.contains(&"device.heartbeat"));
        assert!(!ALLOWED_LOCAL_OPERATIONS.contains(&"commerce.order.create"));
    }
}
