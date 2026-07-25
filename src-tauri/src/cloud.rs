use super::{get_json, post_json};
use serde_json::{json, Value};

#[tauri::command]
pub async fn homeserver_cloud_status() -> Result<Value, String> {
    get_json("/v1/cloud").await
}

#[tauri::command]
pub async fn homeserver_pair_cloud(request: Value) -> Result<Value, String> {
    post_json("/v1/cloud/pair", &request).await
}

#[tauri::command]
pub async fn homeserver_disconnect_cloud() -> Result<Value, String> {
    post_json("/v1/cloud/disconnect", &json!({})).await
}

#[tauri::command]
pub async fn homeserver_cloud_vault_self_test() -> Result<Value, String> {
    post_json("/v1/cloud/vault-self-test", &json!({})).await
}

#[tauri::command]
pub async fn homeserver_enqueue_cloud_sync(request: Value) -> Result<Value, String> {
    post_json("/v1/cloud/enqueue", &request).await
}

#[tauri::command]
pub async fn homeserver_sync_cloud() -> Result<Value, String> {
    post_json("/v1/cloud/sync", &json!({})).await
}
