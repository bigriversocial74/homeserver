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

#[tauri::command]
pub async fn homeserver_microgifter_status() -> Result<Value, String> {
    get_json("/v1/providers/microgifter/status").await
}

#[tauri::command]
pub async fn homeserver_connect_microgifter(request: Value) -> Result<Value, String> {
    post_json("/v1/providers/microgifter/connect", &request).await
}

#[tauri::command]
pub async fn homeserver_refresh_microgifter_entitlement(
    connection_id: String,
) -> Result<Value, String> {
    post_json(
        "/v1/providers/microgifter/entitlement/refresh",
        &json!({ "connection_id": connection_id }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_send_microgifter_heartbeat(connection_id: String) -> Result<Value, String> {
    post_json(
        "/v1/providers/microgifter/heartbeat",
        &json!({ "connection_id": connection_id }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_rotate_microgifter_credentials(
    connection_id: String,
) -> Result<Value, String> {
    post_json(
        "/v1/providers/microgifter/credentials/rotate",
        &json!({ "connection_id": connection_id }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_microgifter_update_preferences() -> Result<Value, String> {
    get_json("/v1/providers/microgifter/update-preferences").await
}

#[tauri::command]
pub async fn homeserver_save_microgifter_update_preferences(
    request: Value,
) -> Result<Value, String> {
    post_json("/v1/providers/microgifter/update-preferences", &request).await
}

#[tauri::command]
pub async fn homeserver_authorize_microgifter_update(request: Value) -> Result<Value, String> {
    post_json("/v1/providers/microgifter/updates/authorize", &request).await
}

#[tauri::command]
pub async fn homeserver_start_microgifter_device_replacement(
    connection_id: String,
    new_device_display_name: String,
) -> Result<Value, String> {
    post_json(
        "/v1/providers/microgifter/device-replacement/start",
        &json!({
            "connection_id": connection_id,
            "new_device_display_name": new_device_display_name,
        }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_complete_microgifter_device_replacement(
    replacement_id: String,
    new_connection_id: String,
) -> Result<Value, String> {
    post_json(
        "/v1/providers/microgifter/device-replacement/complete",
        &json!({
            "replacement_id": replacement_id,
            "new_connection_id": new_connection_id,
        }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_pod_status() -> Result<Value, String> {
    get_json("/v1/providers/pod/status").await
}

#[tauri::command]
pub async fn homeserver_connect_pod(request: Value) -> Result<Value, String> {
    post_json("/v1/providers/pod/connect", &request).await
}

#[tauri::command]
pub async fn homeserver_update_pod_runtime(request: Value) -> Result<Value, String> {
    post_json("/v1/providers/pod/runtime", &request).await
}

#[tauri::command]
pub async fn homeserver_poll_pod(connection_id: String) -> Result<Value, String> {
    post_json(
        "/v1/providers/pod/poll",
        &json!({ "connection_id": connection_id }),
    )
    .await
}

#[tauri::command]
pub async fn homeserver_disconnect_pod(connection_id: String) -> Result<Value, String> {
    post_json(
        "/v1/providers/pod/disconnect",
        &json!({ "connection_id": connection_id }),
    )
    .await
}
