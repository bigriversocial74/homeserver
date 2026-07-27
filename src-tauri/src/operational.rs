use serde_json::Value;

use super::{get_json, post_json};

#[tauri::command]
pub(crate) async fn homeserver_operational_data() -> Result<Value, String> {
    get_json("/v1/operational-data").await
}

#[tauri::command]
pub(crate) async fn homeserver_update_operational_dataset_grant(
    request: Value,
) -> Result<Value, String> {
    post_json("/v1/operational-data/grants", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_import_operational_data(
    request: Value,
) -> Result<Value, String> {
    post_json("/v1/operational-data/import", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_query_operational_data(
    request: Value,
) -> Result<Value, String> {
    post_json("/v1/operational-data/query", &request).await
}
