use crate::{cloud_registry, model_center, semantic_vault, AppState};
use anyhow::{anyhow, bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use microgifter_homeserver_core::{
    api_base_url, BackupKind, CreateBackupRequest, LOCAL_CLIENT_HEADER, LOCAL_CLIENT_VALUE,
};
use reqwest::redirect::Policy;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Arc, time::Duration};
use uuid::Uuid;

const AGENT_MIGRATION: &str =
    include_str!("../../../database/migrations/0011_supervised_agent_workspace.sql");
const AGENT_MIGRATION_KEY: &str = "0011_supervised_agent_workspace";
const MAX_CONTROL_BODY_BYTES: usize = 128 * 1024;
const MAX_LOCAL_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROMPT_CHARS: usize = 4_000;
const MAX_MESSAGE_CHARS: usize = 20_000;
const MAX_REPORT_CHARS: usize = 30_000;
const MAX_CONTEXT_ITEMS: usize = 20;
const MAX_HISTORY_ROWS: i64 = 500;
const MAX_WORLD_LIMIT_BYTES: usize = 8 * 1024;
const LOCAL_ACTOR_ID: &str = "local_control_center";
const ALLOWED_MODES: &[&str] = &["ask", "analyze", "plan", "dispatch", "execute"];
const ALLOWED_DATASET_KEYS: &[&str] = &[
    "system",
    "connections",
    "knowledge",
    "models",
    "goals",
    "operational_data",
    "world_canvas",
];
const ALLOWED_ACTION_TYPES: &[&str] = &[
    "backup.create",
    "model.health_test",
    "cloud.sync_connection",
    "cloud.sync_all",
    "report.save",
];
const ALLOWED_WORLD_OPERATIONS: &[&str] = &[
    "discover",
    "visit_store_canvas",
    "ask_questions",
    "compare",
    "request_information",
    "prepare_recommendation",
    "schedule_follow_up",
    "close_conversation",
];
const PROHIBITED_WORLD_OPERATIONS: &[&str] = &[
    "purchase",
    "payment",
    "claim",
    "redemption",
    "share_private_profile",
    "accept_recurring_commitment",
    "publish_campaign",
    "bulk_message",
];

const GOAL_COLUMNS: &str = "goal_id,title,description,target_metric,target_value,target_date,connection_ids_json,dataset_keys_json,constraints_json,allowed_actions_json,approval_policy,state,created_at_utc,updated_at_utc";
const THREAD_COLUMNS: &str = "thread_id,title,state,created_at_utc,updated_at_utc";
const MESSAGE_COLUMNS: &str = "message_id,thread_id,role,mode,content,context_json,created_at_utc";
const PLAN_COLUMNS: &str = "plan_id,thread_id,goal_id,requested_by_type,requested_by_id,title,rationale,action_type,arguments_json,connection_id,dataset_keys_json,risk_level,state,plan_hash,fresh_state_token,expires_at_utc,failure_code,created_at_utc,updated_at_utc,completed_at_utc";
const APPROVAL_COLUMNS: &str = "r.approval_request_id,r.plan_id,r.plan_hash,r.state,r.risk_summary,r.requested_at_utc,r.expires_at_utc,r.decided_at_utc,r.decision_reason,a.approval_id,a.approved_by,a.approved_at_utc,a.consumed_at_utc";
const RECEIPT_COLUMNS: &str = "receipt_id,plan_id,approval_id,plan_hash,action_type,connection_id,idempotency_key,state,result_code,result_summary,result_json,started_at_utc,completed_at_utc";
const REPORT_COLUMNS: &str = "report_id,plan_id,title,content_markdown,connection_ids_json,dataset_keys_json,created_at_utc";
const MISSION_COLUMNS: &str = "mission_id,thread_id,goal_id,connection_id,world_agent_id,title,objective,allowed_operations_json,prohibited_operations_json,limits_json,disclosure_policy_json,state,expires_at_utc,created_at_utc,updated_at_utc";

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentGoalSummary {
    pub goal_id: String,
    pub title: String,
    pub description: String,
    pub target_metric: Option<String>,
    pub target_value: Option<String>,
    pub target_date: Option<String>,
    pub connection_ids: Vec<String>,
    pub dataset_keys: Vec<String>,
    pub constraints: Value,
    pub allowed_actions: Vec<String>,
    pub approval_policy: String,
    pub state: String,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentThreadSummary {
    pub thread_id: String,
    pub title: String,
    pub state: String,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMessageSummary {
    pub message_id: String,
    pub thread_id: String,
    pub role: String,
    pub mode: String,
    pub content: String,
    pub context: Value,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPlanSummary {
    pub plan_id: String,
    pub thread_id: Option<String>,
    pub goal_id: Option<String>,
    pub requested_by_type: String,
    pub requested_by_id: String,
    pub title: String,
    pub rationale: String,
    pub action_type: String,
    pub arguments: Value,
    pub connection_id: Option<String>,
    pub dataset_keys: Vec<String>,
    pub risk_level: String,
    pub state: String,
    pub plan_hash: String,
    pub fresh_state_token: String,
    pub expires_at_utc: String,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentApprovalSummary {
    pub approval_request_id: String,
    pub plan_id: String,
    pub plan_hash: String,
    pub state: String,
    pub risk_summary: String,
    pub requested_at_utc: String,
    pub expires_at_utc: String,
    pub decided_at_utc: Option<String>,
    pub decision_reason: Option<String>,
    pub approval_id: Option<String>,
    pub approved_by: Option<String>,
    pub approved_at_utc: Option<String>,
    pub consumed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentExecutionReceiptSummary {
    pub receipt_id: String,
    pub plan_id: String,
    pub approval_id: String,
    pub plan_hash: String,
    pub action_type: String,
    pub connection_id: Option<String>,
    pub idempotency_key: String,
    pub state: String,
    pub result_code: String,
    pub result_summary: String,
    pub result: Value,
    pub started_at_utc: String,
    pub completed_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentReportSummary {
    pub report_id: String,
    pub plan_id: Option<String>,
    pub title: String,
    pub content_markdown: String,
    pub connection_ids: Vec<String>,
    pub dataset_keys: Vec<String>,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorldMissionSummary {
    pub mission_id: String,
    pub thread_id: Option<String>,
    pub goal_id: Option<String>,
    pub connection_id: Option<String>,
    pub world_agent_id: String,
    pub title: String,
    pub objective: String,
    pub allowed_operations: Vec<String>,
    pub prohibited_operations: Vec<String>,
    pub limits: Value,
    pub disclosure_policy: Value,
    pub state: String,
    pub expires_at_utc: String,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentDataSourceSummary {
    pub key: String,
    pub label: String,
    pub state: String,
    pub detail: String,
    pub last_updated_utc: Option<String>,
    pub connection_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentWorkspaceSnapshot {
    pub goals: Vec<AgentGoalSummary>,
    pub threads: Vec<AgentThreadSummary>,
    pub messages: Vec<AgentMessageSummary>,
    pub plans: Vec<AgentPlanSummary>,
    pub approvals: Vec<AgentApprovalSummary>,
    pub receipts: Vec<AgentExecutionReceiptSummary>,
    pub reports: Vec<AgentReportSummary>,
    pub missions: Vec<WorldMissionSummary>,
    pub data_sources: Vec<AgentDataSourceSummary>,
    pub connections: Vec<cloud_registry::CloudConnectionSummary>,
    pub model_runtime_state: String,
    pub default_chat_model: Option<String>,
    pub local_only: bool,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateGoalRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub target_metric: Option<String>,
    pub target_value: Option<String>,
    pub target_date: Option<String>,
    #[serde(default)]
    pub connection_ids: Vec<String>,
    #[serde(default)]
    pub dataset_keys: Vec<String>,
    #[serde(default)]
    pub constraints: Value,
    #[serde(default)]
    pub allowed_actions: Vec<String>,
    pub approval_policy: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArchiveGoalRequest {
    pub goal_id: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProposedActionRequest {
    pub title: String,
    pub rationale: String,
    pub action_type: String,
    #[serde(default)]
    pub arguments: Value,
    pub connection_id: Option<String>,
    pub goal_id: Option<String>,
    #[serde(default)]
    pub dataset_keys: Vec<String>,
    pub expires_minutes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePlanRequest {
    pub thread_id: Option<String>,
    pub title: String,
    pub rationale: String,
    pub action_type: String,
    #[serde(default)]
    pub arguments: Value,
    pub connection_id: Option<String>,
    pub goal_id: Option<String>,
    #[serde(default)]
    pub dataset_keys: Vec<String>,
    pub expires_minutes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlanReferenceRequest {
    pub plan_id: String,
    pub confirmation: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateWorldMissionRequest {
    pub thread_id: Option<String>,
    pub goal_id: Option<String>,
    pub connection_id: Option<String>,
    pub world_agent_id: String,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub allowed_operations: Vec<String>,
    #[serde(default)]
    pub prohibited_operations: Vec<String>,
    #[serde(default)]
    pub limits: Value,
    #[serde(default)]
    pub disclosure_policy: Value,
    pub expires_minutes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MissionReferenceRequest {
    pub mission_id: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentPromptRequest {
    pub thread_id: Option<String>,
    pub mode: String,
    pub prompt: String,
    #[serde(default)]
    pub connection_ids: Vec<String>,
    #[serde(default)]
    pub dataset_keys: Vec<String>,
    #[serde(default)]
    pub goal_ids: Vec<String>,
    pub knowledge_query: Option<String>,
    pub model: Option<String>,
    pub proposed_action: Option<ProposedActionRequest>,
    pub world_mission: Option<CreateWorldMissionRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPromptResponse {
    pub thread_id: String,
    pub user_message_id: String,
    pub assistant_message: AgentMessageSummary,
    pub grounding: Value,
    pub plan: Option<AgentPlanSummary>,
    pub mission: Option<WorldMissionSummary>,
    pub approvals_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalDecisionResult {
    pub plan: AgentPlanSummary,
    pub approval: AgentApprovalSummary,
    pub message: String,
}

#[derive(Debug, Clone)]
struct WorkspaceLocalSnapshot {
    goals: Vec<AgentGoalSummary>,
    threads: Vec<AgentThreadSummary>,
    messages: Vec<AgentMessageSummary>,
    plans: Vec<AgentPlanSummary>,
    approvals: Vec<AgentApprovalSummary>,
    receipts: Vec<AgentExecutionReceiptSummary>,
    reports: Vec<AgentReportSummary>,
    missions: Vec<WorldMissionSummary>,
}

#[derive(Debug, Clone)]
struct ApprovalRecord {
    approval_id: String,
    approval_request_id: String,
    plan_id: String,
    plan_hash: String,
    expires_at_utc: String,
    consumed_at_utc: Option<String>,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(AGENT_MIGRATION)?;
    connection.execute(
        "UPDATE agent_plans SET state='failed',failure_code='service_restarted',completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='executing'",
        [],
    )?;
    connection.execute(
        "UPDATE agent_action_idempotency SET state='failed',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='executing'",
        [],
    )?;
    expire_pending(connection)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![AGENT_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "supervised Agent Workspace migration is not registered exactly once"
    );
    for table in [
        "agent_goals",
        "agent_threads",
        "agent_messages",
        "agent_plans",
        "agent_plan_steps",
        "agent_approval_requests",
        "agent_approvals",
        "agent_action_idempotency",
        "agent_execution_receipts",
        "agent_reports",
        "world_missions",
        "world_tasks",
        "world_conversations",
        "world_conversation_commitments",
        "world_follow_ups",
        "world_mission_events",
        "world_receipts",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    expire_pending(connection)?;
    connection.execute(
        "DELETE FROM agent_messages WHERE message_id NOT IN (SELECT message_id FROM agent_messages ORDER BY created_at_utc DESC,message_id DESC LIMIT ?1)",
        params![MAX_HISTORY_ROWS],
    )?;
    connection.execute(
        "DELETE FROM world_mission_events WHERE event_id NOT IN (SELECT event_id FROM world_mission_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1)",
        params![MAX_HISTORY_ROWS],
    )?;
    connection.execute(
        "DELETE FROM agent_execution_receipts WHERE completed_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/agent/workspace", get(workspace))
        .route("/v1/agent/goals", post(create_goal))
        .route("/v1/agent/goals/archive", post(archive_goal))
        .route("/v1/agent/prompt", post(prompt))
        .route("/v1/agent/plans", post(create_plan))
        .route("/v1/agent/plans/cancel", post(cancel_plan))
        .route("/v1/agent/approvals/approve", post(approve_plan))
        .route("/v1/agent/approvals/reject", post(reject_plan))
        .route("/v1/agent/plans/execute", post(execute_plan))
        .route("/v1/world/missions", post(create_world_mission))
        .route("/v1/world/missions/cancel", post(cancel_world_mission))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn workspace(State(state): State<Arc<AppState>>) -> ApiResult<AgentWorkspaceSnapshot> {
    workspace_snapshot(state)
        .await
        .map(Json)
        .map_err(|error| internal_error("agent_workspace_failed", error))
}

async fn create_goal(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateGoalRequest>,
) -> ApiResult<AgentGoalSummary> {
    tokio::task::spawn_blocking(move || save_goal(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("agent_goal_rejected", error))
}

async fn archive_goal(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ArchiveGoalRequest>,
) -> ApiResult<AgentGoalSummary> {
    tokio::task::spawn_blocking(move || archive_goal_record(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("agent_goal_archive_rejected", error))
}

async fn prompt(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AgentPromptRequest>,
) -> ApiResult<AgentPromptResponse> {
    handle_prompt(state, request, "local_user", LOCAL_ACTOR_ID)
        .await
        .map(Json)
        .map_err(|error| action_error("agent_prompt_rejected", error))
}

async fn create_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreatePlanRequest>,
) -> ApiResult<AgentPlanSummary> {
    tokio::task::spawn_blocking(move || {
        save_plan(&state, request, "local_user", LOCAL_ACTOR_ID)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| action_error("agent_plan_rejected", error))
}

async fn cancel_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PlanReferenceRequest>,
) -> ApiResult<AgentPlanSummary> {
    tokio::task::spawn_blocking(move || {
        cancel_plan_record(&state, request, None, true)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| action_error("agent_plan_cancel_rejected", error))
}

async fn approve_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PlanReferenceRequest>,
) -> ApiResult<ApprovalDecisionResult> {
    tokio::task::spawn_blocking(move || approve_plan_record(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("agent_approval_rejected", error))
}

async fn reject_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PlanReferenceRequest>,
) -> ApiResult<ApprovalDecisionResult> {
    tokio::task::spawn_blocking(move || reject_plan_record(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("agent_rejection_rejected", error))
}

async fn execute_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PlanReferenceRequest>,
) -> ApiResult<AgentExecutionReceiptSummary> {
    execute_approved_plan(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("agent_execution_rejected", error))
}

async fn create_world_mission(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateWorldMissionRequest>,
) -> ApiResult<WorldMissionSummary> {
    tokio::task::spawn_blocking(move || {
        save_world_mission(&state, request, "local_user", LOCAL_ACTOR_ID)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| action_error("world_mission_rejected", error))
}

async fn cancel_world_mission(
    State(state): State<Arc<AppState>>,
    Json(request): Json<MissionReferenceRequest>,
) -> ApiResult<WorldMissionSummary> {
    tokio::task::spawn_blocking(move || cancel_mission_record(&state, request, None))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("world_mission_cancel_rejected", error))
}

pub(crate) async fn workspace_snapshot(state: Arc<AppState>) -> Result<AgentWorkspaceSnapshot> {
    let local_state = state.clone();
    let local = tokio::task::spawn_blocking(move || read_workspace_local(&local_state))
        .await
        .context("Agent Workspace database task failed")??;
    let cloud_state = state.clone();
    let clouds = tokio::task::spawn_blocking(move || cloud_state.cloud_connections_snapshot())
        .await
        .context("Agent Workspace cloud registry task failed")??;
    let models = model_center::snapshot(state.clone()).await.ok();
    let model_runtime_state = models
        .as_ref()
        .map(|snapshot| snapshot.runtime.state.clone())
        .unwrap_or_else(|| "unavailable".to_owned());
    let default_chat_model = models
        .as_ref()
        .and_then(|snapshot| snapshot.settings.default_chat_model.clone());
    let data_sources = build_data_sources(&clouds, &local, models.as_ref());
    Ok(AgentWorkspaceSnapshot {
        goals: local.goals,
        threads: local.threads,
        messages: local.messages,
        plans: local.plans,
        approvals: local.approvals,
        receipts: local.receipts,
        reports: local.reports,
        missions: local.missions,
        data_sources,
        connections: clouds.connections,
        model_runtime_state,
        default_chat_model,
        local_only: clouds.local_only,
        capabilities: vec![
            "ask".to_owned(),
            "analyze".to_owned(),
            "plan".to_owned(),
            "dispatch_draft".to_owned(),
            "approval_gated_execute".to_owned(),
        ],
    })
}

pub(crate) async fn mcp_prompt(
    state: Arc<AppState>,
    client_id: &str,
    arguments: Value,
) -> Result<Value> {
    let request = serde_json::from_value::<AgentPromptRequest>(arguments)
        .context("invalid HomeServer agent prompt arguments")?;
    let response = handle_prompt(state, request, "mcp_client", client_id).await?;
    serde_json::to_value(response).map_err(Into::into)
}

pub(crate) fn mcp_submit_plan(
    state: &AppState,
    client_id: &str,
    arguments: Value,
) -> Result<Value> {
    let request = serde_json::from_value::<CreatePlanRequest>(arguments)
        .context("invalid supervised plan arguments")?;
    serde_json::to_value(save_plan(state, request, "mcp_client", client_id)?)
        .map_err(Into::into)
}

pub(crate) fn mcp_get_plan(state: &AppState, plan_id: &str, client_id: &str) -> Result<Value> {
    let connection = state.connection()?;
    let plan = plan_by_id(&connection, plan_id)?;
    ensure_mcp_plan_access(&plan, client_id)?;
    serde_json::to_value(plan).map_err(Into::into)
}

pub(crate) fn mcp_list_plans(state: &AppState, client_id: &str) -> Result<Value> {
    let connection = state.connection()?;
    let mut statement = connection.prepare(&format!(
        "SELECT {PLAN_COLUMNS} FROM agent_plans WHERE requested_by_type='mcp_client' AND requested_by_id=?1 ORDER BY created_at_utc DESC,plan_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map(params![client_id], map_plan)?;
    let plans = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(anyhow::Error::from)?;
    Ok(json!({ "plans": plans }))
}

pub(crate) fn mcp_cancel_plan(
    state: &AppState,
    client_id: &str,
    plan_id: &str,
) -> Result<Value> {
    let plan = cancel_plan_record(
        state,
        PlanReferenceRequest {
            plan_id: plan_id.to_owned(),
            confirmation: "CANCEL".to_owned(),
            reason: Some("Cancelled by the requesting MCP client".to_owned()),
        },
        Some(client_id),
        false,
    )?;
    serde_json::to_value(plan).map_err(Into::into)
}

pub(crate) fn mcp_draft_world_mission(
    state: &AppState,
    client_id: &str,
    arguments: Value,
) -> Result<Value> {
    let request = serde_json::from_value::<CreateWorldMissionRequest>(arguments)
        .context("invalid World Mission arguments")?;
    serde_json::to_value(save_world_mission(
        state,
        request,
        "mcp_client",
        client_id,
    )?)
    .map_err(Into::into)
}

pub(crate) fn mcp_get_world_mission(
    state: &AppState,
    mission_id: &str,
) -> Result<Value> {
    let connection = state.connection()?;
    serde_json::to_value(mission_by_id(&connection, mission_id)?).map_err(Into::into)
}

pub(crate) fn mcp_list_receipts(state: &AppState, client_id: &str) -> Result<Value> {
    let connection = state.connection()?;
    let mut statement = connection.prepare(&format!(
        "SELECT {RECEIPT_COLUMNS} FROM agent_execution_receipts WHERE plan_id IN (SELECT plan_id FROM agent_plans WHERE requested_by_type='mcp_client' AND requested_by_id=?1) ORDER BY completed_at_utc DESC,receipt_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map(params![client_id], map_receipt)?;
    let receipts = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(anyhow::Error::from)?;
    Ok(json!({ "receipts": receipts }))
}

async fn handle_prompt(
    state: Arc<AppState>,
    mut request: AgentPromptRequest,
    actor_type: &str,
    actor_id: &str,
) -> Result<AgentPromptResponse> {
    request.mode = normalize_mode(&request.mode)?;
    request.prompt = sanitize_required_text(&request.prompt, MAX_PROMPT_CHARS, "prompt")?;
    request.connection_ids = normalize_ids(&request.connection_ids, MAX_CONTEXT_ITEMS, "connection")?;
    request.dataset_keys = normalize_dataset_keys(&request.dataset_keys)?;
    request.goal_ids = normalize_ids(&request.goal_ids, MAX_CONTEXT_ITEMS, "goal")?;
    let thread_id = ensure_thread(&state, request.thread_id.as_deref(), &request.prompt)?;
    let user_message_id = save_message(
        &state,
        &thread_id,
        "user",
        &request.mode,
        &request.prompt,
        &json!({
            "connection_ids": request.connection_ids,
            "dataset_keys": request.dataset_keys,
            "goal_ids": request.goal_ids,
            "actor_type": actor_type,
            "actor_id": actor_id,
        }),
    )?;

    let cloud_snapshot = {
        let cloud_state = state.clone();
        tokio::task::spawn_blocking(move || cloud_state.cloud_connections_snapshot())
            .await
            .context("cloud context task failed")??
    };
    let selected_connections = select_connections(&cloud_snapshot, &request.connection_ids)?;
    let selected_goals = {
        let goal_state = state.clone();
        let goal_ids = request.goal_ids.clone();
        tokio::task::spawn_blocking(move || goals_by_ids(&goal_state, &goal_ids))
            .await
            .context("goal context task failed")??
    };
    let knowledge = if request.dataset_keys.iter().any(|key| key == "knowledge") {
        let query = request
            .knowledge_query
            .as_deref()
            .unwrap_or(&request.prompt)
            .trim();
        if query.is_empty() {
            None
        } else {
            semantic_vault::semantic_search(
                state.clone(),
                semantic_vault::SemanticSearchRequest {
                    query: truncate_chars(query, 200),
                    limit: Some(5),
                    mode: Some("hybrid".to_owned()),
                },
            )
            .await
            .ok()
        }
    } else {
        None
    };
    let models = model_center::snapshot(state.clone()).await.ok();
    let grounding = json!({
        "mode": request.mode,
        "connections": selected_connections,
        "goals": selected_goals,
        "datasets": request.dataset_keys,
        "knowledge_hits": knowledge.as_ref().map(|result| &result.hits),
        "operational_data_state": "provider_import_not_enabled_until_phase_5c",
        "world_canvas_state": "mission_drafting_only",
        "model_runtime": models.as_ref().map(|snapshot| &snapshot.runtime.state),
    });

    let plan = if let Some(action) = request.proposed_action.clone() {
        let plan_request = CreatePlanRequest {
            thread_id: Some(thread_id.clone()),
            title: action.title,
            rationale: action.rationale,
            action_type: action.action_type,
            arguments: action.arguments,
            connection_id: action.connection_id,
            goal_id: action.goal_id,
            dataset_keys: action.dataset_keys,
            expires_minutes: action.expires_minutes,
        };
        let plan_state = state.clone();
        let actor_type = actor_type.to_owned();
        let actor_id = actor_id.to_owned();
        Some(
            tokio::task::spawn_blocking(move || {
                save_plan(&plan_state, plan_request, &actor_type, &actor_id)
            })
            .await
            .context("plan draft task failed")??,
        )
    } else {
        None
    };

    let mission = if let Some(mut mission_request) = request.world_mission.clone() {
        mission_request.thread_id = Some(thread_id.clone());
        let mission_state = state.clone();
        let actor_type = actor_type.to_owned();
        let actor_id = actor_id.to_owned();
        Some(
            tokio::task::spawn_blocking(move || {
                save_world_mission(&mission_state, mission_request, &actor_type, &actor_id)
            })
            .await
            .context("World Mission draft task failed")??,
        )
    } else {
        None
    };

    let assistant_text = generate_grounded_response(
        state.clone(),
        &request,
        &selected_connections,
        &selected_goals,
        knowledge.as_ref(),
        models.as_ref(),
        plan.as_ref(),
        mission.as_ref(),
    )
    .await?;
    let assistant_message_id = save_message(
        &state,
        &thread_id,
        "assistant",
        &request.mode,
        &assistant_text,
        &grounding,
    )?;
    let assistant_message = {
        let connection = state.connection()?;
        message_by_id(&connection, &assistant_message_id)?
    };
    Ok(AgentPromptResponse {
        thread_id,
        user_message_id,
        assistant_message,
        grounding,
        approvals_required: plan.is_some(),
        plan,
        mission,
    })
}

async fn generate_grounded_response(
    state: Arc<AppState>,
    request: &AgentPromptRequest,
    connections: &[cloud_registry::CloudConnectionSummary],
    goals: &[AgentGoalSummary],
    knowledge: Option<&semantic_vault::SemanticSearchResult>,
    models: Option<&model_center::ModelCenterSnapshot>,
    plan: Option<&AgentPlanSummary>,
    mission: Option<&WorldMissionSummary>,
) -> Result<String> {
    let context_line = format!(
        "Mode: {}. Connected sites selected: {}. Goals selected: {}. Knowledge hits: {}. Operational platform imports: not enabled until Phase 5C. World Mode: mission drafting only.",
        request.mode,
        connections.len(),
        goals.len(),
        knowledge.map(|result| result.hits.len()).unwrap_or(0)
    );
    let safety_line = if let Some(plan) = plan {
        format!(
            " A supervised {} plan was created and is awaiting one-use local approval; it has not executed.",
            plan.action_type
        )
    } else if mission.is_some() {
        " A World Mission draft was saved locally; no World Agent was dispatched.".to_owned()
    } else {
        " No external action was requested or executed.".to_owned()
    };

    if let Some(snapshot) = models {
        if snapshot.runtime.state == "running" {
            let selected_model = request
                .model
                .as_ref()
                .filter(|candidate| {
                    snapshot
                        .installed_models
                        .iter()
                        .any(|model| model.name == ***candidate)
                })
                .cloned()
                .or_else(|| snapshot.settings.default_chat_model.clone())
                .or_else(|| snapshot.installed_models.first().map(|model| model.name.clone()));
            if let Some(model) = selected_model {
                let compact_prompt = truncate_chars(
                    &format!(
                        "You are the private Microgifter HomeServer operational agent. Answer concisely and never claim unavailable data. User request: {} Context: {}{}",
                        request.prompt, context_line, safety_line
                    ),
                    500,
                );
                if let Ok(result) = local_post_json::<_, model_center::ModelTestResult>(
                    "/v1/models/test",
                    &json!({ "model": model, "prompt": compact_prompt }),
                )
                .await
                {
                    return Ok(truncate_chars(result.output.trim(), MAX_MESSAGE_CHARS));
                }
            }
        }
    }

    Ok(format!(
        "{}{}\n\nHomeServer can use current system, connection, model, goal, and Knowledge Vault context now. Provider operational datasets will become available through the Phase 5C import and incremental-sync layer.",
        context_line, safety_line
    ))
}

fn save_goal(state: &AppState, request: CreateGoalRequest) -> Result<AgentGoalSummary> {
    let title = sanitize_required_text(&request.title, 160, "goal title")?;
    let description = sanitize_optional_text(Some(&request.description), 4000, "goal description")?
        .unwrap_or_default();
    let target_metric = sanitize_optional_text(request.target_metric.as_deref(), 160, "target metric")?;
    let target_value = sanitize_optional_text(request.target_value.as_deref(), 160, "target value")?;
    let target_date = sanitize_optional_text(request.target_date.as_deref(), 40, "target date")?;
    let connection_ids = normalize_ids(&request.connection_ids, MAX_CONTEXT_ITEMS, "connection")?;
    let dataset_keys = normalize_dataset_keys(&request.dataset_keys)?;
    ensure_json_object(&request.constraints, MAX_WORLD_LIMIT_BYTES, "goal constraints")?;
    let allowed_actions = normalize_action_list(&request.allowed_actions)?;
    let approval_policy = request
        .approval_policy
        .unwrap_or_else(|| "always".to_owned())
        .trim()
        .to_ascii_lowercase();
    ensure!(
        ["always", "read_only", "disabled"].contains(&approval_policy.as_str()),
        "goal approval policy is invalid"
    );
    let connection = state.connection()?;
    validate_connection_ids(&connection, &connection_ids)?;
    let goal_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO agent_goals (goal_id,title,description,target_metric,target_value,target_date,connection_ids_json,dataset_keys_json,constraints_json,allowed_actions_json,approval_policy,state) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'active')",
        params![
            goal_id,
            title,
            description,
            target_metric,
            target_value,
            target_date,
            serde_json::to_string(&connection_ids)?,
            serde_json::to_string(&dataset_keys)?,
            serde_json::to_string(&request.constraints)?,
            serde_json::to_string(&allowed_actions)?,
            approval_policy,
        ],
    )?;
    goal_by_id(&connection, &goal_id)
}

fn archive_goal_record(state: &AppState, request: ArchiveGoalRequest) -> Result<AgentGoalSummary> {
    ensure!(request.confirmation == "ARCHIVE", "type ARCHIVE to archive the goal");
    validate_uuid(&request.goal_id, "goal id")?;
    let connection = state.connection()?;
    let affected = connection.execute(
        "UPDATE agent_goals SET state='archived',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE goal_id=?1 AND state!='archived'",
        params![request.goal_id],
    )?;
    ensure!(affected == 1, "active goal was not found");
    goal_by_id(&connection, &request.goal_id)
}

fn save_plan(
    state: &AppState,
    request: CreatePlanRequest,
    actor_type: &str,
    actor_id: &str,
) -> Result<AgentPlanSummary> {
    ensure!(["local_user", "mcp_client", "system"].contains(&actor_type), "plan actor type is invalid");
    let actor_id = sanitize_required_text(actor_id, 160, "plan actor")?;
    let title = sanitize_required_text(&request.title, 180, "plan title")?;
    let rationale = sanitize_required_text(&request.rationale, 4000, "plan rationale")?;
    let action_type = request.action_type.trim().to_ascii_lowercase();
    ensure!(
        ALLOWED_ACTION_TYPES.contains(&action_type.as_str()),
        "action type is not enabled for supervised execution"
    );
    let connection_id = normalize_optional_uuid(request.connection_id.as_deref(), "connection id")?;
    let goal_id = normalize_optional_uuid(request.goal_id.as_deref(), "goal id")?;
    let thread_id = normalize_optional_uuid(request.thread_id.as_deref(), "thread id")?;
    let dataset_keys = normalize_dataset_keys(&request.dataset_keys)?;
    let arguments = validate_action_arguments(&action_type, request.arguments)?;
    let expires_minutes = request.expires_minutes.unwrap_or(30).clamp(5, 24 * 60);
    let now = Utc::now();
    let expires_at_utc = (now + ChronoDuration::minutes(i64::from(expires_minutes)))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let created_at_utc = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let connection = state.connection()?;
    if let Some(connection_id) = connection_id.as_deref() {
        validate_connection_ids(&connection, &[connection_id.to_owned()])?;
    }
    if let Some(goal_id) = goal_id.as_deref() {
        let _ = goal_by_id(&connection, goal_id)?;
    }
    if let Some(thread_id) = thread_id.as_deref() {
        let _ = thread_by_id(&connection, thread_id)?;
    }
    validate_action_target(&connection, &action_type, connection_id.as_deref())?;
    let fresh_state_token = fresh_state_token(&connection, &action_type, connection_id.as_deref())?;
    let plan_id = Uuid::new_v4().to_string();
    let canonical = json!({
        "schema": 1,
        "plan_id": plan_id,
        "requested_by_type": actor_type,
        "requested_by_id": actor_id,
        "title": title,
        "rationale": rationale,
        "action_type": action_type,
        "arguments": arguments,
        "connection_id": connection_id,
        "goal_id": goal_id,
        "dataset_keys": dataset_keys,
        "fresh_state_token": fresh_state_token,
        "expires_at_utc": expires_at_utc,
        "created_at_utc": created_at_utc,
    });
    let plan_hash = hash_json(&canonical)?;
    let risk_level = action_risk(&action_type).to_owned();
    let approval_request_id = Uuid::new_v4().to_string();
    let risk_summary = risk_summary(&action_type, connection_id.as_deref());
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO agent_plans (plan_id,thread_id,goal_id,requested_by_type,requested_by_id,title,rationale,action_type,arguments_json,connection_id,dataset_keys_json,risk_level,state,plan_hash,fresh_state_token,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'awaiting_approval',?13,?14,?15,?16,?16)",
        params![
            plan_id,
            thread_id,
            goal_id,
            actor_type,
            actor_id,
            title,
            rationale,
            action_type,
            serde_json::to_string(&arguments)?,
            connection_id,
            serde_json::to_string(&dataset_keys)?,
            risk_level,
            plan_hash,
            fresh_state_token,
            expires_at_utc,
            created_at_utc,
        ],
    )?;
    transaction.execute(
        "INSERT INTO agent_plan_steps (step_id,plan_id,step_index,title,action_type,arguments_json,state) VALUES (?1,?2,0,?3,?4,?5,'pending')",
        params![
            Uuid::new_v4().to_string(),
            plan_id,
            title,
            action_type,
            serde_json::to_string(&arguments)?,
        ],
    )?;
    transaction.execute(
        "INSERT INTO agent_approval_requests (approval_request_id,plan_id,plan_hash,state,risk_summary,requested_at_utc,expires_at_utc) VALUES (?1,?2,?3,'pending',?4,?5,?6)",
        params![approval_request_id, plan_id, plan_hash, risk_summary, created_at_utc, expires_at_utc],
    )?;
    transaction.commit()?;
    plan_by_id(&connection, &plan_id)
}

fn approve_plan_record(state: &AppState, request: PlanReferenceRequest) -> Result<ApprovalDecisionResult> {
    ensure!(request.confirmation == "APPROVE", "type APPROVE to approve this plan");
    validate_uuid(&request.plan_id, "plan id")?;
    let mut connection = state.connection()?;
    expire_pending(&connection)?;
    let plan = plan_by_id(&connection, &request.plan_id)?;
    ensure!(plan.state == "awaiting_approval", "plan is not awaiting approval");
    let approval_request = approval_by_plan(&connection, &plan.plan_id)?;
    ensure!(approval_request.state == "pending", "approval request is not pending");
    ensure!(approval_request.plan_hash == plan.plan_hash, "approval hash does not match the plan");
    ensure!(!is_expired(&plan.expires_at_utc), "plan approval has expired");
    let approval_id = Uuid::new_v4().to_string();
    let now = now_string();
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO agent_approvals (approval_id,approval_request_id,plan_id,plan_hash,approved_by,approved_at_utc,expires_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![approval_id, approval_request.approval_request_id, plan.plan_id, plan.plan_hash, LOCAL_ACTOR_ID, now, plan.expires_at_utc],
    )?;
    transaction.execute(
        "UPDATE agent_approval_requests SET state='approved',decided_at_utc=?2,decision_reason=?3 WHERE plan_id=?1 AND state='pending'",
        params![plan.plan_id, now, request.reason],
    )?;
    transaction.execute(
        "UPDATE agent_plans SET state='approved',updated_at_utc=?2 WHERE plan_id=?1 AND state='awaiting_approval'",
        params![plan.plan_id, now],
    )?;
    transaction.commit()?;
    Ok(ApprovalDecisionResult {
        plan: plan_by_id(&connection, &request.plan_id)?,
        approval: approval_by_plan(&connection, &request.plan_id)?,
        message: "Plan approved for one supervised execution. Approval is bound to the exact plan hash and expires with the plan.".to_owned(),
    })
}

fn reject_plan_record(state: &AppState, request: PlanReferenceRequest) -> Result<ApprovalDecisionResult> {
    ensure!(request.confirmation == "REJECT", "type REJECT to reject this plan");
    validate_uuid(&request.plan_id, "plan id")?;
    let reason = sanitize_optional_text(request.reason.as_deref(), 500, "rejection reason")?
        .unwrap_or_else(|| "Rejected by the local user".to_owned());
    let connection = state.connection()?;
    expire_pending(&connection)?;
    let plan = plan_by_id(&connection, &request.plan_id)?;
    ensure!(plan.state == "awaiting_approval", "plan is not awaiting approval");
    let now = now_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE agent_approval_requests SET state='rejected',decided_at_utc=?2,decision_reason=?3 WHERE plan_id=?1 AND state='pending'",
        params![plan.plan_id, now, reason],
    )?;
    transaction.execute(
        "UPDATE agent_plans SET state='rejected',failure_code='local_rejection',completed_at_utc=?2,updated_at_utc=?2 WHERE plan_id=?1 AND state='awaiting_approval'",
        params![plan.plan_id, now],
    )?;
    transaction.execute(
        "UPDATE agent_plan_steps SET state='cancelled',updated_at_utc=?2 WHERE plan_id=?1 AND state='pending'",
        params![plan.plan_id, now],
    )?;
    transaction.commit()?;
    Ok(ApprovalDecisionResult {
        plan: plan_by_id(&connection, &request.plan_id)?,
        approval: approval_by_plan(&connection, &request.plan_id)?,
        message: "Plan rejected. No action was executed.".to_owned(),
    })
}

fn cancel_plan_record(
    state: &AppState,
    request: PlanReferenceRequest,
    requesting_mcp_client: Option<&str>,
    allow_approved: bool,
) -> Result<AgentPlanSummary> {
    ensure!(request.confirmation == "CANCEL", "type CANCEL to cancel this plan");
    validate_uuid(&request.plan_id, "plan id")?;
    let connection = state.connection()?;
    let plan = plan_by_id(&connection, &request.plan_id)?;
    if let Some(client_id) = requesting_mcp_client {
        ensure_mcp_plan_access(&plan, client_id)?;
    }
    let allowed_states = if allow_approved {
        ["draft", "awaiting_approval", "approved"].as_slice()
    } else {
        ["draft", "awaiting_approval"].as_slice()
    };
    ensure!(allowed_states.contains(&plan.state.as_str()), "only an unexecuted plan may be cancelled");
    let reason = sanitize_optional_text(request.reason.as_deref(), 500, "cancellation reason")?
        .unwrap_or_else(|| "Cancelled before execution".to_owned());
    let now = now_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE agent_plans SET state='cancelled',failure_code='cancelled',completed_at_utc=?2,updated_at_utc=?2 WHERE plan_id=?1",
        params![plan.plan_id, now],
    )?;
    transaction.execute(
        "UPDATE agent_approval_requests SET state='cancelled',decided_at_utc=?2,decision_reason=?3 WHERE plan_id=?1 AND state IN ('pending','approved')",
        params![plan.plan_id, now, reason],
    )?;
    transaction.execute(
        "UPDATE agent_plan_steps SET state='cancelled',updated_at_utc=?2 WHERE plan_id=?1 AND state='pending'",
        params![plan.plan_id, now],
    )?;
    transaction.commit()?;
    plan_by_id(&connection, &request.plan_id)
}

async fn execute_approved_plan(
    state: Arc<AppState>,
    request: PlanReferenceRequest,
) -> Result<AgentExecutionReceiptSummary> {
    ensure!(request.confirmation == "EXECUTE", "type EXECUTE to run this approved plan");
    validate_uuid(&request.plan_id, "plan id")?;
    if let Some(existing) = {
        let connection = state.connection()?;
        receipt_by_plan(&connection, &request.plan_id).optional()?
    } {
        return Ok(existing);
    }

    let (plan, approval, idempotency_key, started_at_utc) = {
        let mut connection = state.connection()?;
        expire_pending(&connection)?;
        let mut plan = plan_by_id(&connection, &request.plan_id)?;
        ensure!(plan.state == "approved", "plan is not approved for execution");
        ensure!(!is_expired(&plan.expires_at_utc), "approved plan has expired");
        let approval = approval_record_by_plan(&connection, &plan.plan_id)?;
        ensure!(approval.plan_hash == plan.plan_hash, "approval is not bound to the current plan hash");
        ensure!(approval.consumed_at_utc.is_none(), "approval has already been consumed");
        ensure!(!is_expired(&approval.expires_at_utc), "approval has expired");
        let current_fresh_state = fresh_state_token(
            &connection,
            &plan.action_type,
            plan.connection_id.as_deref(),
        )?;
        if current_fresh_state != plan.fresh_state_token {
            refresh_plan_for_reapproval(&mut connection, &mut plan, &current_fresh_state)?;
            bail!("material target state changed; the plan was returned for fresh approval");
        }
        let idempotency_key = format!("agent:{}", plan.plan_hash);
        let started_at_utc = now_string();
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO agent_action_idempotency (idempotency_key,plan_id,state,created_at_utc,updated_at_utc) VALUES (?1,?2,'executing',?3,?3)",
            params![idempotency_key, plan.plan_id, started_at_utc],
        )?;
        transaction.execute(
            "UPDATE agent_plans SET state='executing',updated_at_utc=?2 WHERE plan_id=?1 AND state='approved'",
            params![plan.plan_id, started_at_utc],
        )?;
        transaction.execute(
            "UPDATE agent_plan_steps SET state='executing',updated_at_utc=?2 WHERE plan_id=?1 AND state='pending'",
            params![plan.plan_id, started_at_utc],
        )?;
        transaction.execute(
            "UPDATE agent_approvals SET consumed_at_utc=?2 WHERE plan_id=?1 AND consumed_at_utc IS NULL",
            params![plan.plan_id, started_at_utc],
        )?;
        transaction.execute(
            "UPDATE agent_approval_requests SET state='consumed',decided_at_utc=COALESCE(decided_at_utc,?2) WHERE plan_id=?1 AND state='approved'",
            params![plan.plan_id, started_at_utc],
        )?;
        transaction.commit()?;
        (plan, approval, idempotency_key, started_at_utc)
    };

    let execution = execute_action(state.clone(), &plan).await;
    let completed_at_utc = now_string();
    let (receipt_state, result_code, result_summary, result_json) = match execution {
        Ok((code, summary, result)) => ("completed", code, summary, result),
        Err(error) => (
            "failed",
            public_failure_code(&error),
            "The approved action failed inside its bounded executor.".to_owned(),
            json!({ "error": public_failure_code(&error) }),
        ),
    };
    let receipt_id = Uuid::new_v4().to_string();
    {
        let connection = state.connection()?;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO agent_execution_receipts (receipt_id,plan_id,approval_id,plan_hash,action_type,connection_id,idempotency_key,state,result_code,result_summary,result_json,started_at_utc,completed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![receipt_id, plan.plan_id, approval.approval_id, plan.plan_hash, plan.action_type, plan.connection_id, idempotency_key, receipt_state, result_code, result_summary, serde_json::to_string(&result_json)?, started_at_utc, completed_at_utc],
        )?;
        transaction.execute(
            "UPDATE agent_action_idempotency SET state=?2,receipt_id=?3,updated_at_utc=?4 WHERE idempotency_key=?1",
            params![idempotency_key, receipt_state, receipt_id, completed_at_utc],
        )?;
        transaction.execute(
            "UPDATE agent_plans SET state=?2,failure_code=?3,completed_at_utc=?4,updated_at_utc=?4 WHERE plan_id=?1",
            params![plan.plan_id, if receipt_state == "completed" { "completed" } else { "failed" }, if receipt_state == "completed" { None::<String> } else { Some(result_code.clone()) }, completed_at_utc],
        )?;
        transaction.execute(
            "UPDATE agent_plan_steps SET state=?2,updated_at_utc=?3 WHERE plan_id=?1 AND state='executing'",
            params![plan.plan_id, if receipt_state == "completed" { "completed" } else { "failed" }, completed_at_utc],
        )?;
        transaction.commit()?;
    }
    let connection = state.connection()?;
    receipt_by_plan(&connection, &plan.plan_id)
}

async fn execute_action(
    state: Arc<AppState>,
    plan: &AgentPlanSummary,
) -> Result<(String, String, Value)> {
    match plan.action_type.as_str() {
        "backup.create" => {
            let note = plan
                .arguments
                .get("note")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| Some(format!("Approved Agent Workspace plan {}", plan.plan_id)));
            let backup_state = state.clone();
            let result = tokio::task::spawn_blocking(move || {
                backup_state.create_backup(CreateBackupRequest {
                    kind: BackupKind::Manual,
                    passphrase: None,
                    note,
                })
            })
            .await
            .context("backup execution task failed")??;
            Ok((
                "backup_created".to_owned(),
                result.message.clone(),
                serde_json::to_value(result)?,
            ))
        }
        "model.health_test" => {
            let snapshot = model_center::snapshot(state.clone()).await?;
            ensure!(snapshot.runtime.state == "running", "local model runtime is not running");
            let requested_model = plan.arguments.get("model").and_then(Value::as_str);
            let model = requested_model
                .map(ToOwned::to_owned)
                .or_else(|| snapshot.settings.default_chat_model.clone())
                .or_else(|| snapshot.installed_models.first().map(|model| model.name.clone()))
                .context("no installed local model is available for the health test")?;
            let test: model_center::ModelTestResult = local_post_json(
                "/v1/models/test",
                &json!({
                    "model": model,
                    "prompt": "Reply with HOME_SERVER_MODEL_OK and one short sentence confirming local inference is available."
                }),
            )
            .await?;
            Ok((
                "model_health_passed".to_owned(),
                format!("Local model {} responded in {} ms.", test.model, test.duration_ms),
                serde_json::to_value(test)?,
            ))
        }
        "cloud.sync_connection" => {
            let connection_id = plan.connection_id.as_deref().context("connection id is required")?;
            let result: Value = local_post_json(
                "/v1/cloud/connections/sync",
                &json!({ "connection_id": connection_id }),
            )
            .await?;
            Ok((
                "connection_sync_completed".to_owned(),
                "Approved connection synchronization completed.".to_owned(),
                result,
            ))
        }
        "cloud.sync_all" => {
            let result: Value = local_post_json(
                "/v1/cloud/connections/sync-all",
                &json!({}),
            )
            .await?;
            Ok((
                "all_connections_sync_completed".to_owned(),
                "Approved synchronization completed across active connections.".to_owned(),
                result,
            ))
        }
        "report.save" => {
            let title = plan
                .arguments
                .get("title")
                .and_then(Value::as_str)
                .context("report title is required")?;
            let content = plan
                .arguments
                .get("content_markdown")
                .and_then(Value::as_str)
                .context("report content is required")?;
            let report_state = state.clone();
            let plan_id = plan.plan_id.clone();
            let connection_ids = plan
                .connection_id
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let dataset_keys = plan.dataset_keys.clone();
            let title = title.to_owned();
            let content = content.to_owned();
            let report = tokio::task::spawn_blocking(move || {
                save_report(
                    &report_state,
                    Some(&plan_id),
                    &title,
                    &content,
                    &connection_ids,
                    &dataset_keys,
                )
            })
            .await
            .context("report save task failed")??;
            Ok((
                "report_saved".to_owned(),
                "Approved operational report was saved locally.".to_owned(),
                serde_json::to_value(report)?,
            ))
        }
        _ => bail!("action executor is not installed"),
    }
}

fn save_world_mission(
    state: &AppState,
    request: CreateWorldMissionRequest,
    actor_type: &str,
    actor_id: &str,
) -> Result<WorldMissionSummary> {
    ensure!(["local_user", "mcp_client", "system"].contains(&actor_type), "mission actor type is invalid");
    let actor_id = sanitize_required_text(actor_id, 160, "mission actor")?;
    let thread_id = normalize_optional_uuid(request.thread_id.as_deref(), "thread id")?;
    let goal_id = normalize_optional_uuid(request.goal_id.as_deref(), "goal id")?;
    let connection_id = normalize_optional_uuid(request.connection_id.as_deref(), "connection id")?;
    let world_agent_id = sanitize_required_text(&request.world_agent_id, 160, "World Agent id")?;
    let title = sanitize_required_text(&request.title, 180, "mission title")?;
    let objective = sanitize_required_text(&request.objective, 4000, "mission objective")?;
    let allowed_operations = normalize_world_operations(&request.allowed_operations, false)?;
    let mut prohibited_operations = normalize_world_operations(&request.prohibited_operations, true)?;
    if prohibited_operations.is_empty() {
        prohibited_operations = PROHIBITED_WORLD_OPERATIONS
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
    }
    ensure_json_object(&request.limits, MAX_WORLD_LIMIT_BYTES, "mission limits")?;
    ensure_json_object(
        &request.disclosure_policy,
        MAX_WORLD_LIMIT_BYTES,
        "mission disclosure policy",
    )?;
    let expires_minutes = request.expires_minutes.unwrap_or(240).clamp(15, 7 * 24 * 60);
    let expires_at_utc = (Utc::now() + ChronoDuration::minutes(i64::from(expires_minutes)))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let connection = state.connection()?;
    if let Some(connection_id) = connection_id.as_deref() {
        validate_connection_ids(&connection, &[connection_id.to_owned()])?;
    }
    if let Some(goal_id) = goal_id.as_deref() {
        let _ = goal_by_id(&connection, goal_id)?;
    }
    if let Some(thread_id) = thread_id.as_deref() {
        let _ = thread_by_id(&connection, thread_id)?;
    }
    let mission_id = Uuid::new_v4().to_string();
    let now = now_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO world_missions (mission_id,thread_id,goal_id,connection_id,world_agent_id,title,objective,allowed_operations_json,prohibited_operations_json,limits_json,disclosure_policy_json,state,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'draft',?12,?13,?13)",
        params![mission_id, thread_id, goal_id, connection_id, world_agent_id, title, objective, serde_json::to_string(&allowed_operations)?, serde_json::to_string(&prohibited_operations)?, serde_json::to_string(&request.limits)?, serde_json::to_string(&request.disclosure_policy)?, expires_at_utc, now],
    )?;
    transaction.execute(
        "INSERT INTO world_tasks (task_id,mission_id,title,description,state,due_at_utc) VALUES (?1,?2,?3,?4,'draft',?5)",
        params![Uuid::new_v4().to_string(), mission_id, title, objective, expires_at_utc],
    )?;
    transaction.execute(
        "INSERT INTO world_mission_events (event_id,mission_id,event_type,outcome,metadata_json,created_at_utc) VALUES (?1,?2,'WORLD_MISSION_DRAFTED','success',?3,?4)",
        params![Uuid::new_v4().to_string(), mission_id, serde_json::to_string(&json!({ "actor_type": actor_type, "actor_id": actor_id, "dispatch_enabled": false }))?, now],
    )?;
    transaction.commit()?;
    mission_by_id(&connection, &mission_id)
}

fn cancel_mission_record(
    state: &AppState,
    request: MissionReferenceRequest,
    _requesting_mcp_client: Option<&str>,
) -> Result<WorldMissionSummary> {
    ensure!(request.confirmation == "CANCEL", "type CANCEL to cancel this World Mission");
    validate_uuid(&request.mission_id, "mission id")?;
    let connection = state.connection()?;
    let mission = mission_by_id(&connection, &request.mission_id)?;
    ensure!(
        ["draft", "awaiting_approval", "ready_for_dispatch"].contains(&mission.state.as_str()),
        "only an undispatched World Mission may be cancelled"
    );
    let now = now_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE world_missions SET state='cancelled',updated_at_utc=?2 WHERE mission_id=?1",
        params![mission.mission_id, now],
    )?;
    transaction.execute(
        "UPDATE world_tasks SET state='cancelled',updated_at_utc=?2 WHERE mission_id=?1 AND state IN ('draft','queued')",
        params![mission.mission_id, now],
    )?;
    transaction.execute(
        "INSERT INTO world_mission_events (event_id,mission_id,event_type,outcome,metadata_json,created_at_utc) VALUES (?1,?2,'WORLD_MISSION_CANCELLED','success','{}',?3)",
        params![Uuid::new_v4().to_string(), mission.mission_id, now],
    )?;
    transaction.commit()?;
    mission_by_id(&connection, &request.mission_id)
}

fn save_report(
    state: &AppState,
    plan_id: Option<&str>,
    title: &str,
    content: &str,
    connection_ids: &[String],
    dataset_keys: &[String],
) -> Result<AgentReportSummary> {
    let title = sanitize_required_text(title, 180, "report title")?;
    let content = sanitize_required_text(content, MAX_REPORT_CHARS, "report content")?;
    let connection_ids = normalize_ids(connection_ids, MAX_CONTEXT_ITEMS, "connection")?;
    let dataset_keys = normalize_dataset_keys(dataset_keys)?;
    let connection = state.connection()?;
    validate_connection_ids(&connection, &connection_ids)?;
    let report_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO agent_reports (report_id,plan_id,title,content_markdown,connection_ids_json,dataset_keys_json) VALUES (?1,?2,?3,?4,?5,?6)",
        params![report_id, plan_id, title, content, serde_json::to_string(&connection_ids)?, serde_json::to_string(&dataset_keys)?],
    )?;
    report_by_id(&connection, &report_id)
}

fn read_workspace_local(state: &AppState) -> Result<WorkspaceLocalSnapshot> {
    let connection = state.connection()?;
    expire_pending(&connection)?;
    Ok(WorkspaceLocalSnapshot {
        goals: list_goals(&connection)?,
        threads: list_threads(&connection)?,
        messages: list_messages(&connection)?,
        plans: list_plans(&connection)?,
        approvals: list_approvals(&connection)?,
        receipts: list_receipts(&connection)?,
        reports: list_reports(&connection)?,
        missions: list_missions(&connection)?,
    })
}

fn build_data_sources(
    clouds: &cloud_registry::CloudConnectionsSnapshot,
    local: &WorkspaceLocalSnapshot,
    models: Option<&model_center::ModelCenterSnapshot>,
) -> Vec<AgentDataSourceSummary> {
    let mut sources = vec![
        AgentDataSourceSummary {
            key: "system".to_owned(),
            label: "HomeServer System".to_owned(),
            state: "ready".to_owned(),
            detail: "Local health, backups, services, approvals, and receipts.".to_owned(),
            last_updated_utc: Some(now_string()),
            connection_id: None,
        },
        AgentDataSourceSummary {
            key: "knowledge".to_owned(),
            label: "Knowledge Vault".to_owned(),
            state: "ready".to_owned(),
            detail: "Private local document search with bounded citations.".to_owned(),
            last_updated_utc: None,
            connection_id: None,
        },
        AgentDataSourceSummary {
            key: "models".to_owned(),
            label: "Local Models".to_owned(),
            state: models
                .map(|snapshot| snapshot.runtime.state.clone())
                .unwrap_or_else(|| "unavailable".to_owned()),
            detail: "Optional fixed-loopback local inference.".to_owned(),
            last_updated_utc: None,
            connection_id: None,
        },
        AgentDataSourceSummary {
            key: "goals".to_owned(),
            label: "Saved Goals".to_owned(),
            state: if local.goals.iter().any(|goal| goal.state == "active") {
                "ready".to_owned()
            } else {
                "empty".to_owned()
            },
            detail: format!("{} saved goal records.", local.goals.len()),
            last_updated_utc: local.goals.first().map(|goal| goal.updated_at_utc.clone()),
            connection_id: None,
        },
        AgentDataSourceSummary {
            key: "operational_data".to_owned(),
            label: "Operational Platform Data".to_owned(),
            state: "planned_phase_5c".to_owned(),
            detail: "Initial snapshots, incremental cursors, events, and normalized business records are not imported in this slice.".to_owned(),
            last_updated_utc: None,
            connection_id: None,
        },
        AgentDataSourceSummary {
            key: "world_canvas".to_owned(),
            label: "World Canvas".to_owned(),
            state: "mission_drafting".to_owned(),
            detail: "World Mission drafting and local lifecycle records are enabled; dispatch is not yet installed.".to_owned(),
            last_updated_utc: local.missions.first().map(|mission| mission.updated_at_utc.clone()),
            connection_id: None,
        },
    ];
    for connection in &clouds.connections {
        sources.push(AgentDataSourceSummary {
            key: format!("connection:{}", connection.connection_id),
            label: connection.display_name.clone(),
            state: format!("{:?}", connection.state).to_ascii_lowercase(),
            detail: format!(
                "{} · tenant {} · site {} · {} pending",
                connection.provider_key,
                connection.tenant_id.as_deref().unwrap_or("provider-managed"),
                connection.site_id.as_deref().unwrap_or("provider-managed"),
                connection.pending_sync
            ),
            last_updated_utc: connection.last_success_utc.clone(),
            connection_id: Some(connection.connection_id.clone()),
        });
    }
    sources
}

fn ensure_thread(state: &AppState, requested: Option<&str>, prompt: &str) -> Result<String> {
    let connection = state.connection()?;
    if let Some(thread_id) = requested {
        validate_uuid(thread_id, "thread id")?;
        let _ = thread_by_id(&connection, thread_id)?;
        connection.execute(
            "UPDATE agent_threads SET updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE thread_id=?1",
            params![thread_id],
        )?;
        return Ok(thread_id.to_owned());
    }
    let thread_id = Uuid::new_v4().to_string();
    let title = truncate_chars(prompt, 100);
    connection.execute(
        "INSERT INTO agent_threads (thread_id,title,state) VALUES (?1,?2,'active')",
        params![thread_id, title],
    )?;
    Ok(thread_id)
}

fn save_message(
    state: &AppState,
    thread_id: &str,
    role: &str,
    mode: &str,
    content: &str,
    context: &Value,
) -> Result<String> {
    ensure!(["user", "assistant", "system"].contains(&role), "message role is invalid");
    let content = sanitize_required_text(content, MAX_MESSAGE_CHARS, "message")?;
    ensure_json_size(context, 64 * 1024, "message context")?;
    let connection = state.connection()?;
    let _ = thread_by_id(&connection, thread_id)?;
    let message_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO agent_messages (message_id,thread_id,role,mode,content,context_json) VALUES (?1,?2,?3,?4,?5,?6)",
        params![message_id, thread_id, role, mode, content, serde_json::to_string(context)?],
    )?;
    connection.execute(
        "UPDATE agent_threads SET updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE thread_id=?1",
        params![thread_id],
    )?;
    Ok(message_id)
}

fn refresh_plan_for_reapproval(
    connection: &mut Connection,
    plan: &mut AgentPlanSummary,
    current_fresh_state: &str,
) -> Result<()> {
    let expires_at_utc = (Utc::now() + ChronoDuration::minutes(30))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let canonical = json!({
        "schema": 1,
        "plan_id": plan.plan_id,
        "requested_by_type": plan.requested_by_type,
        "requested_by_id": plan.requested_by_id,
        "title": plan.title,
        "rationale": plan.rationale,
        "action_type": plan.action_type,
        "arguments": plan.arguments,
        "connection_id": plan.connection_id,
        "goal_id": plan.goal_id,
        "dataset_keys": plan.dataset_keys,
        "fresh_state_token": current_fresh_state,
        "expires_at_utc": expires_at_utc,
        "reapproval_revision": Uuid::new_v4().to_string(),
    });
    let new_hash = hash_json(&canonical)?;
    let now = now_string();
    let transaction = connection.transaction()?;
    transaction.execute(
        "DELETE FROM agent_approvals WHERE plan_id=?1 AND consumed_at_utc IS NULL",
        params![plan.plan_id],
    )?;
    transaction.execute(
        "UPDATE agent_plans SET state='awaiting_approval',plan_hash=?2,fresh_state_token=?3,expires_at_utc=?4,updated_at_utc=?5 WHERE plan_id=?1",
        params![plan.plan_id, new_hash, current_fresh_state, expires_at_utc, now],
    )?;
    transaction.execute(
        "UPDATE agent_approval_requests SET plan_hash=?2,state='pending',requested_at_utc=?3,expires_at_utc=?4,decided_at_utc=NULL,decision_reason='Material target state changed; fresh approval required' WHERE plan_id=?1",
        params![plan.plan_id, new_hash, now, expires_at_utc],
    )?;
    transaction.commit()?;
    Ok(())
}

fn fresh_state_token(
    connection: &Connection,
    action_type: &str,
    connection_id: Option<&str>,
) -> Result<String> {
    let payload = match action_type {
        "cloud.sync_connection" => {
            let connection_id = connection_id.context("connection id is required")?;
            let summary = cloud_connection_identity(connection, connection_id)?;
            json!({
                "action": action_type,
                "connection_id": connection_id,
                "provider_key": summary.0,
                "tenant_id": summary.1,
                "site_id": summary.2,
                "device_id": summary.3,
                "state": summary.4,
            })
        }
        "cloud.sync_all" => {
            let mut statement = connection.prepare(
                "SELECT connection_id,provider_key,COALESCE(tenant_id,''),COALESCE(site_id,''),device_id,state FROM cloud_connections WHERE state IN ('connected','degraded','pairing') ORDER BY connection_id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok(json!({
                    "connection_id": row.get::<_, String>(0)?,
                    "provider_key": row.get::<_, String>(1)?,
                    "tenant_id": row.get::<_, String>(2)?,
                    "site_id": row.get::<_, String>(3)?,
                    "device_id": row.get::<_, String>(4)?,
                    "state": row.get::<_, String>(5)?,
                }))
            })?;
            let identities = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            json!({ "action": action_type, "connections": identities })
        }
        _ => json!({ "action": action_type, "local_authority": "homeserver" }),
    };
    hash_json(&payload)
}

fn cloud_connection_identity(
    connection: &Connection,
    connection_id: &str,
) -> Result<(String, Option<String>, Option<String>, String, String)> {
    connection
        .query_row(
            "SELECT provider_key,tenant_id,site_id,device_id,state FROM cloud_connections WHERE connection_id=?1",
            params![connection_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .context("cloud connection was not found")
}

fn validate_action_target(
    connection: &Connection,
    action_type: &str,
    connection_id: Option<&str>,
) -> Result<()> {
    match action_type {
        "cloud.sync_connection" => {
            let connection_id = connection_id.context("connection id is required for a connection sync")?;
            let (_, _, _, _, state) = cloud_connection_identity(connection, connection_id)?;
            ensure!(["connected", "degraded", "pairing"].contains(&state.as_str()), "cloud connection is inactive");
        }
        "cloud.sync_all" => ensure!(connection_id.is_none(), "sync-all cannot be redirected to one connection"),
        _ => ensure!(connection_id.is_none() || action_type == "report.save", "local action cannot target a cloud connection"),
    }
    Ok(())
}

fn validate_action_arguments(action_type: &str, arguments: Value) -> Result<Value> {
    let mut object = match arguments {
        Value::Null => Map::new(),
        Value::Object(object) => object,
        _ => bail!("action arguments must be one JSON object"),
    };
    match action_type {
        "backup.create" => {
            reject_unknown_keys(&object, &["note"])?;
            if let Some(note) = object.get("note").and_then(Value::as_str) {
                object.insert(
                    "note".to_owned(),
                    Value::String(sanitize_required_text(note, 500, "backup note")?),
                );
            }
        }
        "model.health_test" => {
            reject_unknown_keys(&object, &["model"])?;
            if let Some(model) = object.get("model").and_then(Value::as_str) {
                object.insert(
                    "model".to_owned(),
                    Value::String(sanitize_required_text(model, 160, "model")?),
                );
            }
        }
        "cloud.sync_connection" | "cloud.sync_all" => reject_unknown_keys(&object, &[])?,
        "report.save" => {
            reject_unknown_keys(&object, &["title", "content_markdown"])?;
            let title = object
                .get("title")
                .and_then(Value::as_str)
                .context("report title is required")?;
            let content = object
                .get("content_markdown")
                .and_then(Value::as_str)
                .context("report content is required")?;
            object.insert(
                "title".to_owned(),
                Value::String(sanitize_required_text(title, 180, "report title")?),
            );
            object.insert(
                "content_markdown".to_owned(),
                Value::String(sanitize_required_text(content, MAX_REPORT_CHARS, "report content")?),
            );
        }
        _ => bail!("action type is not installed"),
    }
    let value = Value::Object(object);
    ensure_json_size(&value, 64 * 1024, "action arguments")?;
    Ok(value)
}

fn risk_summary(action_type: &str, connection_id: Option<&str>) -> String {
    match action_type {
        "backup.create" => "Creates one encrypted local HomeServer backup. It does not change cloud data.".to_owned(),
        "model.health_test" => "Runs one bounded local inference test against an approved installed model.".to_owned(),
        "cloud.sync_connection" => format!("Runs the existing signed synchronization contract for connection {}. Only the current low-risk allowlisted sync operations are eligible.", connection_id.unwrap_or("unknown")),
        "cloud.sync_all" => "Runs the existing signed synchronization contract for every active connection. No commerce mutation is enabled.".to_owned(),
        "report.save" => "Writes one bounded operational report into local HomeServer storage.".to_owned(),
        _ => "Unknown supervised action.".to_owned(),
    }
}

fn action_risk(action_type: &str) -> &'static str {
    match action_type {
        "cloud.sync_connection" | "cloud.sync_all" => "medium",
        _ => "low",
    }
}

fn normalize_mode(value: &str) -> Result<String> {
    let mode = value.trim().to_ascii_lowercase();
    ensure!(ALLOWED_MODES.contains(&mode.as_str()), "agent mode is invalid");
    Ok(mode)
}

fn normalize_dataset_keys(values: &[String]) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for value in values.iter().take(MAX_CONTEXT_ITEMS) {
        let key = value.trim().to_ascii_lowercase();
        if key.starts_with("connection:") {
            validate_uuid(key.trim_start_matches("connection:"), "connection dataset id")?;
        } else {
            ensure!(ALLOWED_DATASET_KEYS.contains(&key.as_str()), "dataset key is not available");
        }
        if !normalized.contains(&key) {
            normalized.push(key);
        }
    }
    Ok(normalized)
}

fn normalize_action_list(values: &[String]) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for value in values.iter().take(MAX_CONTEXT_ITEMS) {
        let action = value.trim().to_ascii_lowercase();
        ensure!(ALLOWED_ACTION_TYPES.contains(&action.as_str()), "goal action is not enabled");
        if !normalized.contains(&action) {
            normalized.push(action);
        }
    }
    Ok(normalized)
}

fn normalize_world_operations(values: &[String], prohibited: bool) -> Result<Vec<String>> {
    let allowed_set = if prohibited {
        PROHIBITED_WORLD_OPERATIONS
    } else {
        ALLOWED_WORLD_OPERATIONS
    };
    let defaults = if prohibited {
        Vec::new()
    } else {
        vec!["discover".to_owned(), "compare".to_owned(), "prepare_recommendation".to_owned()]
    };
    let source = if values.is_empty() { &defaults } else { values };
    let mut normalized = Vec::new();
    for value in source.iter().take(MAX_CONTEXT_ITEMS) {
        let operation = value.trim().to_ascii_lowercase();
        ensure!(allowed_set.contains(&operation.as_str()), "World Mode operation is not enabled");
        if !normalized.contains(&operation) {
            normalized.push(operation);
        }
    }
    Ok(normalized)
}

fn normalize_ids(values: &[String], maximum: usize, label: &str) -> Result<Vec<String>> {
    ensure!(values.len() <= maximum, "too many {label} ids were supplied");
    let mut normalized = Vec::new();
    for value in values {
        validate_uuid(value, label)?;
        if !normalized.contains(value) {
            normalized.push(value.clone());
        }
    }
    Ok(normalized)
}

fn normalize_optional_uuid(value: Option<&str>, label: &str) -> Result<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            validate_uuid(value, label)?;
            Ok(Some(value.to_owned()))
        }
        None => Ok(None),
    }
}

fn validate_uuid(value: &str, label: &str) -> Result<()> {
    ensure!(Uuid::parse_str(value).is_ok(), "{label} is invalid");
    Ok(())
}

fn validate_connection_ids(connection: &Connection, ids: &[String]) -> Result<()> {
    for connection_id in ids {
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM cloud_connections WHERE connection_id=?1",
            params![connection_id],
            |row| row.get(0),
        )?;
        ensure!(count == 1, "selected cloud connection was not found");
    }
    Ok(())
}

fn select_connections(
    snapshot: &cloud_registry::CloudConnectionsSnapshot,
    requested: &[String],
) -> Result<Vec<cloud_registry::CloudConnectionSummary>> {
    if requested.is_empty() {
        return Ok(snapshot.connections.clone());
    }
    let requested = requested.iter().collect::<HashSet<_>>();
    let selected = snapshot
        .connections
        .iter()
        .filter(|connection| requested.contains(&connection.connection_id))
        .cloned()
        .collect::<Vec<_>>();
    ensure!(selected.len() == requested.len(), "one or more selected connections were not found");
    Ok(selected)
}

fn goals_by_ids(state: &AppState, ids: &[String]) -> Result<Vec<AgentGoalSummary>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let connection = state.connection()?;
    ids.iter().map(|goal_id| goal_by_id(&connection, goal_id)).collect()
}

fn ensure_mcp_plan_access(plan: &AgentPlanSummary, client_id: &str) -> Result<()> {
    ensure!(
        plan.requested_by_type == "mcp_client" && plan.requested_by_id == client_id,
        "MCP client may inspect or cancel only its own plan requests"
    );
    Ok(())
}

fn expire_pending(connection: &Connection) -> Result<()> {
    let now = now_string();
    connection.execute(
        "UPDATE agent_plans SET state='expired',failure_code='approval_expired',completed_at_utc=?1,updated_at_utc=?1 WHERE state IN ('awaiting_approval','approved') AND expires_at_utc<=?1",
        params![now],
    )?;
    connection.execute(
        "UPDATE agent_approval_requests SET state='expired',decided_at_utc=?1,decision_reason='Approval window expired' WHERE state IN ('pending','approved') AND expires_at_utc<=?1",
        params![now],
    )?;
    connection.execute(
        "UPDATE agent_plan_steps SET state='cancelled',updated_at_utc=?1 WHERE plan_id IN (SELECT plan_id FROM agent_plans WHERE state='expired') AND state='pending'",
        params![now],
    )?;
    connection.execute(
        "UPDATE world_missions SET state='expired',updated_at_utc=?1 WHERE state IN ('draft','awaiting_approval','ready_for_dispatch') AND expires_at_utc<=?1",
        params![now],
    )?;
    Ok(())
}

async fn local_post_json<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T> {
    ensure!(path.starts_with("/v1/"), "local action path is invalid");
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(15 * 60))
        .redirect(Policy::none())
        .build()?;
    let response = client
        .post(format!("{}{}", api_base_url(), path))
        .header(LOCAL_CLIENT_HEADER, LOCAL_CLIENT_VALUE)
        .json(body)
        .send()
        .await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    ensure!(bytes.len() <= MAX_LOCAL_RESPONSE_BYTES, "local action response exceeded the size limit");
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| value.get("message").and_then(Value::as_str).map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("local action failed with HTTP {status}"));
        bail!(message);
    }
    serde_json::from_slice(&bytes).context("local action returned invalid JSON")
}

fn reject_unknown_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    for key in object.keys() {
        ensure!(allowed.contains(&key.as_str()), "action argument '{key}' is not allowed");
    }
    Ok(())
}

fn ensure_json_object(value: &Value, maximum_bytes: usize, label: &str) -> Result<()> {
    ensure!(value.is_object() || value.is_null(), "{label} must be a JSON object");
    ensure_json_size(value, maximum_bytes, label)
}

fn ensure_json_size(value: &Value, maximum_bytes: usize, label: &str) -> Result<()> {
    ensure!(serde_json::to_vec(value)?.len() <= maximum_bytes, "{label} exceeds its size limit");
    Ok(())
}

fn sanitize_required_text(value: &str, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty(), "{label} is required");
    ensure!(value.chars().count() <= maximum, "{label} exceeds the {maximum} character limit");
    ensure!(!value.chars().any(|character| character == '\0'), "{label} contains invalid characters");
    Ok(value.to_owned())
}

fn sanitize_optional_text(value: Option<&str>, maximum: usize, label: &str) -> Result<Option<String>> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => sanitize_required_text(value, maximum, label).map(Some),
        None => Ok(None),
    }
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn hash_json(value: &Value) -> Result<String> {
    let canonical = serde_json::to_vec(value)?;
    Ok(hex::encode(Sha256::digest(canonical)))
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn is_expired(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

fn public_failure_code(error: &anyhow::Error) -> String {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("model") && message.contains("not running") {
        "model_runtime_unavailable"
    } else if message.contains("cloud") || message.contains("connection") || message.contains("sync") {
        "connection_action_failed"
    } else if message.contains("backup") {
        "backup_action_failed"
    } else if message.contains("report") {
        "report_action_failed"
    } else {
        "bounded_action_failed"
    }
    .to_owned()
}

fn list_goals(connection: &Connection) -> Result<Vec<AgentGoalSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {GOAL_COLUMNS} FROM agent_goals ORDER BY CASE state WHEN 'active' THEN 0 ELSE 1 END,updated_at_utc DESC,goal_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map([], map_goal)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn goal_by_id(connection: &Connection, goal_id: &str) -> Result<AgentGoalSummary> {
    connection
        .query_row(
            &format!("SELECT {GOAL_COLUMNS} FROM agent_goals WHERE goal_id=?1"),
            params![goal_id],
            map_goal,
        )
        .context("goal was not found")
}

fn map_goal(row: &Row<'_>) -> rusqlite::Result<AgentGoalSummary> {
    Ok(AgentGoalSummary {
        goal_id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        target_metric: row.get(3)?,
        target_value: row.get(4)?,
        target_date: row.get(5)?,
        connection_ids: parse_json(row.get::<_, String>(6)?),
        dataset_keys: parse_json(row.get::<_, String>(7)?),
        constraints: parse_json(row.get::<_, String>(8)?),
        allowed_actions: parse_json(row.get::<_, String>(9)?),
        approval_policy: row.get(10)?,
        state: row.get(11)?,
        created_at_utc: row.get(12)?,
        updated_at_utc: row.get(13)?,
    })
}

fn list_threads(connection: &Connection) -> Result<Vec<AgentThreadSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {THREAD_COLUMNS} FROM agent_threads ORDER BY updated_at_utc DESC,thread_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map([], map_thread)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn thread_by_id(connection: &Connection, thread_id: &str) -> Result<AgentThreadSummary> {
    connection
        .query_row(
            &format!("SELECT {THREAD_COLUMNS} FROM agent_threads WHERE thread_id=?1"),
            params![thread_id],
            map_thread,
        )
        .context("agent thread was not found")
}

fn map_thread(row: &Row<'_>) -> rusqlite::Result<AgentThreadSummary> {
    Ok(AgentThreadSummary {
        thread_id: row.get(0)?,
        title: row.get(1)?,
        state: row.get(2)?,
        created_at_utc: row.get(3)?,
        updated_at_utc: row.get(4)?,
    })
}

fn list_messages(connection: &Connection) -> Result<Vec<AgentMessageSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM agent_messages ORDER BY created_at_utc DESC,message_id DESC LIMIT 200"
    ))?;
    let rows = statement.query_map([], map_message)?;
    let mut messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    messages.reverse();
    Ok(messages)
}

fn message_by_id(connection: &Connection, message_id: &str) -> Result<AgentMessageSummary> {
    connection
        .query_row(
            &format!("SELECT {MESSAGE_COLUMNS} FROM agent_messages WHERE message_id=?1"),
            params![message_id],
            map_message,
        )
        .context("agent message was not found")
}

fn map_message(row: &Row<'_>) -> rusqlite::Result<AgentMessageSummary> {
    Ok(AgentMessageSummary {
        message_id: row.get(0)?,
        thread_id: row.get(1)?,
        role: row.get(2)?,
        mode: row.get(3)?,
        content: row.get(4)?,
        context: parse_json(row.get::<_, String>(5)?),
        created_at_utc: row.get(6)?,
    })
}

fn list_plans(connection: &Connection) -> Result<Vec<AgentPlanSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {PLAN_COLUMNS} FROM agent_plans ORDER BY created_at_utc DESC,plan_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map([], map_plan)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn plan_by_id(connection: &Connection, plan_id: &str) -> Result<AgentPlanSummary> {
    connection
        .query_row(
            &format!("SELECT {PLAN_COLUMNS} FROM agent_plans WHERE plan_id=?1"),
            params![plan_id],
            map_plan,
        )
        .context("agent plan was not found")
}

fn map_plan(row: &Row<'_>) -> rusqlite::Result<AgentPlanSummary> {
    Ok(AgentPlanSummary {
        plan_id: row.get(0)?,
        thread_id: row.get(1)?,
        goal_id: row.get(2)?,
        requested_by_type: row.get(3)?,
        requested_by_id: row.get(4)?,
        title: row.get(5)?,
        rationale: row.get(6)?,
        action_type: row.get(7)?,
        arguments: parse_json(row.get::<_, String>(8)?),
        connection_id: row.get(9)?,
        dataset_keys: parse_json(row.get::<_, String>(10)?),
        risk_level: row.get(11)?,
        state: row.get(12)?,
        plan_hash: row.get(13)?,
        fresh_state_token: row.get(14)?,
        expires_at_utc: row.get(15)?,
        failure_code: row.get(16)?,
        created_at_utc: row.get(17)?,
        updated_at_utc: row.get(18)?,
        completed_at_utc: row.get(19)?,
    })
}

fn list_approvals(connection: &Connection) -> Result<Vec<AgentApprovalSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM agent_approval_requests r LEFT JOIN agent_approvals a ON a.approval_request_id=r.approval_request_id ORDER BY r.requested_at_utc DESC,r.approval_request_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map([], map_approval)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn approval_by_plan(connection: &Connection, plan_id: &str) -> Result<AgentApprovalSummary> {
    connection
        .query_row(
            &format!("SELECT {APPROVAL_COLUMNS} FROM agent_approval_requests r LEFT JOIN agent_approvals a ON a.approval_request_id=r.approval_request_id WHERE r.plan_id=?1"),
            params![plan_id],
            map_approval,
        )
        .context("approval request was not found")
}

fn map_approval(row: &Row<'_>) -> rusqlite::Result<AgentApprovalSummary> {
    Ok(AgentApprovalSummary {
        approval_request_id: row.get(0)?,
        plan_id: row.get(1)?,
        plan_hash: row.get(2)?,
        state: row.get(3)?,
        risk_summary: row.get(4)?,
        requested_at_utc: row.get(5)?,
        expires_at_utc: row.get(6)?,
        decided_at_utc: row.get(7)?,
        decision_reason: row.get(8)?,
        approval_id: row.get(9)?,
        approved_by: row.get(10)?,
        approved_at_utc: row.get(11)?,
        consumed_at_utc: row.get(12)?,
    })
}

fn approval_record_by_plan(connection: &Connection, plan_id: &str) -> Result<ApprovalRecord> {
    connection
        .query_row(
            "SELECT approval_id,approval_request_id,plan_id,plan_hash,expires_at_utc,consumed_at_utc FROM agent_approvals WHERE plan_id=?1",
            params![plan_id],
            |row| {
                Ok(ApprovalRecord {
                    approval_id: row.get(0)?,
                    approval_request_id: row.get(1)?,
                    plan_id: row.get(2)?,
                    plan_hash: row.get(3)?,
                    expires_at_utc: row.get(4)?,
                    consumed_at_utc: row.get(5)?,
                })
            },
        )
        .context("approved plan does not have a usable approval")
}

fn list_receipts(connection: &Connection) -> Result<Vec<AgentExecutionReceiptSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {RECEIPT_COLUMNS} FROM agent_execution_receipts ORDER BY completed_at_utc DESC,receipt_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map([], map_receipt)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn receipt_by_plan(connection: &Connection, plan_id: &str) -> rusqlite::Result<AgentExecutionReceiptSummary> {
    connection.query_row(
        &format!("SELECT {RECEIPT_COLUMNS} FROM agent_execution_receipts WHERE plan_id=?1"),
        params![plan_id],
        map_receipt,
    )
}

fn map_receipt(row: &Row<'_>) -> rusqlite::Result<AgentExecutionReceiptSummary> {
    Ok(AgentExecutionReceiptSummary {
        receipt_id: row.get(0)?,
        plan_id: row.get(1)?,
        approval_id: row.get(2)?,
        plan_hash: row.get(3)?,
        action_type: row.get(4)?,
        connection_id: row.get(5)?,
        idempotency_key: row.get(6)?,
        state: row.get(7)?,
        result_code: row.get(8)?,
        result_summary: row.get(9)?,
        result: parse_json(row.get::<_, String>(10)?),
        started_at_utc: row.get(11)?,
        completed_at_utc: row.get(12)?,
    })
}

fn list_reports(connection: &Connection) -> Result<Vec<AgentReportSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {REPORT_COLUMNS} FROM agent_reports ORDER BY created_at_utc DESC,report_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map([], map_report)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn report_by_id(connection: &Connection, report_id: &str) -> Result<AgentReportSummary> {
    connection
        .query_row(
            &format!("SELECT {REPORT_COLUMNS} FROM agent_reports WHERE report_id=?1"),
            params![report_id],
            map_report,
        )
        .context("agent report was not found")
}

fn map_report(row: &Row<'_>) -> rusqlite::Result<AgentReportSummary> {
    Ok(AgentReportSummary {
        report_id: row.get(0)?,
        plan_id: row.get(1)?,
        title: row.get(2)?,
        content_markdown: row.get(3)?,
        connection_ids: parse_json(row.get::<_, String>(4)?),
        dataset_keys: parse_json(row.get::<_, String>(5)?),
        created_at_utc: row.get(6)?,
    })
}

fn list_missions(connection: &Connection) -> Result<Vec<WorldMissionSummary>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {MISSION_COLUMNS} FROM world_missions ORDER BY created_at_utc DESC,mission_id DESC LIMIT 100"
    ))?;
    let rows = statement.query_map([], map_mission)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn mission_by_id(connection: &Connection, mission_id: &str) -> Result<WorldMissionSummary> {
    connection
        .query_row(
            &format!("SELECT {MISSION_COLUMNS} FROM world_missions WHERE mission_id=?1"),
            params![mission_id],
            map_mission,
        )
        .context("World Mission was not found")
}

fn map_mission(row: &Row<'_>) -> rusqlite::Result<WorldMissionSummary> {
    Ok(WorldMissionSummary {
        mission_id: row.get(0)?,
        thread_id: row.get(1)?,
        goal_id: row.get(2)?,
        connection_id: row.get(3)?,
        world_agent_id: row.get(4)?,
        title: row.get(5)?,
        objective: row.get(6)?,
        allowed_operations: parse_json(row.get::<_, String>(7)?),
        prohibited_operations: parse_json(row.get::<_, String>(8)?),
        limits: parse_json(row.get::<_, String>(9)?),
        disclosure_policy: parse_json(row.get::<_, String>(10)?),
        state: row.get(11)?,
        expires_at_utc: row.get(12)?,
        created_at_utc: row.get(13)?,
        updated_at_utc: row.get(14)?,
    })
}

fn parse_json<T: serde::de::DeserializeOwned + Default>(value: String) -> T {
    serde_json::from_str(&value).unwrap_or_default()
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("agent_task_failed", error.into())
}

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(?error, code, "Agent Workspace operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: "HomeServer could not complete the local Agent Workspace operation.".to_owned(),
        }),
    )
}
