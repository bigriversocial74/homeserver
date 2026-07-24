use crate::AppState;
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use microgifter_homeserver_core::HealthSnapshot;
use std::sync::Arc;
use tower_http::{catch_panic::CatchPanicLayer, trace::TraceLayer};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/status", get(status))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn healthz() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn status(State(state): State<Arc<AppState>>) -> Json<HealthSnapshot> {
    Json(state.snapshot())
}
