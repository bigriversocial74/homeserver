use super::{get_json, post_json};
use serde_json::{json, Value};

#[tauri::command]
pub(crate) async fn homeserver_federated_settings() -> Result<Value, String> {
    get_json("/v1/federated-settings").await
}

#[tauri::command]
pub(crate) async fn homeserver_update_federated_setting(
    setting_key: String,
    value: Value,
    expected_local_revision: u64,
) -> Result<Value, String> {
    post_json(
        "/v1/federated-settings/update",
        &json!({
            "setting_key": setting_key,
            "value": value,
            "expected_local_revision": expected_local_revision
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_sync_federated_settings() -> Result<Value, String> {
    post_json("/v1/federated-settings/sync", &json!({})).await
}
