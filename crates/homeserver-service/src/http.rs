use crate::AppState;
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    routing::get,
    Json, Router,
};
use microgifter_homeserver_core::{HealthSnapshot, ServiceState};
use std::sync::Arc;
use tower_http::{
    catch_panic::CatchPanicLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/status", get(status))
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
