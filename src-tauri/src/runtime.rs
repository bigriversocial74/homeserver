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
pub(crate) async fn homeserver_action_orchestration() -> Result<Value, String> {
    get_json("/v1/action-orchestration").await
}

#[tauri::command]
pub(crate) async fn homeserver_run_action_orchestration_once() -> Result<Value, String> {
    post_json("/v1/action-orchestration/run-once", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_rollback_supervised_action(
    checkpoint_id: String,
    confirmation: String,
    reason: String,
) -> Result<Value, String> {
    post_json(
        "/v1/action-orchestration/checkpoints/rollback",
        &json!({
            "checkpoint_id": checkpoint_id,
            "confirmation": confirmation,
            "reason": reason
        }),
    )
    .await
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

#[tauri::command]
pub(crate) async fn homeserver_agent_schedules() -> Result<Value, String> {
    get_json("/v1/agent-schedules").await
}

#[tauri::command]
pub(crate) async fn homeserver_run_agent_scheduler_once() -> Result<Value, String> {
    post_json("/v1/agent-schedules/run-once", &json!({})).await
}

async fn mutate_agent_schedule(
    route: &str,
    schedule_id: String,
    confirmation: String,
    reason: String,
) -> Result<Value, String> {
    post_json(
        route,
        &json!({
            "schedule_id": schedule_id,
            "actor_user_id": LOCAL_CONTROL_CENTER_ACTOR,
            "confirmation": confirmation,
            "reason": reason
        }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_pause_agent_schedule(
    schedule_id: String,
    confirmation: String,
    reason: String,
) -> Result<Value, String> {
    mutate_agent_schedule(
        "/v1/agent-schedules/pause",
        schedule_id,
        confirmation,
        reason,
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_resume_agent_schedule(
    schedule_id: String,
    confirmation: String,
    reason: String,
) -> Result<Value, String> {
    mutate_agent_schedule(
        "/v1/agent-schedules/resume",
        schedule_id,
        confirmation,
        reason,
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_cancel_agent_schedule(
    schedule_id: String,
    confirmation: String,
    reason: String,
) -> Result<Value, String> {
    mutate_agent_schedule(
        "/v1/agent-schedules/cancel",
        schedule_id,
        confirmation,
        reason,
    )
    .await
}
