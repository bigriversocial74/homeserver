use super::{get_json, post_json};
use serde_json::{json, Value};

#[tauri::command]
pub(crate) async fn homeserver_vp3_authority_status() -> Result<Value, String> {
    get_json("/v1/software-authority/vp3").await
}

#[tauri::command]
pub(crate) async fn homeserver_vp3_device_identity() -> Result<Value, String> {
    get_json("/v1/software-authority/vp3/device-identity").await
}

#[tauri::command]
pub(crate) async fn homeserver_activate_vp3_authority(
    account_id: i64,
    device_public_id: String,
    license_public_id: Option<String>,
    credential: String,
    enrollment_code: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/software-authority/vp3/activate",
        &json!({
            "account_id": account_id,
            "device_public_id": device_public_id,
            "license_public_id": license_public_id,
            "credential": credential,
            "enrollment_code": enrollment_code,
            "confirmation": confirmation
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_vp3_heartbeat() -> Result<Value, String> {
    post_json("/v1/software-authority/vp3/heartbeat", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_vp3_refresh_lease() -> Result<Value, String> {
    post_json("/v1/software-authority/vp3/lease", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_vp3_check_update() -> Result<Value, String> {
    post_json("/v1/software-authority/vp3/check-update", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_vp3_download_update() -> Result<Value, String> {
    post_json("/v1/software-authority/vp3/download-update", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_vp3_submit_receipts() -> Result<Value, String> {
    post_json("/v1/software-authority/vp3/submit-receipts", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_disconnect_vp3_authority(
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/software-authority/vp3/disconnect",
        &json!({ "confirmation": confirmation }),
    )
    .await
}
