use crate::AppState;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderValue, StatusCode},
    routing::{delete, get, post},
    Json, Router,
};
use microgifter_homeserver_core::{
    CloudConnectionSnapshot, EnqueueSyncRequest, HealthSnapshot, PairCloudRequest, ServiceState,
    SyncRunSnapshot,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tower_http::{
    catch_panic::CatchPanicLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};

const MAX_CONTROL_BODY_BYTES: usize = 256 * 1024;

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
        .route("/v1/connection", get(connection).delete(disconnect))
        .route("/v1/connection/pair", post(pair))
        .route("/v1/sync/enqueue", post(enqueue_sync))
        .route("/v1/sync/run", post(run_sync))
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

async fn connection(State(state): State<Arc<AppState>>) -> ApiResult<CloudConnectionSnapshot> {
    state
        .cloud_snapshot()
        .map(Json)
        .map_err(|error| internal_error("connection_state_failed", error))
}

async fn pair(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PairCloudRequest>,
) -> ApiResult<CloudConnectionSnapshot> {
    state
        .pair_cloud(request)
        .await
        .map(Json)
        .map_err(|error| gateway_error("pairing_failed", error))
}

async fn disconnect(
    State(state): State<Arc<AppState>>,
) -> ApiResult<CloudConnectionSnapshot> {
    state
        .disconnect_cloud()
        .map(Json)
        .map_err(|error| internal_error("disconnect_failed", error))
}

async fn enqueue_sync(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EnqueueSyncRequest>,
) -> ApiResult<Value> {
    state
        .enqueue_sync(request)
        .map(|idempotency_key| Json(json!({ "ok": true, "idempotency_key": idempotency_key })))
        .map_err(|error| validation_error("sync_enqueue_rejected", error))
}

async fn run_sync(State(state): State<Arc<AppState>>) -> ApiResult<SyncRunSnapshot> {
    state
        .sync_once()
        .await
        .map(Json)
        .map_err(|error| gateway_error("sync_failed", error))
}

fn validation_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::UNPROCESSABLE_ENTITY, code, error)
}

fn gateway_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::BAD_GATEWAY, code, error)
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
    use crate::{config::AppConfig, database};
    use axum::{
        body::{to_bytes, Body},
        http::Request,
    };
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn test_state() -> (Arc<AppState>, tempfile::TempDir) {
        let directory = tempdir().expect("temporary directory");
        let config = AppConfig {
            data_dir: directory.path().to_path_buf(),
            database_path: directory.path().join("homeserver.sqlite3"),
            logs_dir: directory.path().join("logs"),
            server_name: "Test HomeServer".to_owned(),
        };
        std::fs::create_dir_all(&config.logs_dir).expect("logs directory");
        let connection = database::initialize(&config.database_path).expect("database");
        (Arc::new(AppState::new(config, connection).expect("state")), directory)
    }

    #[tokio::test]
    async fn connection_endpoint_starts_not_paired() {
        let (state, _directory) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/connection")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 32 * 1024).await.unwrap();
        let snapshot: CloudConnectionSnapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            snapshot.state,
            microgifter_homeserver_core::CloudConnectionState::NotPaired
        );
    }

    #[tokio::test]
    async fn commerce_work_cannot_enter_local_sync_queue() {
        let (state, _directory) = test_state();
        let response = router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/sync/enqueue")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"operation_type":"commerce.order.create","payload":{}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
