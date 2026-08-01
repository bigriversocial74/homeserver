use super::{get_json, post_json};
use serde_json::Value;

#[tauri::command]
pub(crate) async fn homeserver_audio_status() -> Result<Value, String> {
    get_json("/v1/audio/status").await
}

#[tauri::command]
pub(crate) async fn homeserver_audio_action(request: Value) -> Result<Value, String> {
    match request.get("action").and_then(Value::as_str) {
        Some("start_session") => post_json("/v1/audio/sessions/start", &request).await,
        Some("set_state") => post_json("/v1/audio/sessions/state", &request).await,
        Some("finalize_segment") => post_json("/v1/audio/segments", &request).await,
        Some("update_transcript") => post_json("/v1/audio/segments/transcript", &request).await,
        Some("delete_session") => post_json("/v1/audio/sessions/delete", &request).await,
        _ => Err("Unsupported Agent audio action.".to_owned()),
    }
}
