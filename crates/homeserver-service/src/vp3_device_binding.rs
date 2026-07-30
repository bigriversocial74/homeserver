use crate::{database, AppState};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const ACTIVATION_PATH: &str = "/v1/software-authority/vp3/activate";
const MAX_ACTIVATION_BODY_BYTES: usize = 64 * 1024;
const FINGERPRINT_NAMESPACE: &str = "MicrogifterHomeServer:vp3-device:";

#[derive(Debug, Serialize)]
struct DeviceIdentitySnapshot {
    device_fingerprint: String,
    algorithm: &'static str,
    source: &'static str,
}

#[derive(Debug, Serialize)]
struct BindingError {
    ok: bool,
    error: &'static str,
    message: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/v1/software-authority/vp3/device-identity",
            get(device_identity_handler),
        )
        .with_state(state)
}

pub async fn bind_activation_identity(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::POST || request.uri().path() != ACTIVATION_PATH {
        return next.run(request).await;
    }

    match bind_request(&state, request).await {
        Ok(request) => next.run(request).await,
        Err(error) => binding_error_response(error),
    }
}

async fn device_identity_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DeviceIdentitySnapshot>, (StatusCode, Json<BindingError>)> {
    local_device_fingerprint(&state)
        .map(|device_fingerprint| {
            Json(DeviceIdentitySnapshot {
                device_fingerprint,
                algorithm: "SHA-256",
                source: "local_installation_identity",
            })
        })
        .map_err(|error| binding_error("vp3_device_identity_unavailable", error))
}

async fn bind_request(state: &AppState, request: Request) -> Result<Request> {
    let expected = local_device_fingerprint(state)?;
    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_ACTIVATION_BODY_BYTES)
        .await
        .context("VP3 activation request exceeds the supported size")?;
    let mut payload: Value =
        serde_json::from_slice(&bytes).context("VP3 activation request JSON is invalid")?;
    bind_payload(&mut payload, &expected)?;
    let encoded = serde_json::to_vec(&payload).context("VP3 activation request could not be bound")?;

    parts.headers.remove(header::CONTENT_LENGTH);
    parts
        .headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(Request::from_parts(parts, Body::from(encoded)))
}

fn bind_payload(payload: &mut Value, expected: &str) -> Result<()> {
    ensure!(valid_fingerprint(expected), "local VP3 device identity is invalid");
    let object = payload
        .as_object_mut()
        .context("VP3 activation request must be a JSON object")?;

    match object.get("device_fingerprint") {
        Some(Value::String(value)) if !value.eq_ignore_ascii_case(expected) => {
            bail!("VP3 activation fingerprint does not match this HomeServer")
        }
        Some(Value::String(_)) | None => {}
        Some(_) => bail!("VP3 activation fingerprint must be a string"),
    }

    object.insert(
        "device_fingerprint".to_owned(),
        Value::String(expected.to_owned()),
    );
    Ok(())
}

fn local_device_fingerprint(state: &AppState) -> Result<String> {
    let connection = state.connection()?;
    let installation_id = database::installation_id(&connection)?;
    fingerprint_from_installation_id(&installation_id)
}

fn fingerprint_from_installation_id(installation_id: &str) -> Result<String> {
    let installation_id = installation_id.trim();
    ensure!(
        !installation_id.is_empty(),
        "HomeServer installation identity is unavailable"
    );
    Ok(hex::encode(Sha256::digest(
        format!("{FINGERPRINT_NAMESPACE}{installation_id}").as_bytes(),
    )))
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn binding_error_response(error: anyhow::Error) -> Response {
    let (status, payload) = binding_error("vp3_device_identity_mismatch", error);
    (status, payload).into_response()
}

fn binding_error(
    code: &'static str,
    error: impl std::fmt::Display,
) -> (StatusCode, Json<BindingError>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(BindingError {
            ok: false,
            error: code,
            message: error
                .to_string()
                .chars()
                .filter(|character| !character.is_control())
                .take(300)
                .collect(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fingerprint_is_stable_namespaced_and_bounded() {
        let first = fingerprint_from_installation_id("installation-one").unwrap();
        let same = fingerprint_from_installation_id(" installation-one ").unwrap();
        let second = fingerprint_from_installation_id("installation-two").unwrap();
        assert_eq!(first, same);
        assert_ne!(first, second);
        assert!(valid_fingerprint(&first));
    }

    #[test]
    fn activation_payload_receives_local_identity() {
        let fingerprint = "a".repeat(64);
        let mut payload = json!({"account_id": 1, "device_public_id": "HS-1"});
        bind_payload(&mut payload, &fingerprint).unwrap();
        assert_eq!(
            payload.get("device_fingerprint").and_then(Value::as_str),
            Some(fingerprint.as_str())
        );
    }

    #[test]
    fn conflicting_activation_identity_is_rejected() {
        let expected = "a".repeat(64);
        let mut payload = json!({"device_fingerprint": "b".repeat(64)});
        assert!(bind_payload(&mut payload, &expected).is_err());
    }
}
