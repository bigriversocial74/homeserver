use super::{get_json, post_json};
use serde_json::{json, Value};

#[tauri::command]
pub(crate) async fn homeserver_agent_workspace() -> Result<Value, String> {
    let mut workspace: Value = get_json("/v1/agent/workspace").await?;
    let activity = get_json::<Value>("/v1/activity")
        .await
        .unwrap_or_else(|_| json!({
            "last_user_active_at_utc": null,
            "current_session_started_at_utc": null,
            "previous_session_started_at_utc": null,
            "previous_session_stopped_at_utc": null,
            "previous_session_clean": false,
            "recent_events": []
        }));
    if let Some(object) = workspace.as_object_mut() {
        object.insert("activity".to_owned(), activity);
    }
    let _: Result<Value, String> = post_json("/v1/activity/active", &json!({})).await;
    Ok(workspace)
}

#[tauri::command]
pub(crate) async fn homeserver_agent_prompt(request: Value) -> Result<Value, String> {
    post_json("/v1/agent/prompt", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_create_agent_goal(request: Value) -> Result<Value, String> {
    post_json("/v1/agent/goals", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_archive_agent_goal(
    goal_id: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/agent/goals/archive",
        &json!({ "goal_id": goal_id, "confirmation": confirmation }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_create_agent_plan(request: Value) -> Result<Value, String> {
    post_json("/v1/agent/plans", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_cancel_agent_plan(
    plan_id: String,
    confirmation: String,
    reason: Option<String>,
) -> Result<Value, String> {
    post_json(
        "/v1/agent/plans/cancel",
        &json!({ "plan_id": plan_id, "confirmation": confirmation, "reason": reason }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_approve_agent_plan(
    plan_id: String,
    confirmation: String,
    reason: Option<String>,
) -> Result<Value, String> {
    post_json(
        "/v1/agent/approvals/approve",
        &json!({ "plan_id": plan_id, "confirmation": confirmation, "reason": reason }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_reject_agent_plan(
    plan_id: String,
    confirmation: String,
    reason: Option<String>,
) -> Result<Value, String> {
    post_json(
        "/v1/agent/approvals/reject",
        &json!({ "plan_id": plan_id, "confirmation": confirmation, "reason": reason }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_execute_agent_plan(
    plan_id: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/agent/plans/execute",
        &json!({ "plan_id": plan_id, "confirmation": confirmation, "reason": null }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_create_world_mission(request: Value) -> Result<Value, String> {
    post_json("/v1/world/missions", &request).await
}

#[tauri::command]
pub(crate) async fn homeserver_cancel_world_mission(
    mission_id: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/world/missions/cancel",
        &json!({ "mission_id": mission_id, "confirmation": confirmation }),
    )
    .await
}
