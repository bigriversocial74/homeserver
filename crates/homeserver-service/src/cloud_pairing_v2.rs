use crate::{database, AppState};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::StreamExt;
use keyring::Entry;
use rand::rngs::OsRng;
use reqwest::Method;
use rusqlite::{params, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{net::IpAddr, sync::Arc, time::Duration};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";
const PAIR_PATH: &str = "/api/homeserver/pair.php";
const STATUS_PATH: &str = "/api/homeserver/status.php";
const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;
const MAX_CLOUD_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CONNECTIONS: i64 = 50;
const MAX_SYNC_PAYLOAD_BYTES: usize = 48 * 1024;
const LEGACY_PAIR_PATH: &str = "/v1/cloud/connections/pair";

#[derive(Debug, Deserialize)]
pub struct PairCloudConnectionV2Request {
    provider_key: String,
    display_name: String,
    cloud_base_url: String,
    pairing_code: String,
    tenant_id: Option<String>,
    site_id: Option<String>,
    make_default: Option<bool>,
}

#[derive(Debug, Serialize)]
struct CloudConnectionV2Summary {
    connection_id: String,
    provider_key: String,
    display_name: String,
    cloud_base_url: String,
    tenant_id: Option<String>,
    site_id: Option<String>,
    device_id: String,
    state: String,
    scopes: Vec<String>,
    is_default: bool,
    paired_at_utc: String,
    last_success_utc: Option<String>,
    last_error: Option<String>,
    pending_sync: u64,
}

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

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

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    ok: bool,
    message: String,
    data: Option<T>,
}

#[derive(Debug)]
struct PairingOutcome {
    cloud_base_url: String,
    device_id: String,
    public_key_base64: String,
    scopes: Vec<String>,
    secrets: DeviceSecrets,
}

#[derive(Clone)]
struct MicrogifterPairingClient {
    client: reqwest::Client,
}

impl MicrogifterPairingClient {
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
        connection_id: &str,
        server_name: &str,
    ) -> Result<PairingOutcome> {
        let cloud_base_url = normalize_cloud_base_url(cloud_base_url)?;
        let pairing_code = pairing_code.trim();
        ensure!(
            (20..=80).contains(&pairing_code.len()),
            "pairing code must contain between 20 and 80 characters"
        );
        ensure!(
            Uuid::parse_str(connection_id).is_ok(),
            "connection identity is invalid"
        );

        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key_base64 = URL_SAFE_NO_PAD.encode(signing_key.verifying_key().as_bytes());
        let payload = PairingPayload {
            pairing_code,
            installation_id: connection_id,
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
        ensure!(
            Uuid::parse_str(&data.device_id).is_ok(),
            "Microgifter returned an invalid HomeServer device identity"
        );
        ensure!(
            data.device_token.len() >= 32 && !data.scopes.is_empty(),
            "Microgifter returned incomplete HomeServer credentials"
        );

        Ok(PairingOutcome {
            cloud_base_url,
            device_id: data.device_id,
            public_key_base64,
            scopes: data.scopes,
            secrets: DeviceSecrets {
                device_token: data.device_token,
                signing_key_base64: URL_SAFE_NO_PAD.encode(signing_key.to_bytes()),
            },
        })
    }

    async fn verify(&self, base_url: &str, device_id: &str, secrets: &DeviceSecrets) -> Result<()> {
        let timestamp = Utc::now().timestamp().to_string();
        let nonce = Uuid::new_v4().simple().to_string();
        let canonical = canonical_request(&Method::GET, STATUS_PATH, &timestamp, &nonce, "");
        let signing_bytes = URL_SAFE_NO_PAD
            .decode(&secrets.signing_key_base64)
            .context("HomeServer signing key is invalid")?;
        let signing_array: [u8; 32] = signing_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("HomeServer signing key has an invalid length"))?;
        let signing_key = SigningKey::from_bytes(&signing_array);
        let signature = URL_SAFE_NO_PAD.encode(signing_key.sign(canonical.as_bytes()).to_bytes());
        let _: Value = decode_response(
            self.client
                .get(format!("{base_url}{STATUS_PATH}"))
                .bearer_auth(&secrets.device_token)
                .header("X-MG-Homeserver-ID", device_id)
                .header("X-MG-Timestamp", timestamp)
                .header("X-MG-Nonce", nonce)
                .header("X-MG-Signature", signature)
                .header("X-MG-Homeserver-Version", env!("CARGO_PKG_VERSION"))
                .send()
                .await
                .context("Microgifter signed pairing verification failed")?,
        )
        .await?;
        Ok(())
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/v1/cloud/connections/pair-v2",
            post(pair_cloud_connection_v2),
        )
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub async fn reject_legacy_pairing(request: Request, next: Next) -> Response {
    if request.uri().path() == LEGACY_PAIR_PATH {
        return (
            StatusCode::GONE,
            Json(ApiError {
                ok: false,
                error: "connection_scoped_pairing_required",
                message: "Use the connection-scoped HomeServer pairing endpoint.".to_owned(),
            }),
        )
            .into_response();
    }
    next.run(request).await
}

async fn pair_cloud_connection_v2(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PairCloudConnectionV2Request>,
) -> ApiResult<CloudConnectionV2Summary> {
    pair_connection(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("cloud_connection_pairing_failed", error))
}

async fn pair_connection(
    state: Arc<AppState>,
    request: PairCloudConnectionV2Request,
) -> Result<CloudConnectionV2Summary> {
    let provider_key = request.provider_key.trim().to_lowercase();
    ensure!(
        provider_key == "microgifter",
        "cloud provider adapter is not installed"
    );
    let display_name = sanitize_required_text(&request.display_name, 120, "display name")?;
    let tenant_id = sanitize_optional_text(request.tenant_id.as_deref(), 120, "tenant id")?;
    let site_id = sanitize_optional_text(request.site_id.as_deref(), 120, "site id")?;
    let connection_id = Uuid::new_v4().to_string();
    let installation_id = database::installation_id(&*state.connection()?)?;
    let credential_key = format!("{installation_id}:cloud:{connection_id}");
    let client = MicrogifterPairingClient::new()?;
    let outcome = client
        .pair(
            &request.cloud_base_url,
            &request.pairing_code,
            &connection_id,
            &state.config.server_name,
        )
        .await?;

    save_secrets(&credential_key, &outcome.secrets)?;
    if let Err(error) = persist_connection(
        &state,
        &connection_id,
        &provider_key,
        &display_name,
        tenant_id.as_deref(),
        site_id.as_deref(),
        &credential_key,
        &outcome,
        request.make_default.unwrap_or(false),
    ) {
        let _ = delete_secrets(&credential_key);
        return Err(error);
    }

    if let Err(error) = client
        .verify(
            &outcome.cloud_base_url,
            &outcome.device_id,
            &outcome.secrets,
        )
        .await
    {
        mark_pairing_error(&state, &connection_id, &public_cloud_error(&error))?;
        return Err(error).context("pairing completed but signed cloud verification failed");
    }
    mark_pairing_success(&state, &connection_id)?;
    enqueue_initial_heartbeat(&state, &connection_id, &provider_key, tenant_id, site_id)?;
    connection_summary(&state, &connection_id)
}

#[allow(clippy::too_many_arguments)]
fn persist_connection(
    state: &AppState,
    connection_id: &str,
    provider_key: &str,
    display_name: &str,
    tenant_id: Option<&str>,
    site_id: Option<&str>,
    credential_key: &str,
    outcome: &PairingOutcome,
    requested_default: bool,
) -> Result<()> {
    let connection = state.connection()?;
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM cloud_connections", [], |row| {
        row.get(0)
    })?;
    ensure!(count < MAX_CONNECTIONS, "cloud connection limit reached");
    let default_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM cloud_connections WHERE is_default=1 AND state!='disconnected'",
        [],
        |row| row.get(0),
    )?;
    let make_default = requested_default || default_count == 0;
    let transaction = connection.unchecked_transaction()?;
    if make_default {
        transaction.execute("UPDATE cloud_connections SET is_default=0", [])?;
    }
    transaction.execute(
        "INSERT INTO cloud_connections (connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,public_key_base64,credential_key,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'pairing',?10,?11,strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL,NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            connection_id,
            provider_key,
            display_name,
            outcome.cloud_base_url,
            tenant_id,
            site_id,
            outcome.device_id,
            outcome.public_key_base64,
            credential_key,
            serde_json::to_string(&outcome.scopes)?,
            if make_default { 1_i64 } else { 0_i64 },
        ],
    )?;
    transaction.execute(
        "INSERT INTO cloud_connection_events (event_id,connection_id,event_type,outcome,detail_code,metadata_json,created_at_utc) VALUES (?1,?2,'connection.pairing','success',NULL,?3,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            Uuid::new_v4().to_string(),
            connection_id,
            serde_json::to_string(&json!({
                "provider_key": provider_key,
                "cloud_installation_id": connection_id,
            }))?,
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

fn mark_pairing_success(state: &AppState, connection_id: &str) -> Result<()> {
    let connection = state.connection()?;
    connection.execute(
        "UPDATE cloud_connections SET state='connected',last_success_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?1",
        params![connection_id],
    )?;
    connection.execute(
        "INSERT INTO cloud_connection_events (event_id,connection_id,event_type,outcome,detail_code,metadata_json,created_at_utc) VALUES (?1,?2,'connection.paired','success',NULL,'{}',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![Uuid::new_v4().to_string(), connection_id],
    )?;
    Ok(())
}

fn mark_pairing_error(state: &AppState, connection_id: &str, code: &str) -> Result<()> {
    let connection = state.connection()?;
    connection.execute(
        "UPDATE cloud_connections SET state='degraded',last_error=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE connection_id=?2",
        params![code, connection_id],
    )?;
    connection.execute(
        "INSERT INTO cloud_connection_events (event_id,connection_id,event_type,outcome,detail_code,metadata_json,created_at_utc) VALUES (?1,?2,'connection.pairing','error',?3,'{}',strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![Uuid::new_v4().to_string(), connection_id, code],
    )?;
    Ok(())
}

fn enqueue_initial_heartbeat(
    state: &AppState,
    connection_id: &str,
    provider_key: &str,
    tenant_id: Option<String>,
    site_id: Option<String>,
) -> Result<()> {
    let connection = state.connection()?;
    let physical_installation_id = database::installation_id(&connection)?;
    let bucket = Utc::now().timestamp() / 300;
    let idempotency_key = format!("heartbeat:{connection_id}:{bucket}");
    let payload = json!({
        "installation_id": physical_installation_id,
        "connection_id": connection_id,
        "provider_key": provider_key,
        "tenant_id": tenant_id,
        "site_id": site_id,
        "server_name": &state.config.server_name,
        "version": env!("CARGO_PKG_VERSION"),
    });
    let payload_json = serde_json::to_string(&payload)?;
    ensure!(
        payload_json.len() <= MAX_SYNC_PAYLOAD_BYTES,
        "initial heartbeat payload exceeds the HomeServer size limit"
    );
    connection.execute(
        "INSERT OR IGNORE INTO cloud_sync_queue (connection_id,idempotency_key,operation_type,payload_json,state,attempts,available_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,'device.heartbeat',?3,'pending',0,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![connection_id, idempotency_key, payload_json],
    )?;
    Ok(())
}

fn connection_summary(state: &AppState, connection_id: &str) -> Result<CloudConnectionV2Summary> {
    let connection = state.connection()?;
    let row = connection
        .query_row(
            "SELECT provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,state,scopes_json,is_default,paired_at_utc,last_success_utc,last_error FROM cloud_connections WHERE connection_id=?1",
            params![connection_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            },
        )
        .optional()?
        .context("paired cloud connection was not found")?;
    let pending: i64 = connection.query_row(
        "SELECT COUNT(*) FROM cloud_sync_queue WHERE connection_id=?1 AND state IN ('pending','processing')",
        params![connection_id],
        |row| row.get(0),
    )?;
    Ok(CloudConnectionV2Summary {
        connection_id: connection_id.to_owned(),
        provider_key: row.0,
        display_name: row.1,
        cloud_base_url: row.2,
        tenant_id: row.3,
        site_id: row.4,
        device_id: row.5,
        state: row.6,
        scopes: serde_json::from_str(&row.7).unwrap_or_default(),
        is_default: row.8 == 1,
        paired_at_utc: row.9,
        last_success_utc: row.10,
        last_error: row.11,
        pending_sync: pending.max(0) as u64,
    })
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

fn delete_secrets(credential_key: &str) -> Result<()> {
    match credential_entry(credential_key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to delete cloud connection credentials"),
    }
}

fn sanitize_required_text(value: &str, max: usize, label: &str) -> Result<String> {
    sanitize_optional_text(Some(value), max, label)?.with_context(|| format!("{label} is required"))
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

fn public_cloud_error(error: &anyhow::Error) -> String {
    let text = error.to_string().to_lowercase();
    if text.contains("http 401") || text.contains("http 403") || text.contains("revoked") {
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

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    let text = error.to_string().to_lowercase();
    let status = if text.contains("pairing code")
        || text.contains("cloud url")
        || text.contains("not installed")
        || text.contains("invalid")
    {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if text.contains("limit") || text.contains("already") {
        StatusCode::CONFLICT
    } else if text.contains("cloud") || text.contains("microgifter") {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
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

    #[test]
    fn pairing_payload_uses_connection_identity() {
        let connection_id = Uuid::new_v4().to_string();
        let payload = PairingPayload {
            pairing_code: "12345678901234567890",
            installation_id: &connection_id,
            server_name: "HomeServer",
            version: "0.1.3",
            public_key: "key",
        };
        assert_eq!(payload.installation_id, connection_id);
    }

    #[test]
    fn provider_and_transport_are_closed_world() {
        assert_eq!(
            normalize_cloud_base_url("https://microgifter.com/").unwrap(),
            "https://microgifter.com"
        );
        assert!(normalize_cloud_base_url("http://microgifter.com").is_err());
        assert_eq!(LEGACY_PAIR_PATH, "/v1/cloud/connections/pair");
    }
}
