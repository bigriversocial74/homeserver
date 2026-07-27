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

#[tauri::command]
pub async fn homeserver_cloud_connections() -> Result<Value, String> {
    get_json("/v1/cloud/connections").await
}

#[tauri::command]
pub async fn homeserver_pair_cloud_connection(request: Value) -> Result<Value, String> {
    post_json("/v1/cloud/connections/pair-v2", &request).await
}

#[tauri::command]
pub async fn homeserver_disconnect_cloud_connection(
    connection_id: String,
) -> Result<Value, String> {
    post_json(
        "/v1/cloud/connections/disconnect",
        &json!({ "connection_id": connection_id }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_enqueue_connection_sync(request: Value) -> Result<Value, String> {
    post_json("/v1/cloud/connections/enqueue", &request).await
}

#[tauri::command]
pub async fn homeserver_sync_cloud_connection(connection_id: String) -> Result<Value, String> {
    post_json(
        "/v1/cloud/connections/sync",
        &json!({ "connection_id": connection_id }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_sync_all_cloud_connections() -> Result<Value, String> {
    post_json("/v1/cloud/connections/sync-all", &json!({})).await
}
