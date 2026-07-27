use super::{get_json, post_json};
use serde_json::{json, Value};

#[tauri::command]
pub(crate) async fn homeserver_models() -> Result<Value, String> {
    get_json("/v1/models").await
}

#[tauri::command]
pub(crate) async fn homeserver_pull_model(model: String) -> Result<Value, String> {
    post_json("/v1/models/pull", &json!({ "model": model })).await
}

#[tauri::command]
pub(crate) async fn homeserver_delete_model(
    model: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/models/delete",
        &json!({ "model": model, "confirmation": confirmation }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_unload_model(model: String) -> Result<Value, String> {
    post_json("/v1/models/unload", &json!({ "model": model })).await
}

#[tauri::command]
pub(crate) async fn homeserver_test_model(model: String, prompt: String) -> Result<Value, String> {
    post_json(
        "/v1/models/test",
        &json!({ "model": model, "prompt": prompt }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_update_model_settings(
    default_chat_model: Option<String>,
    default_embedding_model: Option<String>,
    context_size: u32,
    test_timeout_seconds: u64,
    max_download_gb: u32,
) -> Result<Value, String> {
    post_json(
        "/v1/models/settings",
        &json!({
            "default_chat_model": default_chat_model,
            "default_embedding_model": default_embedding_model,
            "context_size": context_size,
            "test_timeout_seconds": test_timeout_seconds,
            "max_download_gb": max_download_gb
        }),
    )
    .await
}
