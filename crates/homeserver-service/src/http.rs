use crate::AppState;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderValue, StatusCode},
    routing::{get, post},
    Json, Router,
};
use microgifter_homeserver_core::{
    BackupActionResult, BackupCatalog, BackupReferenceRequest, CreateBackupRequest, HealthSnapshot,
    ServiceState,
};
use serde::Serialize;
use std::sync::Arc;
use tower_http::{
    catch_panic::CatchPanicLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};

const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/status", get(status))
        .route("/v1/backups", get(backups))
        .route("/v1/backups/create", post(create_backup))
        .route("/v1/backups/verify", post(verify_backup))
        .route("/v1/backups/stage-restore", post(stage_restore))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
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

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "backup_task_failed",
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
        || text.contains("integrity is invalid")
    {
        StatusCode::UNPROCESSABLE_ENTITY
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
