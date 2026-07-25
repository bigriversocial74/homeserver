use crate::AppState;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use futures_util::StreamExt;
use microgifter_homeserver_core::{
    ApplyUpdateRequest, BackupActionResult, BackupCatalog, BackupReferenceRequest,
    CreateBackupRequest, HealthSnapshot, ServiceState, UpdateActionResult, UpdateStatus,
    LOCAL_CLIENT_HEADER, LOCAL_CLIENT_VALUE,
};
use serde::Serialize;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use tower_http::{
    catch_panic::CatchPanicLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use zeroize::Zeroizing;

const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;
const MAX_IMPORT_BODY_BYTES: usize = 320 * 1024 * 1024;
const MAX_ENCODED_PASSPHRASE_HEADER_BYTES: usize = 4 * 1024;
const PASSPHRASE_HEADER: &str = "x-mg-recovery-passphrase";
const LOCAL_API_HOST: &str = "127.0.0.1:47831";

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;
type ResponseResult = Result<Response, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    let control_routes = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/status", get(status))
        .route("/v1/backups", get(backups))
        .route("/v1/backups/create", post(create_backup))
        .route("/v1/backups/verify", post(verify_backup))
        .route("/v1/backups/stage-restore", post(stage_restore))
        .route("/v1/updates", get(update_status))
        .route("/v1/updates/check", post(check_updates))
        .route("/v1/updates/download", post(download_update))
        .route("/v1/updates/apply", post(apply_update))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES));
    let transfer_routes = Router::new()
        .route("/v1/backups/import", post(import_recovery_package))
        .route(
            "/v1/backups/{backup_id}/package",
            get(export_recovery_package),
        )
        .layer(DefaultBodyLimit::max(MAX_IMPORT_BODY_BYTES));

    Router::new()
        .merge(control_routes)
        .merge(transfer_routes)
        .with_state(state)
}

pub fn secure(router: Router) -> Router {
    router
        .layer(middleware::from_fn(protect_local_api))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("cross-origin-resource-policy"),
            HeaderValue::from_static("same-origin"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
}

async fn protect_local_api(request: Request, next: Next) -> Response {
    if let Some(message) = local_request_rejection(
        request.method(),
        request.uri().path(),
        request.headers(),
    ) {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiError {
                ok: false,
                error: "local_client_required",
                message: message.to_owned(),
            }),
        )
            .into_response();
    }
    next.run(request).await
}

fn local_request_rejection(
    _method: &Method,
    path: &str,
    headers: &HeaderMap,
) -> Option<&'static str> {
    let valid_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case(LOCAL_API_HOST));
    if !valid_host {
        return Some("The HomeServer local API requires its fixed loopback host.");
    }
    if headers.contains_key(header::ORIGIN)
        || headers.contains_key("sec-fetch-site")
        || headers.contains_key("forwarded")
        || headers.contains_key("x-forwarded-host")
        || headers.contains_key("x-forwarded-for")
    {
        return Some("Browser-originated or forwarded requests are not accepted by the HomeServer local API.");
    }
    if path.starts_with("/v1/") {
        let authorized = headers
            .get(LOCAL_CLIENT_HEADER)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == LOCAL_CLIENT_VALUE);
        if !authorized {
            return Some("A trusted HomeServer local client header is required.");
        }
    }
    None
}

async fn healthz(State(state): State<Arc<AppState>>) -> StatusCode {
    match state.snapshot().state {
        ServiceState::Running => StatusCode::NO_CONTENT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn status(State(state): State<Arc<AppState>>) -> Json<HealthSnapshot> {
    Json(state.snapshot())
}

async fn backups(State(state): State<Arc<AppState>>) -> ApiResult<BackupCatalog> {
    tokio::task::spawn_blocking(move || state.backup_catalog())
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("backup_catalog_failed", error))
}

async fn create_backup(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateBackupRequest>,
) -> ApiResult<BackupActionResult> {
    tokio::task::spawn_blocking(move || state.create_backup(request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("backup_creation_failed", error))
}

async fn verify_backup(
    State(state): State<Arc<AppState>>,
    Json(request): Json<BackupReferenceRequest>,
) -> ApiResult<BackupActionResult> {
    tokio::task::spawn_blocking(move || state.verify_backup(request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("backup_verification_failed", error))
}

async fn stage_restore(
    State(state): State<Arc<AppState>>,
    Json(request): Json<BackupReferenceRequest>,
) -> ApiResult<BackupActionResult> {
    tokio::task::spawn_blocking(move || state.stage_restore(request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("restore_stage_failed", error))
}

async fn update_status(State(state): State<Arc<AppState>>) -> ApiResult<UpdateStatus> {
    tokio::task::spawn_blocking(move || state.update_status())
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("update_status_failed", error))
}

async fn check_updates(State(state): State<Arc<AppState>>) -> ApiResult<UpdateActionResult> {
    state
        .check_for_updates()
        .await
        .map(Json)
        .map_err(|error| action_error("update_check_failed", error))
}

async fn download_update(State(state): State<Arc<AppState>>) -> ApiResult<UpdateActionResult> {
    state
        .download_update()
        .await
        .map(Json)
        .map_err(|error| action_error("update_download_failed", error))
}

async fn apply_update(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ApplyUpdateRequest>,
) -> ApiResult<UpdateActionResult> {
    tokio::task::spawn_blocking(move || state.apply_update(request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("update_apply_failed", error))
}

async fn import_recovery_package(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<BackupActionResult> {
    let passphrase = decode_passphrase(&headers)?;
    let temporary_path = state.new_import_path();
    let mut output = tokio::fs::File::create(&temporary_path)
        .await
        .map_err(|error| internal_error("recovery_import_staging_failed", error.into()))?;
    let mut stream = body.into_data_stream();
    let mut total_bytes = 0_usize;

    let transfer_result = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|error| action_error("recovery_import_stream_failed", error.into()))?;
            total_bytes = total_bytes.checked_add(chunk.len()).ok_or_else(|| {
                action_error(
                    "recovery_import_too_large",
                    anyhow::anyhow!("recovery package size overflow"),
                )
            })?;
            if total_bytes > MAX_IMPORT_BODY_BYTES {
                return Err(action_error(
                    "recovery_import_too_large",
                    anyhow::anyhow!("recovery package exceeds the import size limit"),
                ));
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|error| internal_error("recovery_import_write_failed", error.into()))?;
        }
        output
            .sync_all()
            .await
            .map_err(|error| internal_error("recovery_import_sync_failed", error.into()))?;
        Ok::<(), (StatusCode, Json<ApiError>)>(())
    }
    .await;
    if let Err(error) = transfer_result {
        drop(output);
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(error);
    }
    drop(output);
    if total_bytes <= 12 {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(action_error(
            "recovery_import_invalid",
            anyhow::anyhow!("recovery package is empty or truncated"),
        ));
    }

    let imported_path = temporary_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        state.import_recovery_package(imported_path, passphrase.to_string())
    })
    .await
    .map_err(task_error)?;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary_path).await;
    }
    result
        .map(Json)
        .map_err(|error| action_error("recovery_import_failed", error))
}

async fn export_recovery_package(
    State(state): State<Arc<AppState>>,
    Path(backup_id): Path<String>,
) -> ResponseResult {
    let package =
        tokio::task::spawn_blocking(move || state.recovery_package_for_export(&backup_id))
            .await
            .map_err(task_error)?
            .map_err(|error| action_error("recovery_export_failed", error))?;
    let file = tokio::fs::File::open(&package.path)
        .await
        .map_err(|error| internal_error("recovery_export_open_failed", error.into()))?;
    let file_name: String = package
        .file_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || ".-_".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "application/vnd.microgifter.homeserver-backup",
        )
        .header(header::CONTENT_LENGTH, package.size_bytes.to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{file_name}\""),
        )
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|error| internal_error("recovery_export_response_failed", error.into()))
}

fn decode_passphrase(
    headers: &HeaderMap,
) -> Result<Zeroizing<String>, (StatusCode, Json<ApiError>)> {
    let encoded = headers
        .get(PASSPHRASE_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            action_error(
                "recovery_passphrase_required",
                anyhow::anyhow!("recovery passphrase is required"),
            )
        })?;
    if encoded.len() > MAX_ENCODED_PASSPHRASE_HEADER_BYTES {
        return Err(action_error(
            "recovery_passphrase_invalid",
            anyhow::anyhow!("recovery passphrase header is too large"),
        ));
    }
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
        action_error(
            "recovery_passphrase_invalid",
            anyhow::anyhow!("recovery passphrase encoding is invalid"),
        )
    })?;
    let passphrase = String::from_utf8(bytes).map_err(|_| {
        action_error(
            "recovery_passphrase_invalid",
            anyhow::anyhow!("recovery passphrase must be UTF-8"),
        )
    })?;
    let count = passphrase.chars().count();
    if !(12..=256).contains(&count) {
        return Err(action_error(
            "recovery_passphrase_invalid",
            anyhow::anyhow!("recovery passphrase must contain between 12 and 256 characters"),
        ));
    }
    Ok(Zeroizing::new(passphrase))
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "homeserver_task_failed",
        anyhow::anyhow!(error),
    )
}

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    let text = error.to_string().to_lowercase();
    let status = if text.contains("passphrase")
        || text.contains("confirmation")
        || text.contains("not found")
        || text.contains("unsupported")
        || text.contains("already staged")
        || text.contains("does not match")
        || text.contains("invalid")
        || text.contains("conflict")
        || text.contains("outside managed")
        || text.contains("only portable")
        || text.contains("not ready")
        || text.contains("not available")
        || text.contains("not staged")
        || text.contains("size limit")
        || text.contains("requires a newer")
        || text.contains("type update")
    {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if text.contains("request was rejected") {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    api_error(status, code, error)
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::INTERNAL_SERVER_ERROR, code, error)
}

fn api_error(
    status: StatusCode,
    code: &'static str,
    error: anyhow::Error,
) -> (StatusCode, Json<ApiError>) {
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
    fn browser_forwarded_and_unmarked_api_requests_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static(LOCAL_API_HOST));
        assert!(local_request_rejection(&Method::GET, "/healthz", &headers).is_none());
        assert!(local_request_rejection(&Method::GET, "/v1/test", &headers).is_some());

        headers.insert(
            LOCAL_CLIENT_HEADER,
            HeaderValue::from_static(LOCAL_CLIENT_VALUE),
        );
        assert!(local_request_rejection(&Method::GET, "/v1/test", &headers).is_none());
        assert!(local_request_rejection(&Method::POST, "/v1/test", &headers).is_none());

        headers.insert(header::ORIGIN, HeaderValue::from_static("https://example.com"));
        assert!(local_request_rejection(&Method::POST, "/v1/test", &headers).is_some());
        headers.remove(header::ORIGIN);
        headers.insert("x-forwarded-host", HeaderValue::from_static("example.com"));
        assert!(local_request_rejection(&Method::GET, "/v1/test", &headers).is_some());
    }

    #[test]
    fn local_api_rejects_an_unexpected_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:47831"));
        headers.insert(
            LOCAL_CLIENT_HEADER,
            HeaderValue::from_static(LOCAL_CLIENT_VALUE),
        );
        assert!(local_request_rejection(&Method::GET, "/v1/test", &headers).is_some());
    }

    #[test]
    fn absent_verified_update_is_a_validation_error() {
        let error = action_error(
            "update_download_failed",
            anyhow::anyhow!("update is not available in the available state"),
        );
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn accepts_maximum_multibyte_passphrase_header() {
        let passphrase = "🦀".repeat(256);
        let encoded = URL_SAFE_NO_PAD.encode(passphrase.as_bytes());
        assert!(encoded.len() > 1024);
        assert!(encoded.len() <= MAX_ENCODED_PASSPHRASE_HEADER_BYTES);

        let mut headers = HeaderMap::new();
        headers.insert(
            PASSPHRASE_HEADER,
            HeaderValue::from_str(&encoded).expect("encoded passphrase header should be valid"),
        );

        let decoded = decode_passphrase(&headers).expect("maximum passphrase should be accepted");
        assert_eq!(decoded.as_str(), passphrase);
    }

    #[test]
    fn rejects_oversized_passphrase_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            PASSPHRASE_HEADER,
            HeaderValue::from_str(&"A".repeat(MAX_ENCODED_PASSPHRASE_HEADER_BYTES + 1))
                .expect("oversized test header should still be syntactically valid"),
        );

        let error = decode_passphrase(&headers).expect_err("oversized header should be rejected");
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.1 .0.error, "recovery_passphrase_invalid");
    }
}
