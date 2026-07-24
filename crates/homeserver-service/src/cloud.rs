use crate::{database, secrets::DeviceSecrets};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use reqwest::{Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use url::Url;
use uuid::Uuid;

const PAIR_PATH: &str = "/api/homeserver/pair.php";
const STATUS_PATH: &str = "/api/homeserver/status.php";
const SYNC_PATH: &str = "/api/homeserver/sync.php";

#[derive(Debug)]
pub struct PairingOutcome {
    pub cloud_base_url: String,
    pub device_id: String,
    pub scopes: Vec<String>,
    pub public_key_base64: String,
    pub secrets: DeviceSecrets,
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
pub struct SyncOperation {
    pub idempotency_key: String,
    pub operation_type: String,
    pub payload: Value,
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
pub struct CloudClient {
    client: reqwest::Client,
}

impl CloudClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(20))
                .user_agent(format!(
                    "Microgifter-HomeServer/{}",
                    env!("CARGO_PKG_VERSION")
                ))
                .build()?,
        })
    }

    pub async fn pair(
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

    pub async fn status(
        &self,
        record: &database::CloudConnectionRecord,
        secrets: &DeviceSecrets,
    ) -> Result<()> {
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

    pub async fn sync(
        &self,
        record: &database::CloudConnectionRecord,
        secrets: &DeviceSecrets,
        operations: &[database::QueuedOperation],
    ) -> Result<Vec<database::ReceiptRecord>> {
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
        let mut receipts = Vec::with_capacity(data.receipts.len());
        for receipt in data.receipts {
            let receipt_id = receipt
                .receipt_id
                .unwrap_or_else(|| format!("local-rejected:{}", receipt.idempotency_key));
            receipts.push(database::ReceiptRecord {
                receipt_id,
                idempotency_key: receipt.idempotency_key,
                operation_type: receipt.operation_type,
                disposition: receipt.disposition,
                reason_code: receipt.reason_code,
                response: receipt.response,
            });
        }
        Ok(receipts)
    }

    async fn signed_request<T: DeserializeOwned>(
        &self,
        method: Method,
        base_url: &str,
        path: &str,
        body: &str,
        record: &database::CloudConnectionRecord,
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

        let url = format!("{base_url}{path}");
        let mut request = self
            .client
            .request(method, url)
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
        let prefix = if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
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

pub fn normalize_cloud_base_url(value: &str) -> Result<String> {
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

pub fn authentication_failed(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .starts_with("cloud_authentication_failed:")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Verifier;

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
}
