use crate::AppState;
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;
const APPROVAL_MODES: &[&str] = &["always", "per_action", "none"];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRuntimePolicyRequest {
    pub agent_id: String,
    pub actor_user_id: String,
    pub tool_key: String,
    pub approval_mode: String,
    pub max_executions: Option<u32>,
    pub window_seconds: Option<u32>,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RuntimePolicyResponse {
    pub policy_id: String,
    pub agent_id: String,
    pub policy_revision: u64,
    pub action_type: String,
    pub risk_class: String,
    pub approval_mode: String,
    pub tool_adapter: String,
    pub max_executions: u32,
    pub window_seconds: u32,
    pub expires_at_utc: String,
    pub replaced_policy_id: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/v1/agent-runtime/policies/create",
            post(create_policy_handler),
        )
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn create_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateRuntimePolicyRequest>,
) -> ApiResult<RuntimePolicyResponse> {
    tokio::task::spawn_blocking(move || create_policy(&state, request))
        .await
        .map_err(|error| {
            api_error(
                "agent_runtime_policy_task_failed",
                anyhow::anyhow!("runtime policy task failed: {error}"),
            )
        })?
        .map(Json)
        .map_err(|error| api_error("agent_runtime_policy_create_failed", error))
}

fn create_policy(
    state: &AppState,
    request: CreateRuntimePolicyRequest,
) -> Result<RuntimePolicyResponse> {
    ensure!(
        (1..=525_600).contains(&request.expires_minutes),
        "runtime policy expiration must be between one minute and one year"
    );
    let agent_id = validate_uuid(&request.agent_id, "agent ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let tool_key = validate_symbol(&request.tool_key, 120, "tool key")?;
    let approval_mode = validate_symbol(&request.approval_mode, 24, "approval mode")?;
    ensure!(
        APPROVAL_MODES.contains(&approval_mode.as_str()),
        "runtime policy approval mode is invalid"
    );
    let max_executions = request.max_executions.unwrap_or(100);
    ensure!(
        (1..=10_000).contains(&max_executions),
        "runtime policy maximum executions must be between one and 10000"
    );
    let window_seconds = request.window_seconds.unwrap_or(3600);
    ensure!(
        (60..=2_592_000).contains(&window_seconds),
        "runtime policy window must be between 60 seconds and 30 days"
    );

    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let (
        adapter_key,
        risk_class,
        approval_requirement,
        tool_state,
    ): (String, String, String, String) = transaction
        .query_row(
            "SELECT adapter_key,risk_class,approval_requirement,state FROM agent_tool_catalog WHERE tool_key=?1",
            params![tool_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .context("runtime tool was not found")?;
    ensure!(tool_state == "active", "runtime tool is not active");

    let (agent_state, autonomy_level, agent_expires): (String, i64, String) = transaction
        .query_row(
            "SELECT state,autonomy_level,expires_at_utc FROM homeserver_agents WHERE agent_id=?1",
            params![agent_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .context("runtime policy agent was not found")?;
    ensure!(agent_state == "active", "runtime policy agent is not active");
    let required_autonomy = match risk_class.as_str() {
        "read_only" => 1,
        "reversible" => 2,
        "external_side_effect" => 3,
        "high_risk" => 4,
        _ => anyhow::bail!("runtime tool risk class is invalid"),
    };
    ensure!(
        autonomy_level >= required_autonomy,
        "agent autonomy level is below the runtime tool risk class"
    );
    if approval_requirement == "proposal"
        || matches!(risk_class.as_str(), "external_side_effect" | "high_risk")
    {
        ensure!(
            approval_mode != "none",
            "this runtime tool requires an approval-gated policy"
        );
    }

    let now = Utc::now();
    let requested_expires = now + Duration::minutes(i64::from(request.expires_minutes));
    let agent_expires = parse_utc(&agent_expires, "agent expiration")?;
    let expires_at = timestamp(requested_expires.min(agent_expires));
    ensure!(
        parse_utc(&expires_at, "runtime policy expiration")? > now,
        "runtime policy would already be expired"
    );
    let not_before = timestamp(now);
    let replaced_policy_id: Option<String> = transaction
        .query_row(
            "SELECT policy_id FROM agent_execution_policies WHERE agent_id=?1 AND action_type=?2 AND state='active'",
            params![agent_id, tool_key],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(policy_id) = replaced_policy_id.as_deref() {
        transaction.execute(
            "UPDATE agent_execution_policies SET state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE policy_id=?2 AND state='active'",
            params![not_before, policy_id],
        )?;
        transaction.execute(
            "UPDATE agent_action_proposals SET state='cancelled',failure_code='policy_replaced',completed_at_utc=?1,updated_at_utc=?1 WHERE policy_id=?2 AND state IN ('proposed','awaiting_approval','approved','executing')",
            params![not_before, policy_id],
        )?;
        transaction.execute(
            "UPDATE agent_action_approvals SET state='cancelled' WHERE proposal_id IN (SELECT proposal_id FROM agent_action_proposals WHERE policy_id=?1) AND state IN ('pending','approved')",
            params![policy_id],
        )?;
    }
    let policy_revision: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(policy_revision),0)+1 FROM agent_execution_policies WHERE agent_id=?1 AND action_type=?2",
        params![agent_id, tool_key],
        |row| row.get(0),
    )?;
    let policy_id = Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO agent_execution_policies (policy_id,agent_id,policy_revision,action_type,risk_class,approval_mode,tool_adapter,max_executions,window_seconds,state,not_before_utc,expires_at_utc,created_by_user_id,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'active',?10,?11,?12,?10,?10)",
        params![policy_id,agent_id,policy_revision,tool_key,risk_class,approval_mode,adapter_key,i64::from(max_executions),i64::from(window_seconds),not_before,expires_at,actor],
    )?;
    let event_id = Uuid::new_v4().to_string();
    let event_metadata = json!({
        "policy_id": policy_id,
        "policy_revision": policy_revision,
        "tool_key": tool_key,
        "adapter_key": adapter_key,
        "risk_class": risk_class,
        "approval_mode": approval_mode,
        "replaced_policy_id": replaced_policy_id
    });
    let event_hash = hash_json(&json!({
        "event_id": event_id,
        "agent_id": agent_id,
        "event_type": "agent.runtime_policy_created",
        "actor_id": actor,
        "metadata": event_metadata,
        "created_at_utc": not_before
    }))?;
    transaction.execute(
        "INSERT INTO agent_runtime_events (event_id,agent_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,'agent.runtime_policy_created','success','local_user',?3,'catalog_bound_policy_active',?4,?5,?6)",
        params![event_id,agent_id,actor,serde_json::to_string(&event_metadata)?,event_hash,not_before],
    )?;
    transaction.commit()?;
    super::wrapper_jobs::reconcile_authority(&connection)?;

    Ok(RuntimePolicyResponse {
        policy_id,
        agent_id,
        policy_revision: policy_revision.max(1) as u64,
        action_type: tool_key,
        risk_class,
        approval_mode,
        tool_adapter: adapter_key,
        max_executions,
        window_seconds,
        expires_at_utc: expires_at,
        replaced_policy_id,
    })
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    Uuid::parse_str(value).with_context(|| format!("{label} is invalid"))?;
    Ok(value.to_owned())
}

fn validate_symbol(value: &str, max: usize, label: &str) -> Result<String> {
    let value = bounded_text(value, 1, max, label)?;
    ensure!(
        value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':')),
        "{label} contains unsupported characters"
    );
    Ok(value)
}

fn bounded_text(value: &str, min: usize, max: usize, label: &str) -> Result<String> {
    let trimmed = value.trim();
    ensure!(
        (min..=max).contains(&trimmed.len()),
        "{label} must contain between {min} and {max} characters"
    );
    ensure!(
        !trimmed.chars().any(char::is_control),
        "{label} contains control characters"
    );
    Ok(trimmed.to_owned())
}

fn parse_utc(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{label} is invalid"))
        .map(|value| value.with_timezone(&Utc))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn hash_json(value: &Value) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value)?)
    ))
}

fn api_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}
