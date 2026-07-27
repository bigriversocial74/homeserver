use super::{get_json, post_json};
use serde_json::{json, Value};
use tauri::Manager;

const MCP_BRIDGE_FILE_NAME: &str = "microgifter-homeserver-mcp.exe";

#[tauri::command]
pub(crate) async fn homeserver_mcp() -> Result<Value, String> {
    get_json("/v1/mcp").await
}

#[tauri::command]
pub(crate) async fn homeserver_create_mcp_client(
    display_name: String,
    scopes: Vec<String>,
    expires_days: u32,
) -> Result<Value, String> {
    post_json(
        "/v1/mcp/clients",
        &json!({
            "display_name": display_name,
            "scopes": scopes,
            "expires_days": expires_days
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_revoke_mcp_client(
    client_id: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/mcp/clients/revoke",
        &json!({ "client_id": client_id, "confirmation": confirmation }),
    )
    .await
}

#[tauri::command]
pub(crate) fn homeserver_mcp_bridge_path(app: tauri::AppHandle) -> Result<String, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?;
    let path = resource_dir.join(MCP_BRIDGE_FILE_NAME);
    if !path.is_file() {
        return Err("The packaged HomeServer MCP stdio bridge is not installed.".to_owned());
    }
    Ok(path.to_string_lossy().into_owned())
}
