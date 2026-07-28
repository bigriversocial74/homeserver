use serde_json::Value;

use super::{get_json, post_json};

#[tauri::command]
pub(crate) async fn homeserver_review_intelligence() -> Result<Value, String> {
    get_json("/v1/review-intelligence").await
}

#[tauri::command]
pub(crate) async fn homeserver_update_review_intelligence_settings(
    request: Value,
) -> Result<Value, String> {
    post_json("/v1/review-intelligence/settings", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_sync_review_dataset(request: Value) -> Result<Value, String> {
    post_json("/v1/review-intelligence/sync", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_run_review_analysis(request: Value) -> Result<Value, String> {
    post_json("/v1/review-intelligence/analyze", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_record_review_recommendation_outcome(
    request: Value,
) -> Result<Value, String> {
    post_json("/v1/review-intelligence/recommendations/outcome", &request).await
}
