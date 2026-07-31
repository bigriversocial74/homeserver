use super::{get_json, post_json};
use serde_json::{json, Value};

const LOCAL_CONTROL_CENTER_ACTOR: &str = "local_control_center";

#[tauri::command]
pub(crate) async fn homeserver_agent_runtime() -> Result<Value, String> {
    get_json("/v1/agent-runtime").await
}

#[tauri::command]
pub(crate) async fn homeserver_agent_authority() -> Result<Value, String> {
    get_json("/v1/agents").await
}

#[tauri::command]
pub(crate) async fn homeserver_run_agent_runtime_once() -> Result<Value, String> {
    post_json("/v1/agent-runtime/run-once", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_cancel_agent_runtime_plan(
    plan_id: String,
    confirmation: String,
    reason: String,
) -> Result<Value, String> {
    post_json(
        "/v1/agent-runtime/plans/cancel",
        &json!({
            "plan_id": plan_id,
            "actor_user_id": LOCAL_CONTROL_CENTER_ACTOR,
            "confirmation": confirmation,
            "reason": reason
        }),
    )
    .await
}
