use crate::AppState;
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0023_wrapper_agents_and_action_approvals.sql");
const MIGRATION_KEY: &str = "0023_wrapper_agents_and_action_approvals";
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;
const MAX_PRIVATE_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_EVENTS: i64 = 50_000;
const AUTONOMY_LEVELS: &[u8] = &[0, 1, 2, 3, 4];
const RISK_CLASSES: &[&str] = &[
    "read_only",
    "reversible",
    "external_side_effect",
    "high_risk",
];
const APPROVAL_MODES: &[&str] = &["always", "per_action", "none"];
const TOOL_ADAPTERS: &[&str] = &["proposal_only", "audit.record", "report.save"];
const FORBIDDEN_SAFE_KEYS: &[&str] = &[
    "source_text",
    "full_document",
    "system_prompt",
    "credential",
    "api_key",
    "secret",
    "memory",
    "private_data",
    "file_path",
    "local_path",
    "raw_prompt",
    "conversation",
];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentSummary {
    pub agent_id: String,
    pub owner_user_id: String,
    pub display_name: String,
    pub purpose: String,
    pub description: String,
    pub state: String,
    pub autonomy_level: u8,
    pub revision: u64,
    pub allowed_job_types: Vec<String>,
    pub model_restrictions: Value,
    pub tool_restrictions: Value,
    pub expires_at_utc: String,
    pub activated_at_utc: Option<String>,
    pub suspended_at_utc: Option<String>,
    pub revoked_at_utc: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssignmentSummary {
    pub assignment_id: String,
    pub agent_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub assignment_revision: u64,
    pub state: String,
    pub assigned_by_user_id: String,
    pub purpose: String,
    pub allowed_job_types: Vec<String>,
    pub grant_ids: Vec<String>,
    pub expires_at_utc: String,
    pub revoked_at_utc: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicySummary {
    pub policy_id: String,
    pub agent_id: String,
    pub policy_revision: u64,
    pub action_type: String,
    pub risk_class: String,
    pub approval_mode: String,
    pub tool_adapter: String,
    pub max_executions: u32,
    pub window_seconds: u32,
    pub state: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub created_by_user_id: String,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub revoked_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalSummary {
    pub proposal_id: String,
    pub agent_id: String,
    pub agent_revision: u64,
    pub assignment_id: String,
    pub assignment_revision: u64,
    pub job_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub connection_authority_revision: u64,
    pub authorization_decision_id: String,
    pub policy_id: String,
    pub policy_revision: u64,
    pub action_type: String,
    pub risk_class: String,
    pub title: String,
    pub rationale: String,
    pub safe_summary: Value,
    pub payload_hash: String,
    pub plan_hash: String,
    pub state: String,
    pub approval_required: bool,
    pub expires_at_utc: String,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionApprovalSummary {
    pub approval_id: String,
    pub proposal_id: String,
    pub plan_hash: String,
    pub payload_hash: String,
    pub agent_revision: u64,
    pub assignment_revision: u64,
    pub policy_revision: u64,
    pub grant_revision: u64,
    pub connection_authority_revision: u64,
    pub state: String,
    pub requested_by_user_id: String,
    pub decided_by_user_id: Option<String>,
    pub decision_reason: Option<String>,
    pub requested_at_utc: String,
    pub decided_at_utc: Option<String>,
    pub consumed_at_utc: Option<String>,
    pub expires_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionReceiptSummary {
    pub receipt_id: String,
    pub proposal_id: String,
    pub attempt_id: String,
    pub agent_id: String,
    pub agent_revision: u64,
    pub assignment_id: String,
    pub assignment_revision: u64,
    pub job_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub connection_authority_revision: u64,
    pub authorization_decision_id: String,
    pub policy_id: String,
    pub policy_revision: u64,
    pub approval_id: Option<String>,
    pub plan_hash: String,
    pub payload_hash: String,
    pub action_type: String,
    pub risk_class: String,
    pub tool_adapter: String,
    pub outcome: String,
    pub result_code: String,
    pub safe_result_hash: Option<String>,
    pub receipt_hash: String,
    pub completed_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmergencyStopSummary {
    pub stop_id: String,
    pub scope_type: String,
    pub agent_id: Option<String>,
    pub wrapper_id: Option<String>,
    pub connection_id: Option<String>,
    pub state: String,
    pub reason: String,
    pub stop_hash: String,
    pub activated_by_user_id: String,
    pub activated_at_utc: String,
    pub expires_at_utc: Option<String>,
    pub released_by_user_id: Option<String>,
    pub released_at_utc: Option<String>,
    pub release_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentRegistrySnapshot {
    pub schema: String,
    pub connection_id: Option<String>,
    pub agents: Vec<AgentSummary>,
    pub assignments: Vec<AssignmentSummary>,
    pub policies: Vec<PolicySummary>,
    pub proposals: Vec<ProposalSummary>,
    pub approvals: Vec<ActionApprovalSummary>,
    pub receipts: Vec<ActionReceiptSummary>,
    pub emergency_stops: Vec<EmergencyStopSummary>,
    pub private_payloads_exposed: bool,
    pub private_results_exposed: bool,
    pub pairing_implies_agent_authority: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyInput {
    pub action_type: String,
    pub risk_class: String,
    pub approval_mode: String,
    pub tool_adapter: String,
    pub max_executions: Option<u32>,
    pub window_seconds: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAgentRequest {
    pub owner_user_id: String,
    pub display_name: String,
    pub purpose: String,
    #[serde(default)]
    pub description: String,
    pub autonomy_level: u8,
    pub allowed_job_types: Vec<String>,
    #[serde(default = "empty_object")]
    pub model_restrictions: Value,
    #[serde(default = "empty_object")]
    pub tool_restrictions: Value,
    #[serde(default)]
    pub policies: Vec<PolicyInput>,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentLifecycleRequest {
    pub agent_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePolicyRequest {
    pub agent_id: String,
    pub actor_user_id: String,
    pub policy: PolicyInput,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevokePolicyRequest {
    pub policy_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAssignmentRequest {
    pub agent_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub assigned_by_user_id: String,
    pub purpose: String,
    pub allowed_job_types: Vec<String>,
    pub grant_ids: Vec<String>,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevokeAssignmentRequest {
    pub assignment_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateProposalRequest {
    pub agent_id: String,
    pub assignment_id: String,
    pub job_id: String,
    pub policy_id: String,
    pub title: String,
    pub rationale: String,
    #[serde(default = "empty_object")]
    pub safe_summary: Value,
    pub private_payload: Value,
    pub requested_by_user_id: String,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProposalDecisionRequest {
    pub proposal_id: String,
    pub plan_hash: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteProposalRequest {
    pub proposal_id: String,
    pub plan_hash: String,
    pub actor_user_id: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionSnapshotRequest {
    pub connection_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivateEmergencyStopRequest {
    pub scope_type: String,
    pub agent_id: Option<String>,
    pub wrapper_id: Option<String>,
    pub connection_id: Option<String>,
    pub actor_user_id: String,
    pub reason: String,
    pub expires_minutes: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseEmergencyStopRequest {
    pub stop_id: String,
    pub stop_hash: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentSubmissionBinding {
    agent_id: String,
    agent_revision: u64,
    assignment_id: String,
    assignment_revision: u64,
    binding_id: String,
    policy_context_hash: String,
}

#[derive(Debug, Clone)]
struct ProposalAuthority {
    proposal: ProposalSummary,
    tool_adapter: String,
    autonomy_level: u8,
}

struct PolicyInsertContext<'a> {
    agent_id: &'a str,
    revision: u64,
    autonomy_level: u8,
    actor: &'a str,
    not_before: &'a str,
    expires: &'a str,
}

type AgentJobAuthorityRow = (
    String,
    i64,
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

type ProposalAuthorityRow = (
    String,
    i64,
    String,
    i64,
    String,
    i64,
    String,
    i64,
    String,
    i64,
    String,
    String,
    String,
    String,
);

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    expire_and_reconcile(connection)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        count == 1,
        "agent lifecycle migration is not registered exactly once"
    );
    for table in [
        "homeserver_agents",
        "wrapper_agent_assignments",
        "agent_capability_bindings",
        "agent_execution_policies",
        "agent_job_bindings",
        "agent_action_proposals",
        "agent_action_private_payloads",
        "agent_action_approvals",
        "agent_action_attempts",
        "agent_action_private_results",
        "agent_action_receipts",
        "agent_lifecycle_events",
        "agent_emergency_stops",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let cross_wrapper: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_agent_assignments a JOIN wrapper_connections c ON c.connection_id=a.connection_id WHERE c.wrapper_id<>a.wrapper_id",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        cross_wrapper == 0,
        "agent assignments contain cross-wrapper bindings"
    );
    let stale: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_action_proposals p JOIN homeserver_agents a ON a.agent_id=p.agent_id WHERE p.state IN ('proposed','awaiting_approval','approved','executing') AND (a.state<>'active' OR a.revision<>p.agent_revision)",
        [],
        |row| row.get(0),
    )?;
    ensure!(stale == 0, "stale agent action proposals remain executable");
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    expire_and_reconcile(connection)?;
    connection.execute(
        "DELETE FROM agent_lifecycle_events WHERE created_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM agent_lifecycle_events WHERE event_id NOT IN (SELECT event_id FROM agent_lifecycle_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1)",
        params![MAX_EVENTS],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/agents", get(snapshot_handler))
        .route(
            "/v1/agents/connection-snapshot",
            post(connection_snapshot_handler),
        )
        .route("/v1/agents/create", post(create_agent_handler))
        .route("/v1/agents/activate", post(activate_agent_handler))
        .route("/v1/agents/suspend", post(suspend_agent_handler))
        .route("/v1/agents/revoke", post(revoke_agent_handler))
        .route("/v1/agents/policies/create", post(create_policy_handler))
        .route("/v1/agents/policies/revoke", post(revoke_policy_handler))
        .route(
            "/v1/agents/assignments/create",
            post(create_assignment_handler),
        )
        .route(
            "/v1/agents/assignments/revoke",
            post(revoke_assignment_handler),
        )
        .route("/v1/action-proposals", get(snapshot_handler))
        .route("/v1/action-proposals/create", post(create_proposal_handler))
        .route(
            "/v1/action-proposals/approve",
            post(approve_proposal_handler),
        )
        .route("/v1/action-proposals/reject", post(reject_proposal_handler))
        .route(
            "/v1/action-proposals/execute",
            post(execute_proposal_handler),
        )
        .route("/v1/agents/emergency-stop", post(activate_stop_handler))
        .route(
            "/v1/agents/emergency-stop/release",
            post(release_stop_handler),
        )
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn snapshot_handler(State(state): State<Arc<AppState>>) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || snapshot(&state, None),
        "agent_registry_snapshot_failed",
    )
    .await
}

async fn connection_snapshot_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionSnapshotRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
            snapshot(&state, Some(&connection_id))
        },
        "agent_connection_snapshot_failed",
    )
    .await
}

async fn create_agent_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateAgentRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            create_agent(&connection, request)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_create_failed",
    )
    .await
}

macro_rules! lifecycle_handler {
    ($name:ident, $function:ident, $code:literal) => {
        async fn $name(
            State(state): State<Arc<AppState>>,
            Json(request): Json<AgentLifecycleRequest>,
        ) -> ApiResult<AgentRegistrySnapshot> {
            run_blocking(
                move || {
                    let connection = state.connection()?;
                    $function(&connection, request)?;
                    super::wrapper_jobs::reconcile_authority(&connection)?;
                    snapshot_with_connection(&connection, None)
                },
                $code,
            )
            .await
        }
    };
}

lifecycle_handler!(
    activate_agent_handler,
    activate_agent,
    "agent_activate_failed"
);
lifecycle_handler!(suspend_agent_handler, suspend_agent, "agent_suspend_failed");
lifecycle_handler!(revoke_agent_handler, revoke_agent, "agent_revoke_failed");

async fn create_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreatePolicyRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            create_policy(&connection, request)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_policy_create_failed",
    )
    .await
}

async fn revoke_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RevokePolicyRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            revoke_policy(&connection, request)?;
            expire_and_reconcile(&connection)?;
            super::wrapper_jobs::reconcile_authority(&connection)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_policy_revoke_failed",
    )
    .await
}

async fn create_assignment_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateAssignmentRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            create_assignment(&connection, request)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_assignment_create_failed",
    )
    .await
}

async fn revoke_assignment_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RevokeAssignmentRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            revoke_assignment(&connection, request)?;
            expire_and_reconcile(&connection)?;
            super::wrapper_jobs::reconcile_authority(&connection)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_assignment_revoke_failed",
    )
    .await
}

async fn create_proposal_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateProposalRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            create_proposal(&connection, request)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_action_proposal_failed",
    )
    .await
}

async fn approve_proposal_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProposalDecisionRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            approve_proposal(&connection, request)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_action_approval_failed",
    )
    .await
}

async fn reject_proposal_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ProposalDecisionRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            reject_proposal(&connection, request)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_action_rejection_failed",
    )
    .await
}

async fn execute_proposal_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ExecuteProposalRequest>,
) -> ApiResult<ActionReceiptSummary> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            execute_proposal(&connection, request)
        },
        "agent_action_execution_failed",
    )
    .await
}

async fn activate_stop_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ActivateEmergencyStopRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            activate_emergency_stop(&connection, request)?;
            expire_and_reconcile(&connection)?;
            super::wrapper_jobs::reconcile_authority(&connection)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_emergency_stop_failed",
    )
    .await
}

async fn release_stop_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ReleaseEmergencyStopRequest>,
) -> ApiResult<AgentRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            release_emergency_stop(&connection, request)?;
            snapshot_with_connection(&connection, None)
        },
        "agent_emergency_stop_release_failed",
    )
    .await
}

async fn run_blocking<T, F>(task: F, code: &'static str) -> ApiResult<T>
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| api_error(code, anyhow::anyhow!("agent task failed: {error}")))?
        .map(Json)
        .map_err(|error| api_error(code, error))
}

fn create_agent(connection: &Connection, request: CreateAgentRequest) -> Result<String> {
    ensure!(
        AUTONOMY_LEVELS.contains(&request.autonomy_level),
        "invalid autonomy level"
    );
    ensure!(
        (1..=525_600).contains(&request.expires_minutes),
        "agent expiration must be between one minute and one year"
    );
    let owner_user_id = bounded_text(&request.owner_user_id, 1, 160, "owner user ID")?;
    let display_name = bounded_text(&request.display_name, 1, 120, "display name")?;
    let purpose = bounded_text(&request.purpose, 1, 500, "purpose")?;
    let description = bounded_text(&request.description, 0, 4_000, "description")?;
    let allowed_job_types = validate_symbol_list(request.allowed_job_types, 64, 80, "job type")?;
    ensure!(
        !allowed_job_types.is_empty(),
        "an agent must declare at least one job type"
    );
    validate_restrictions(&request.model_restrictions, "model restrictions")?;
    validate_restrictions(&request.tool_restrictions, "tool restrictions")?;
    ensure!(request.policies.len() <= 64, "too many execution policies");
    let agent_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let now_text = timestamp(now);
    let expires = timestamp(now + Duration::minutes(i64::from(request.expires_minutes)));
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO homeserver_agents (agent_id,owner_user_id,display_name,purpose,description,state,autonomy_level,revision,allowed_job_types_json,model_restrictions_json,tool_restrictions_json,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,'draft',?6,1,?7,?8,?9,?10,?11,?11)",
        params![
            agent_id,
            owner_user_id,
            display_name,
            purpose,
            description,
            i64::from(request.autonomy_level),
            serde_json::to_string(&allowed_job_types)?,
            json_text(&request.model_restrictions)?,
            json_text(&request.tool_restrictions)?,
            expires,
            now_text
        ],
    )?;
    for policy in request.policies {
        insert_policy_tx(
            &transaction,
            PolicyInsertContext {
                agent_id: &agent_id,
                revision: 1,
                autonomy_level: request.autonomy_level,
                actor: &owner_user_id,
                not_before: &now_text,
                expires: &expires,
            },
            policy,
        )?;
    }
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&agent_id),
            wrapper_id: None,
            connection_id: None,
            assignment_id: None,
            proposal_id: None,
            event_type: "agent.created",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &owner_user_id,
            detail_code: "draft_created",
            metadata: json!({"autonomy_level": request.autonomy_level}),
        },
    )?;
    transaction.commit()?;
    Ok(agent_id)
}

fn activate_agent(connection: &Connection, request: AgentLifecycleRequest) -> Result<()> {
    ensure!(
        request.confirmation == "ACTIVATE AGENT",
        "agent activation confirmation is invalid"
    );
    let agent_id = validate_uuid(&request.agent_id, "agent ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let transaction = connection.unchecked_transaction()?;
    let (state, expires_at): (String, String) = transaction.query_row(
        "SELECT state,expires_at_utc FROM homeserver_agents WHERE agent_id=?1",
        params![agent_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    ensure!(
        matches!(state.as_str(), "draft" | "suspended"),
        "agent cannot be activated from its current state"
    );
    ensure!(
        parse_utc(&expires_at, "agent expiration")? > Utc::now(),
        "agent has expired"
    );
    transaction.execute(
        "UPDATE homeserver_agents SET state='active',revision=revision+1,activated_at_utc=?1,suspended_at_utc=NULL,updated_at_utc=?1 WHERE agent_id=?2",
        params![now_utc(), agent_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&agent_id),
            wrapper_id: None,
            connection_id: None,
            assignment_id: None,
            proposal_id: None,
            event_type: "agent.activated",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: "agent_active",
            metadata: json!({}),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn suspend_agent(connection: &Connection, request: AgentLifecycleRequest) -> Result<()> {
    ensure!(
        request.confirmation == "SUSPEND AGENT",
        "agent suspension confirmation is invalid"
    );
    transition_agent_to_blocked(connection, request, "suspended", "agent.suspended")
}

fn revoke_agent(connection: &Connection, request: AgentLifecycleRequest) -> Result<()> {
    ensure!(
        request.confirmation == "REVOKE AGENT",
        "agent revocation confirmation is invalid"
    );
    transition_agent_to_blocked(connection, request, "revoked", "agent.revoked")
}

fn transition_agent_to_blocked(
    connection: &Connection,
    request: AgentLifecycleRequest,
    target_state: &str,
    event_type: &str,
) -> Result<()> {
    let agent_id = validate_uuid(&request.agent_id, "agent ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(
        request.reason.as_deref().unwrap_or("user_requested"),
        1,
        500,
        "reason",
    )?;
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    let state: String = transaction.query_row(
        "SELECT state FROM homeserver_agents WHERE agent_id=?1",
        params![agent_id],
        |row| row.get(0),
    )?;
    ensure!(state != "revoked", "revoked agent cannot change state");
    transaction.execute(
        "UPDATE homeserver_agents SET state=?1,revision=revision+1,suspended_at_utc=CASE WHEN ?1='suspended' THEN ?2 ELSE suspended_at_utc END,revoked_at_utc=CASE WHEN ?1='revoked' THEN ?2 ELSE revoked_at_utc END,updated_at_utc=?2 WHERE agent_id=?3",
        params![target_state, now, agent_id],
    )?;
    transaction.execute(
        "UPDATE wrapper_agent_assignments SET state=CASE WHEN ?1='revoked' THEN 'revoked' ELSE 'suspended' END,assignment_revision=assignment_revision+1,revoked_at_utc=CASE WHEN ?1='revoked' THEN ?2 ELSE revoked_at_utc END,updated_at_utc=?2 WHERE agent_id=?3 AND state IN ('active','suspended')",
        params![target_state, now, agent_id],
    )?;
    cancel_agent_proposals_tx(&transaction, &agent_id, &format!("agent_{target_state}"))?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&agent_id),
            wrapper_id: None,
            connection_id: None,
            assignment_id: None,
            proposal_id: None,
            event_type,
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: &reason,
            metadata: json!({"previous_state": state, "target_state": target_state}),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn create_policy(connection: &Connection, request: CreatePolicyRequest) -> Result<String> {
    ensure!(
        (1..=525_600).contains(&request.expires_minutes),
        "policy expiration must be between one minute and one year"
    );
    let agent_id = validate_uuid(&request.agent_id, "agent ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let transaction = connection.unchecked_transaction()?;
    let (autonomy, agent_expires): (i64, String) = transaction.query_row(
        "SELECT autonomy_level,expires_at_utc FROM homeserver_agents WHERE agent_id=?1 AND state<>'revoked'",
        params![agent_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let requested_expires = Utc::now() + Duration::minutes(i64::from(request.expires_minutes));
    let expires = timestamp(requested_expires.min(parse_utc(&agent_expires, "agent expiration")?));
    let now = now_utc();
    let next_revision: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(policy_revision),0)+1 FROM agent_execution_policies WHERE agent_id=?1 AND action_type=?2",
        params![agent_id, request.policy.action_type],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE agent_execution_policies SET state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE agent_id=?2 AND action_type=?3 AND state='active'",
        params![now, agent_id, request.policy.action_type],
    )?;
    let policy_id = insert_policy_tx(
        &transaction,
        PolicyInsertContext {
            agent_id: &agent_id,
            revision: next_revision.max(1) as u64,
            autonomy_level: autonomy.max(0) as u8,
            actor: &actor,
            not_before: &now,
            expires: &expires,
        },
        request.policy,
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&agent_id),
            wrapper_id: None,
            connection_id: None,
            assignment_id: None,
            proposal_id: None,
            event_type: "agent.policy_created",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: "policy_active",
            metadata: json!({"policy_id": policy_id}),
        },
    )?;
    transaction.commit()?;
    Ok(policy_id)
}

fn insert_policy_tx(
    transaction: &Transaction<'_>,
    context: PolicyInsertContext<'_>,
    policy: PolicyInput,
) -> Result<String> {
    let action_type = validate_symbol(&policy.action_type, 120, "action type")?;
    let risk_class = validate_enum(&policy.risk_class, RISK_CLASSES, "risk class")?;
    let approval_mode = validate_enum(&policy.approval_mode, APPROVAL_MODES, "approval mode")?;
    let tool_adapter = validate_enum(&policy.tool_adapter, TOOL_ADAPTERS, "tool adapter")?;
    let max_executions = policy.max_executions.unwrap_or(1);
    let window_seconds = policy.window_seconds.unwrap_or(3_600);
    ensure!(
        (1..=10_000).contains(&max_executions),
        "invalid policy execution limit"
    );
    ensure!(
        (60..=2_592_000).contains(&window_seconds),
        "invalid policy window"
    );
    if matches!(risk_class.as_str(), "external_side_effect" | "high_risk") {
        ensure!(
            approval_mode != "none",
            "sensitive actions always require approval"
        );
    }
    if context.autonomy_level <= 1 {
        ensure!(
            tool_adapter == "proposal_only",
            "suggest-only agents cannot receive executable adapters"
        );
    }
    if context.autonomy_level == 2 {
        ensure!(
            approval_mode != "none",
            "approval-required agents cannot use approval-free policies"
        );
    }
    if approval_mode == "none" {
        ensure!(
            context.autonomy_level >= 3,
            "approval-free policy requires scoped autonomy"
        );
        ensure!(
            matches!(risk_class.as_str(), "read_only" | "reversible"),
            "approval-free policy is limited to low-risk actions"
        );
    }
    let policy_id = Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO agent_execution_policies (policy_id,agent_id,policy_revision,action_type,risk_class,approval_mode,tool_adapter,max_executions,window_seconds,state,not_before_utc,expires_at_utc,created_by_user_id,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'active',?10,?11,?12,?10,?10)",
        params![
            policy_id,
            context.agent_id,
            context.revision as i64,
            action_type,
            risk_class,
            approval_mode,
            tool_adapter,
            i64::from(max_executions),
            i64::from(window_seconds),
            context.not_before,
            context.expires,
            context.actor
        ],
    )?;
    Ok(policy_id)
}

fn revoke_policy(connection: &Connection, request: RevokePolicyRequest) -> Result<()> {
    ensure!(
        request.confirmation == "REVOKE POLICY",
        "policy revocation confirmation is invalid"
    );
    let policy_id = validate_uuid(&request.policy_id, "policy ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 500, "reason")?;
    let transaction = connection.unchecked_transaction()?;
    let agent_id: String = transaction.query_row(
        "SELECT agent_id FROM agent_execution_policies WHERE policy_id=?1 AND state='active'",
        params![policy_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE agent_execution_policies SET state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE policy_id=?2",
        params![now_utc(), policy_id],
    )?;
    transaction.execute(
        "UPDATE agent_action_proposals SET state='cancelled',failure_code='policy_revoked',completed_at_utc=?1,updated_at_utc=?1 WHERE policy_id=?2 AND state IN ('proposed','awaiting_approval','approved','executing')",
        params![now_utc(), policy_id],
    )?;
    transaction.execute(
        "UPDATE agent_action_approvals SET state='cancelled' WHERE proposal_id IN (SELECT proposal_id FROM agent_action_proposals WHERE policy_id=?1) AND state IN ('pending','approved')",
        params![policy_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&agent_id),
            wrapper_id: None,
            connection_id: None,
            assignment_id: None,
            proposal_id: None,
            event_type: "agent.policy_revoked",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: &reason,
            metadata: json!({"policy_id": policy_id}),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn create_assignment(connection: &Connection, request: CreateAssignmentRequest) -> Result<String> {
    ensure!(
        (1..=525_600).contains(&request.expires_minutes),
        "assignment expiration must be between one minute and one year"
    );
    ensure!(
        !request.grant_ids.is_empty() && request.grant_ids.len() <= 64,
        "assignment must bind between one and 64 grants"
    );
    let agent_id = validate_uuid(&request.agent_id, "agent ID")?;
    let wrapper_id = validate_uuid(&request.wrapper_id, "wrapper ID")?;
    let connection_id = validate_uuid(&request.connection_id, "connection ID")?;
    let actor = bounded_text(&request.assigned_by_user_id, 1, 160, "assigned-by user ID")?;
    let purpose = bounded_text(&request.purpose, 1, 500, "assignment purpose")?;
    let allowed_jobs = validate_symbol_list(request.allowed_job_types, 64, 80, "job type")?;
    ensure!(
        !allowed_jobs.is_empty(),
        "assignment must declare at least one job type"
    );
    let transaction = connection.unchecked_transaction()?;
    let (agent_state, agent_revision, agent_jobs_json, agent_expires): (String, i64, String, String) =
        transaction.query_row(
            "SELECT state,revision,allowed_job_types_json,expires_at_utc FROM homeserver_agents WHERE agent_id=?1",
            params![agent_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    ensure!(
        agent_state == "active",
        "agent must be active before assignment"
    );
    let agent_jobs = parse_string_list(&agent_jobs_json);
    ensure!(
        allowed_jobs.iter().all(|item| agent_jobs.contains(item)),
        "assignment job types exceed the agent definition"
    );
    let connection_context: (String, String, String) = transaction.query_row(
        "SELECT c.wrapper_id,c.lifecycle_state,w.state FROM wrapper_connections c JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id WHERE c.connection_id=?1",
        params![connection_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    ensure!(
        connection_context.0 == wrapper_id,
        "connection belongs to a different wrapper"
    );
    ensure!(
        matches!(
            connection_context.1.as_str(),
            "active" | "offline" | "grace"
        ),
        "wrapper connection is not assignable"
    );
    ensure!(connection_context.2 == "active", "wrapper is not active");
    let expires_at = timestamp(
        (Utc::now() + Duration::minutes(i64::from(request.expires_minutes)))
            .min(parse_utc(&agent_expires, "agent expiration")?),
    );
    let now = now_utc();
    let assignment_id = Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO wrapper_agent_assignments (assignment_id,agent_id,wrapper_id,connection_id,assignment_revision,state,assigned_by_user_id,purpose,allowed_job_types_json,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,1,'active',?5,?6,?7,?8,?9,?9)",
        params![assignment_id, agent_id, wrapper_id, connection_id, actor, purpose, serde_json::to_string(&allowed_jobs)?, expires_at, now],
    )?;
    let mut seen = BTreeSet::new();
    for raw_grant_id in request.grant_ids {
        let grant_id = validate_uuid(&raw_grant_id, "grant ID")?;
        ensure!(seen.insert(grant_id.clone()), "duplicate grant binding");
        let (grant_wrapper, grant_connection, capability_key, grant_revision, operations_json, grant_expires): (String, String, String, i64, String, String) =
            transaction.query_row(
                "SELECT wrapper_id,connection_id,capability_key,grant_revision,allowed_operations_json,expires_at_utc FROM wrapper_capability_grants WHERE grant_id=?1 AND state='active' AND not_before_utc<=?2 AND expires_at_utc>?2",
                params![grant_id, now],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
            )?;
        ensure!(
            grant_wrapper == wrapper_id && grant_connection == connection_id,
            "grant belongs to a different wrapper connection"
        );
        let binding_id = Uuid::new_v4().to_string();
        let binding_expires = if parse_utc(&grant_expires, "grant expiration")?
            < parse_utc(&expires_at, "assignment expiration")?
        {
            grant_expires
        } else {
            expires_at.clone()
        };
        transaction.execute(
            "INSERT INTO agent_capability_bindings (binding_id,assignment_id,grant_id,grant_revision,capability_key,allowed_operations_json,state,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'active',?7,?8,?8)",
            params![binding_id, assignment_id, grant_id, grant_revision, capability_key, operations_json, binding_expires, now],
        )?;
    }
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&agent_id),
            wrapper_id: Some(&wrapper_id),
            connection_id: Some(&connection_id),
            assignment_id: Some(&assignment_id),
            proposal_id: None,
            event_type: "agent.assignment_created",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: "scoped_assignment_active",
            metadata: json!({"agent_revision": agent_revision, "grant_count": seen.len()}),
        },
    )?;
    transaction.commit()?;
    Ok(assignment_id)
}

fn revoke_assignment(connection: &Connection, request: RevokeAssignmentRequest) -> Result<()> {
    ensure!(
        request.confirmation == "REVOKE ASSIGNMENT",
        "assignment revocation confirmation is invalid"
    );
    let assignment_id = validate_uuid(&request.assignment_id, "assignment ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 500, "reason")?;
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    let context: (String, String, String) = transaction.query_row(
        "SELECT agent_id,wrapper_id,connection_id FROM wrapper_agent_assignments WHERE assignment_id=?1 AND state IN ('active','suspended')",
        params![assignment_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    transaction.execute(
        "UPDATE wrapper_agent_assignments SET state='revoked',assignment_revision=assignment_revision+1,revoked_at_utc=?1,updated_at_utc=?1 WHERE assignment_id=?2",
        params![now, assignment_id],
    )?;
    transaction.execute(
        "UPDATE agent_capability_bindings SET state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE assignment_id=?2 AND state IN ('active','suspended')",
        params![now, assignment_id],
    )?;
    transaction.execute(
        "UPDATE agent_action_proposals SET state='cancelled',failure_code='assignment_revoked',completed_at_utc=?1,updated_at_utc=?1 WHERE assignment_id=?2 AND state IN ('proposed','awaiting_approval','approved','executing')",
        params![now, assignment_id],
    )?;
    transaction.execute(
        "UPDATE agent_action_approvals SET state='cancelled' WHERE proposal_id IN (SELECT proposal_id FROM agent_action_proposals WHERE assignment_id=?1) AND state IN ('pending','approved')",
        params![assignment_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&context.0),
            wrapper_id: Some(&context.1),
            connection_id: Some(&context.2),
            assignment_id: Some(&assignment_id),
            proposal_id: None,
            event_type: "agent.assignment_revoked",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: &reason,
            metadata: json!({}),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

pub(crate) fn validate_agent_job_submission(
    connection: &Connection,
    agent_id: &str,
    connection_id: &str,
    grant_id: &str,
    capability_key: &str,
    operation: &str,
    job_type: &str,
) -> Result<AgentSubmissionBinding> {
    let agent_id = validate_uuid(agent_id, "agent ID")?;
    let connection_id = validate_uuid(connection_id, "connection ID")?;
    let grant_id = validate_uuid(grant_id, "grant ID")?;
    let capability_key = validate_symbol(capability_key, 120, "capability key")?;
    let operation = validate_symbol(operation, 80, "operation")?;
    let now = now_utc();
    let context: (i64, String, i64, String, String, i64, String, String, String, String) = connection.query_row(
        "SELECT a.revision,x.assignment_id,x.assignment_revision,b.binding_id,x.allowed_job_types_json,b.grant_revision,b.allowed_operations_json,a.state,x.state,b.capability_key FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.agent_id=a.agent_id AND x.connection_id=?2 JOIN agent_capability_bindings b ON b.assignment_id=x.assignment_id AND b.grant_id=?3 WHERE a.agent_id=?1 AND a.expires_at_utc>?4 AND x.expires_at_utc>?4 AND b.expires_at_utc>?4",
        params![agent_id, connection_id, grant_id, now],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?)),
    )?;
    ensure!(
        context.7 == "active" && context.8 == "active",
        "agent or assignment is not active"
    );
    ensure!(
        context.9 == capability_key,
        "capability binding does not match the authorized grant"
    );
    ensure!(
        parse_string_list(&context.6)
            .iter()
            .any(|item| item == &operation),
        "operation is not bound to the agent"
    );
    let autonomy: i64 = connection.query_row(
        "SELECT autonomy_level FROM homeserver_agents WHERE agent_id=?1",
        params![agent_id],
        |row| row.get(0),
    )?;
    ensure!(autonomy > 0, "disabled agent cannot submit jobs");
    ensure!(
        parse_string_list(&context.4)
            .iter()
            .any(|item| item == job_type),
        "job type is not assigned to the agent"
    );
    let binding_state: String = connection.query_row(
        "SELECT state FROM agent_capability_bindings WHERE binding_id=?1",
        params![context.3],
        |row| row.get(0),
    )?;
    ensure!(
        binding_state == "active",
        "agent capability binding is not active"
    );
    ensure!(
        !emergency_stop_active(connection, &agent_id, &connection_id)?,
        "agent authority is stopped"
    );
    let policy_context_hash = hash_json(&json!({
        "agent_id": agent_id,
        "agent_revision": context.0,
        "assignment_id": context.1,
        "assignment_revision": context.2,
        "binding_id": context.3,
        "grant_id": grant_id,
        "grant_revision": context.5,
        "capability_key": capability_key,
        "operation": operation,
        "job_type": job_type
    }))?;
    Ok(AgentSubmissionBinding {
        agent_id,
        agent_revision: context.0.max(0) as u64,
        assignment_id: context.1,
        assignment_revision: context.2.max(0) as u64,
        binding_id: context.3,
        policy_context_hash,
    })
}

pub(crate) fn bind_agent_job_tx(
    transaction: &Transaction<'_>,
    job_id: &str,
    binding: &AgentSubmissionBinding,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO agent_job_bindings (job_id,agent_id,agent_revision,assignment_id,assignment_revision,binding_id,policy_context_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            job_id,
            binding.agent_id,
            binding.agent_revision as i64,
            binding.assignment_id,
            binding.assignment_revision as i64,
            binding.binding_id,
            binding.policy_context_hash,
            now_utc()
        ],
    )?;
    Ok(())
}

pub(crate) fn agent_job_authority_is_current_tx(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<bool> {
    let binding: Option<AgentJobAuthorityRow> =
        transaction.query_row(
            "SELECT b.agent_id,b.agent_revision,b.assignment_id,b.assignment_revision,a.state,a.expires_at_utc,x.state,x.expires_at_utc,c.state,c.expires_at_utc,j.connection_id FROM agent_job_bindings b JOIN homeserver_agents a ON a.agent_id=b.agent_id JOIN wrapper_agent_assignments x ON x.assignment_id=b.assignment_id JOIN agent_capability_bindings c ON c.binding_id=b.binding_id JOIN wrapper_jobs j ON j.job_id=b.job_id WHERE b.job_id=?1",
            params![job_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?)),
        ).optional()?;
    let Some((
        agent_id,
        captured_agent_revision,
        assignment_id,
        captured_assignment_revision,
        agent_state,
        agent_expires,
        assignment_state,
        assignment_expires,
        binding_state,
        binding_expires,
        connection_id,
    )) = binding
    else {
        return Ok(true);
    };
    let current: (i64, i64) = transaction.query_row(
        "SELECT a.revision,x.assignment_revision FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.assignment_id=?2 AND x.agent_id=a.agent_id WHERE a.agent_id=?1",
        params![agent_id, assignment_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(agent_state == "active"
        && assignment_state == "active"
        && binding_state == "active"
        && current.0 == captured_agent_revision
        && current.1 == captured_assignment_revision
        && parse_utc(&agent_expires, "agent expiration")? > Utc::now()
        && parse_utc(&assignment_expires, "assignment expiration")? > Utc::now()
        && parse_utc(&binding_expires, "binding expiration")? > Utc::now()
        && !emergency_stop_active_tx(transaction, &agent_id, &connection_id)?)
}

pub(crate) fn create_proposal(
    connection: &Connection,
    request: CreateProposalRequest,
) -> Result<String> {
    expire_and_reconcile(connection)?;
    ensure!(
        (1..=10_080).contains(&request.expires_minutes),
        "proposal expiration must be between one minute and seven days"
    );
    let agent_id = validate_uuid(&request.agent_id, "agent ID")?;
    let assignment_id = validate_uuid(&request.assignment_id, "assignment ID")?;
    let job_id = validate_uuid(&request.job_id, "job ID")?;
    let policy_id = validate_uuid(&request.policy_id, "policy ID")?;
    let title = bounded_text(&request.title, 1, 180, "title")?;
    let rationale = bounded_text(&request.rationale, 1, 4_000, "rationale")?;
    let actor = bounded_text(&request.requested_by_user_id, 1, 160, "requesting user ID")?;
    ensure_safe_value(&request.safe_summary, 0)?;
    let private_payload_text = json_text(&request.private_payload)?;
    ensure!(
        (2..=MAX_PRIVATE_PAYLOAD_BYTES).contains(&private_payload_text.len()),
        "private action payload exceeds the HomeServer limit"
    );
    let payload_hash = hash_text(&private_payload_text);
    let transaction = connection.unchecked_transaction()?;
    let job: (String, String, String, i64, i64, String, String, String, String, String) = transaction.query_row(
        "SELECT j.wrapper_id,j.connection_id,j.grant_id,j.grant_revision,s.connection_authority_revision,j.authorization_decision_id,j.capability_key,j.operation,j.state,j.result_policy FROM wrapper_jobs j JOIN wrapper_job_authority_snapshots s ON s.job_id=j.job_id WHERE j.job_id=?1",
        params![job_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?)),
    )?;
    ensure!(
        job.6 == "action.propose" && job.7 == "propose",
        "job is not an action proposal job"
    );
    ensure!(
        job.8 == "completed" && job.9 == "proposal_only",
        "action proposal job is not safely completed"
    );
    let binding: (String, i64, i64, String) = transaction.query_row(
        "SELECT b.assignment_id,b.agent_revision,b.assignment_revision,b.agent_id FROM agent_job_bindings b WHERE b.job_id=?1",
        params![job_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    ensure!(
        binding.0 == assignment_id && binding.3 == agent_id,
        "job is bound to a different agent assignment"
    );
    let assignment: (String, String, String, i64) = transaction.query_row(
        "SELECT wrapper_id,connection_id,state,assignment_revision FROM wrapper_agent_assignments WHERE assignment_id=?1",
        params![assignment_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    )?;
    ensure!(
        assignment.0 == job.0 && assignment.1 == job.1 && assignment.2 == "active",
        "assignment does not authorize the job connection"
    );
    let (policy_agent, policy_revision, action_type, risk_class, approval_mode, tool_adapter, policy_state, policy_not_before, policy_expires): (String, i64, String, String, String, String, String, String, String) =
        transaction.query_row(
            "SELECT agent_id,policy_revision,action_type,risk_class,approval_mode,tool_adapter,state,not_before_utc,expires_at_utc FROM agent_execution_policies WHERE policy_id=?1",
            params![policy_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?)),
        )?;
    ensure!(
        policy_agent == agent_id && policy_state == "active",
        "action policy is not active for the agent"
    );
    ensure!(
        parse_utc(&policy_not_before, "policy start")? <= Utc::now(),
        "action policy is not active yet"
    );
    let (agent_state, agent_revision, autonomy_level, agent_expires): (String, i64, i64, String) = transaction.query_row(
        "SELECT state,revision,autonomy_level,expires_at_utc FROM homeserver_agents WHERE agent_id=?1",
        params![agent_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    )?;
    ensure!(
        agent_state == "active" && agent_revision == binding.1,
        "agent authority changed after job submission"
    );
    ensure!(
        assignment.3 == binding.2,
        "assignment authority changed after job submission"
    );
    ensure!(
        !emergency_stop_active_tx(&transaction, &agent_id, &job.1)?,
        "agent authority is stopped"
    );
    let approval_required = matches!(risk_class.as_str(), "external_side_effect" | "high_risk")
        || approval_mode != "none"
        || autonomy_level < 3;
    let requested_expiration = Utc::now() + Duration::minutes(i64::from(request.expires_minutes));
    let expires = requested_expiration
        .min(parse_utc(&policy_expires, "policy expiration")?)
        .min(parse_utc(&agent_expires, "agent expiration")?);
    ensure!(expires > Utc::now(), "proposal authority has expired");
    let proposal_id = Uuid::new_v4().to_string();
    let plan_hash = hash_json(&json!({
        "schema": "homeserver.agent-action-plan.v1",
        "proposal_id": proposal_id,
        "agent_id": agent_id,
        "agent_revision": agent_revision,
        "assignment_id": assignment_id,
        "assignment_revision": assignment.3,
        "job_id": job_id,
        "wrapper_id": job.0,
        "connection_id": job.1,
        "grant_id": job.2,
        "grant_revision": job.3,
        "connection_authority_revision": job.4,
        "authorization_decision_id": job.5,
        "policy_id": policy_id,
        "policy_revision": policy_revision,
        "action_type": action_type,
        "risk_class": risk_class,
        "payload_hash": payload_hash,
        "tool_adapter": tool_adapter
    }))?;
    let state = if approval_required {
        "awaiting_approval"
    } else {
        "approved"
    };
    let now = now_utc();
    transaction.execute(
        "INSERT INTO agent_action_proposals (proposal_id,agent_id,agent_revision,assignment_id,assignment_revision,job_id,wrapper_id,connection_id,grant_id,grant_revision,connection_authority_revision,authorization_decision_id,policy_id,policy_revision,action_type,risk_class,title,rationale,safe_summary_json,payload_hash,plan_hash,state,approval_required,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?25)",
        params![
            proposal_id,agent_id,agent_revision,assignment_id,assignment.3,job_id,job.0,job.1,job.2,job.3,job.4,job.5,
            policy_id,policy_revision,action_type,risk_class,title,rationale,json_text(&request.safe_summary)?,payload_hash,plan_hash,state,
            if approval_required {1} else {0},timestamp(expires),now
        ],
    )?;
    transaction.execute(
        "INSERT INTO agent_action_private_payloads (proposal_id,private_payload_json,payload_bytes,created_at_utc) VALUES (?1,?2,?3,?4)",
        params![proposal_id, private_payload_text, request.private_payload.to_string().len() as i64, now],
    )?;
    if approval_required {
        let approval_id = Uuid::new_v4().to_string();
        transaction.execute(
            "INSERT INTO agent_action_approvals (approval_id,proposal_id,plan_hash,payload_hash,agent_revision,assignment_revision,policy_revision,grant_revision,connection_authority_revision,state,requested_by_user_id,requested_at_utc,expires_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10,?11,?12)",
            params![approval_id,proposal_id,plan_hash,payload_hash,agent_revision,assignment.3,policy_revision,job.3,job.4,actor,now,timestamp(expires)],
        )?;
    }
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&agent_id),
            wrapper_id: Some(&job.0),
            connection_id: Some(&job.1),
            assignment_id: Some(&assignment_id),
            proposal_id: Some(&proposal_id),
            event_type: "agent.action_proposed",
            outcome: "success",
            actor_type: "agent",
            actor_id: &agent_id,
            detail_code: if approval_required {
                "approval_required"
            } else {
                "scoped_autonomy"
            },
            metadata: json!({"plan_hash": plan_hash, "payload_hash": payload_hash, "private_payload_exposed": false}),
        },
    )?;
    transaction.commit()?;
    Ok(proposal_id)
}

fn approve_proposal(connection: &Connection, request: ProposalDecisionRequest) -> Result<()> {
    ensure!(
        request.confirmation == "APPROVE ACTION",
        "action approval confirmation is invalid"
    );
    decide_proposal(connection, request, true)
}

fn reject_proposal(connection: &Connection, request: ProposalDecisionRequest) -> Result<()> {
    ensure!(
        request.confirmation == "REJECT ACTION",
        "action rejection confirmation is invalid"
    );
    decide_proposal(connection, request, false)
}

fn decide_proposal(
    connection: &Connection,
    request: ProposalDecisionRequest,
    approve: bool,
) -> Result<()> {
    expire_and_reconcile(connection)?;
    let proposal_id = validate_uuid(&request.proposal_id, "proposal ID")?;
    let plan_hash = validate_sha256(&request.plan_hash, "plan hash")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(
        request
            .reason
            .as_deref()
            .unwrap_or(if approve { "approved" } else { "rejected" }),
        1,
        500,
        "decision reason",
    )?;
    let transaction = connection.unchecked_transaction()?;
    let authority = proposal_authority_tx(&transaction, &proposal_id)?;
    ensure!(
        authority.proposal.plan_hash == plan_hash,
        "proposal plan hash changed"
    );
    ensure!(
        authority.proposal.state == "awaiting_approval",
        "proposal is not awaiting approval"
    );
    ensure!(
        parse_utc(&authority.proposal.expires_at_utc, "proposal expiration")? > Utc::now(),
        "proposal expired"
    );
    ensure!(
        proposal_authority_is_current_tx(&transaction, &authority.proposal)?,
        "proposal authority changed"
    );
    let target = if approve { "approved" } else { "rejected" };
    let approval_target = if approve { "approved" } else { "rejected" };
    transaction.execute(
        "UPDATE agent_action_approvals SET state=?1,decided_by_user_id=?2,decision_reason=?3,decided_at_utc=?4 WHERE proposal_id=?5 AND state='pending'",
        params![approval_target, actor, reason, now_utc(), proposal_id],
    )?;
    transaction.execute(
        "UPDATE agent_action_proposals SET state=?1,failure_code=CASE WHEN ?1='rejected' THEN 'user_rejected' ELSE NULL END,completed_at_utc=CASE WHEN ?1='rejected' THEN ?2 ELSE NULL END,updated_at_utc=?2 WHERE proposal_id=?3",
        params![target, now_utc(), proposal_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&authority.proposal.agent_id),
            wrapper_id: Some(&authority.proposal.wrapper_id),
            connection_id: Some(&authority.proposal.connection_id),
            assignment_id: Some(&authority.proposal.assignment_id),
            proposal_id: Some(&proposal_id),
            event_type: if approve {
                "agent.action_approved"
            } else {
                "agent.action_rejected"
            },
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: &reason,
            metadata: json!({"plan_hash": plan_hash}),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn execute_proposal(
    connection: &Connection,
    request: ExecuteProposalRequest,
) -> Result<ActionReceiptSummary> {
    execute_proposal_with_actor_type(connection, request, "local_user")
}

pub(crate) fn execute_proposal_as_orchestrator(
    connection: &Connection,
    request: ExecuteProposalRequest,
) -> Result<ActionReceiptSummary> {
    ensure!(
        request.actor_user_id == "agent_orchestrator",
        "supervised orchestration actor identity is invalid"
    );
    execute_proposal_with_actor_type(connection, request, "system")
}

fn execute_proposal_with_actor_type(
    connection: &Connection,
    request: ExecuteProposalRequest,
    actor_type: &'static str,
) -> Result<ActionReceiptSummary> {
    let proposal_id = validate_uuid(&request.proposal_id, "proposal ID")?;
    let plan_hash = validate_sha256(&request.plan_hash, "plan hash")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let idempotency_key = bounded_text(&request.idempotency_key, 8, 160, "idempotency key")?;
    if let Some(existing) = read_receipt_by_proposal(connection, &proposal_id)? {
        let stored_key: String = connection.query_row(
            "SELECT idempotency_key FROM agent_action_attempts WHERE proposal_id=?1",
            params![proposal_id],
            |row| row.get(0),
        )?;
        ensure!(
            stored_key == idempotency_key,
            "proposal already executed with a different idempotency key"
        );
        return Ok(existing);
    }
    expire_and_reconcile(connection)?;
    let transaction = connection.unchecked_transaction()?;
    let authority = proposal_authority_tx(&transaction, &proposal_id)?;
    ensure!(
        authority.proposal.plan_hash == plan_hash,
        "proposal plan hash changed"
    );
    ensure!(
        authority.proposal.state == "approved",
        "proposal is not approved for execution"
    );
    ensure!(
        proposal_authority_is_current_tx(&transaction, &authority.proposal)?,
        "proposal authority changed"
    );
    ensure!(
        authority.autonomy_level >= 2,
        "suggest-only agent cannot execute actions"
    );
    let approval = if authority.proposal.approval_required {
        Some(read_approval_tx(&transaction, &proposal_id)?)
    } else {
        None
    };
    if let Some(approval) = &approval {
        ensure!(
            approval.state == "approved",
            "action approval is not active"
        );
        ensure!(
            approval.plan_hash == authority.proposal.plan_hash,
            "approval plan hash mismatch"
        );
        ensure!(
            approval.payload_hash == authority.proposal.payload_hash,
            "approval payload hash mismatch"
        );
        ensure!(
            approval.agent_revision == authority.proposal.agent_revision,
            "approval agent revision mismatch"
        );
        ensure!(
            approval.assignment_revision == authority.proposal.assignment_revision,
            "approval assignment revision mismatch"
        );
        ensure!(
            approval.policy_revision == authority.proposal.policy_revision,
            "approval policy revision mismatch"
        );
        ensure!(
            approval.grant_revision == authority.proposal.grant_revision,
            "approval grant revision mismatch"
        );
        ensure!(
            approval.connection_authority_revision
                == authority.proposal.connection_authority_revision,
            "approval connection revision mismatch"
        );
        ensure!(
            approval.consumed_at_utc.is_none(),
            "approval was already consumed"
        );
        ensure!(
            parse_utc(&approval.expires_at_utc, "approval expiration")? > Utc::now(),
            "approval expired"
        );
    }
    let payload_text: String = transaction.query_row(
        "SELECT private_payload_json FROM agent_action_private_payloads WHERE proposal_id=?1",
        params![proposal_id],
        |row| row.get(0),
    )?;
    ensure!(
        hash_text(&payload_text) == authority.proposal.payload_hash,
        "private action payload changed"
    );
    let private_payload: Value = serde_json::from_str(&payload_text)?;
    enforce_policy_window_tx(&transaction, &authority)?;
    let attempt_id = Uuid::new_v4().to_string();
    let started = now_utc();
    transaction.execute(
        "UPDATE agent_action_proposals SET state='executing',updated_at_utc=?1 WHERE proposal_id=?2 AND state='approved'",
        params![started, proposal_id],
    )?;
    transaction.execute(
        "INSERT INTO agent_action_attempts (attempt_id,proposal_id,approval_id,tool_adapter,idempotency_key,state,started_at_utc) VALUES (?1,?2,?3,?4,?5,'executing',?6)",
        params![attempt_id,proposal_id,approval.as_ref().map(|item| item.approval_id.clone()),authority.tool_adapter,idempotency_key,started],
    )?;
    let adapter_result = execute_adapter_tx(&transaction, &authority, &private_payload, &actor);
    let (outcome, result_code, safe_result, private_result) = match adapter_result {
        Ok((safe, private)) => ("completed", "action_completed", safe, private),
        Err(error) => (
            "failed",
            "adapter_rejected",
            json!({"completed": false, "detail_code": "adapter_rejected"}),
            json!({"error_class": "adapter_rejected", "message": error.to_string()}),
        ),
    };
    ensure_safe_value(&safe_result, 0)?;
    let safe_result_text = json_text(&safe_result)?;
    let safe_result_hash = hash_text(&safe_result_text);
    let private_result_text = json_text(&private_result)?;
    let private_result_hash = hash_text(&private_result_text);
    let completed = now_utc();
    transaction.execute(
        "UPDATE agent_action_attempts SET state=?1,result_code=?2,safe_result_json=?3,safe_result_hash=?4,completed_at_utc=?5 WHERE attempt_id=?6",
        params![outcome,result_code,safe_result_text,safe_result_hash,completed,attempt_id],
    )?;
    transaction.execute(
        "INSERT INTO agent_action_private_results (attempt_id,private_result_json,private_result_hash,created_at_utc) VALUES (?1,?2,?3,?4)",
        params![attempt_id,private_result_text,private_result_hash,completed],
    )?;
    if let Some(approval) = &approval {
        transaction.execute(
            "UPDATE agent_action_approvals SET state='consumed',consumed_at_utc=?1 WHERE approval_id=?2 AND state='approved'",
            params![completed, approval.approval_id],
        )?;
    }
    transaction.execute(
        "UPDATE agent_action_proposals SET state=?1,failure_code=CASE WHEN ?1='failed' THEN ?2 ELSE NULL END,completed_at_utc=?3,updated_at_utc=?3 WHERE proposal_id=?4",
        params![outcome,result_code,completed,proposal_id],
    )?;
    let receipt_id = Uuid::new_v4().to_string();
    let receipt_hash = hash_json(&json!({
        "schema": "homeserver.agent-action-receipt.v1",
        "receipt_id": receipt_id,
        "proposal_id": proposal_id,
        "attempt_id": attempt_id,
        "agent_id": authority.proposal.agent_id,
        "agent_revision": authority.proposal.agent_revision,
        "assignment_id": authority.proposal.assignment_id,
        "assignment_revision": authority.proposal.assignment_revision,
        "job_id": authority.proposal.job_id,
        "wrapper_id": authority.proposal.wrapper_id,
        "connection_id": authority.proposal.connection_id,
        "grant_id": authority.proposal.grant_id,
        "grant_revision": authority.proposal.grant_revision,
        "connection_authority_revision": authority.proposal.connection_authority_revision,
        "authorization_decision_id": authority.proposal.authorization_decision_id,
        "policy_id": authority.proposal.policy_id,
        "policy_revision": authority.proposal.policy_revision,
        "approval_id": approval.as_ref().map(|item| item.approval_id.clone()),
        "plan_hash": authority.proposal.plan_hash,
        "payload_hash": authority.proposal.payload_hash,
        "action_type": authority.proposal.action_type,
        "risk_class": authority.proposal.risk_class,
        "tool_adapter": authority.tool_adapter,
        "outcome": outcome,
        "result_code": result_code,
        "safe_result_hash": safe_result_hash,
        "completed_at_utc": completed
    }))?;
    transaction.execute(
        "INSERT INTO agent_action_receipts (receipt_id,proposal_id,attempt_id,agent_id,agent_revision,assignment_id,assignment_revision,job_id,wrapper_id,connection_id,grant_id,grant_revision,connection_authority_revision,authorization_decision_id,policy_id,policy_revision,approval_id,plan_hash,payload_hash,action_type,risk_class,tool_adapter,outcome,result_code,safe_result_hash,receipt_hash,completed_at_utc,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?27)",
        params![
            receipt_id,proposal_id,attempt_id,authority.proposal.agent_id,authority.proposal.agent_revision as i64,
            authority.proposal.assignment_id,authority.proposal.assignment_revision as i64,authority.proposal.job_id,
            authority.proposal.wrapper_id,authority.proposal.connection_id,authority.proposal.grant_id,
            authority.proposal.grant_revision as i64,authority.proposal.connection_authority_revision as i64,
            authority.proposal.authorization_decision_id,authority.proposal.policy_id,authority.proposal.policy_revision as i64,
            approval.as_ref().map(|item| item.approval_id.clone()),authority.proposal.plan_hash,authority.proposal.payload_hash,
            authority.proposal.action_type,authority.proposal.risk_class,authority.tool_adapter,outcome,result_code,safe_result_hash,
            receipt_hash,completed
        ],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: Some(&authority.proposal.agent_id),
            wrapper_id: Some(&authority.proposal.wrapper_id),
            connection_id: Some(&authority.proposal.connection_id),
            assignment_id: Some(&authority.proposal.assignment_id),
            proposal_id: Some(&proposal_id),
            event_type: "agent.action_executed",
            outcome: if outcome == "completed" {
                "success"
            } else {
                "error"
            },
            actor_type,
            actor_id: &actor,
            detail_code: result_code,
            metadata: json!({"receipt_hash": receipt_hash, "private_result_exposed": false}),
        },
    )?;
    transaction.commit()?;
    read_receipt_by_proposal(connection, &proposal_id)?.context("action receipt was not stored")
}

fn execute_adapter_tx(
    transaction: &Transaction<'_>,
    authority: &ProposalAuthority,
    private_payload: &Value,
    actor: &str,
) -> Result<(Value, Value)> {
    match authority.tool_adapter.as_str() {
        "audit.record" => Ok((
            json!({
                "recorded": true,
                "proposal_id": authority.proposal.proposal_id,
                "action_type": authority.proposal.action_type
            }),
            json!({"recorded_by": actor, "payload_hash": authority.proposal.payload_hash}),
        )),
        "report.save" => {
            ensure!(
                matches!(
                    authority.proposal.risk_class.as_str(),
                    "read_only" | "reversible"
                ),
                "report adapter is not valid for sensitive external actions"
            );
            let title = private_payload
                .get("title")
                .and_then(Value::as_str)
                .map(|value| bounded_text(value, 1, 180, "report title"))
                .transpose()?
                .context("report title is required")?;
            let content = private_payload
                .get("content_markdown")
                .and_then(Value::as_str)
                .map(|value| bounded_text(value, 1, 30_000, "report content"))
                .transpose()?
                .context("report content is required")?;
            let report_id = Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO agent_reports (report_id,plan_id,title,content_markdown,connection_ids_json,dataset_keys_json,created_at_utc) VALUES (?1,NULL,?2,?3,?4,'[]',?5)",
                params![report_id,title,content,serde_json::to_string(&vec![authority.proposal.connection_id.clone()])?,now_utc()],
            )?;
            Ok((
                json!({"report_id": report_id, "title": title, "saved": true}),
                json!({"content_hash": hash_text(&content), "payload_hash": authority.proposal.payload_hash}),
            ))
        }
        "proposal_only" => bail!("proposal-only policy cannot execute"),
        _ => bail!("tool adapter is not registered"),
    }
}

fn enforce_policy_window_tx(
    transaction: &Transaction<'_>,
    authority: &ProposalAuthority,
) -> Result<()> {
    let (max_executions, window_seconds): (i64, i64) = transaction.query_row(
        "SELECT max_executions,window_seconds FROM agent_execution_policies WHERE policy_id=?1",
        params![authority.proposal.policy_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let since = timestamp(Utc::now() - Duration::seconds(window_seconds.max(60)));
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_action_receipts WHERE policy_id=?1 AND outcome='completed' AND completed_at_utc>=?2",
        params![authority.proposal.policy_id, since],
        |row| row.get(0),
    )?;
    ensure!(
        count < max_executions,
        "agent execution-policy window limit exceeded"
    );
    Ok(())
}

fn activate_emergency_stop(
    connection: &Connection,
    request: ActivateEmergencyStopRequest,
) -> Result<String> {
    let scope_type = validate_enum(
        &request.scope_type,
        &["global", "agent", "wrapper", "connection"],
        "stop scope",
    )?;
    let agent_id = request
        .agent_id
        .as_deref()
        .map(|value| validate_uuid(value, "agent ID"))
        .transpose()?;
    let wrapper_id = request
        .wrapper_id
        .as_deref()
        .map(|value| validate_uuid(value, "wrapper ID"))
        .transpose()?;
    let connection_id = request
        .connection_id
        .as_deref()
        .map(|value| validate_uuid(value, "connection ID"))
        .transpose()?;
    validate_stop_scope(
        &scope_type,
        agent_id.as_deref(),
        wrapper_id.as_deref(),
        connection_id.as_deref(),
    )?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 1_000, "stop reason")?;
    if let Some(minutes) = request.expires_minutes {
        ensure!(
            (1..=10_080).contains(&minutes),
            "stop expiration must be between one minute and seven days"
        );
    }
    let expires = request
        .expires_minutes
        .map(|minutes| timestamp(Utc::now() + Duration::minutes(i64::from(minutes))));
    let stop_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let stop_hash = hash_json(&json!({
        "schema": "homeserver.agent-emergency-stop.v1",
        "stop_id": stop_id,
        "scope_type": scope_type,
        "agent_id": agent_id,
        "wrapper_id": wrapper_id,
        "connection_id": connection_id,
        "reason": reason,
        "activated_by_user_id": actor,
        "activated_at_utc": now,
        "expires_at_utc": expires
    }))?;
    let transaction = connection.unchecked_transaction()?;
    expire_stops_tx(&transaction)?;
    let active: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_emergency_stops WHERE state='active' AND scope_type=?1 AND COALESCE(agent_id,'')=COALESCE(?2,'') AND COALESCE(wrapper_id,'')=COALESCE(?3,'') AND COALESCE(connection_id,'')=COALESCE(?4,'')",
        params![scope_type,agent_id,wrapper_id,connection_id],
        |row| row.get(0),
    )?;
    ensure!(
        active == 0,
        "an emergency stop is already active for this scope"
    );
    transaction.execute(
        "INSERT INTO agent_emergency_stops (stop_id,scope_type,agent_id,wrapper_id,connection_id,state,reason,stop_hash,activated_by_user_id,activated_at_utc,expires_at_utc) VALUES (?1,?2,?3,?4,?5,'active',?6,?7,?8,?9,?10)",
        params![stop_id,scope_type,agent_id,wrapper_id,connection_id,reason,stop_hash,actor,now,expires],
    )?;
    if let Some(agent) = agent_id.as_deref() {
        cancel_agent_proposals_tx(&transaction, agent, "emergency_stop")?;
    } else if let Some(connection) = connection_id.as_deref() {
        cancel_connection_proposals_tx(&transaction, connection, "emergency_stop")?;
    } else if let Some(wrapper) = wrapper_id.as_deref() {
        transaction.execute(
            "UPDATE agent_action_proposals SET state='cancelled',failure_code='emergency_stop',completed_at_utc=?1,updated_at_utc=?1 WHERE wrapper_id=?2 AND state IN ('proposed','awaiting_approval','approved','executing')",
            params![now, wrapper],
        )?;
    } else {
        transaction.execute(
            "UPDATE agent_action_proposals SET state='cancelled',failure_code='emergency_stop',completed_at_utc=?1,updated_at_utc=?1 WHERE state IN ('proposed','awaiting_approval','approved','executing')",
            params![now],
        )?;
    }
    transaction.execute(
        "UPDATE agent_action_approvals SET state='cancelled' WHERE state IN ('pending','approved') AND proposal_id IN (SELECT proposal_id FROM agent_action_proposals WHERE failure_code='emergency_stop')",
        [],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: agent_id.as_deref(),
            wrapper_id: wrapper_id.as_deref(),
            connection_id: connection_id.as_deref(),
            assignment_id: None,
            proposal_id: None,
            event_type: "agent.emergency_stop_activated",
            outcome: "warning",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: "execution_blocked",
            metadata: json!({"scope_type": scope_type, "stop_hash": stop_hash}),
        },
    )?;
    transaction.commit()?;
    Ok(stop_id)
}

fn release_emergency_stop(
    connection: &Connection,
    request: ReleaseEmergencyStopRequest,
) -> Result<()> {
    ensure!(
        request.confirmation == "RELEASE EMERGENCY STOP",
        "emergency-stop release confirmation is invalid"
    );
    let stop_id = validate_uuid(&request.stop_id, "stop ID")?;
    let stop_hash = validate_sha256(&request.stop_hash, "stop hash")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 1_000, "release reason")?;
    let transaction = connection.unchecked_transaction()?;
    let stored: String = transaction.query_row(
        "SELECT stop_hash FROM agent_emergency_stops WHERE stop_id=?1 AND state='active'",
        params![stop_id],
        |row| row.get(0),
    )?;
    ensure!(stored == stop_hash, "emergency-stop hash changed");
    transaction.execute(
        "UPDATE agent_emergency_stops SET state='released',released_by_user_id=?1,released_at_utc=?2,release_reason=?3 WHERE stop_id=?4 AND state='active'",
        params![actor,now_utc(),reason,stop_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            agent_id: None,
            wrapper_id: None,
            connection_id: None,
            assignment_id: None,
            proposal_id: None,
            event_type: "agent.emergency_stop_released",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: "execution_may_resume",
            metadata: json!({"stop_id": stop_id, "stop_hash": stop_hash}),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn expire_and_reconcile(connection: &Connection) -> Result<()> {
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    expire_stops_tx(&transaction)?;
    transaction.execute(
        "UPDATE homeserver_agents SET state='expired',revision=revision+1,updated_at_utc=?1 WHERE state IN ('draft','active','suspended') AND expires_at_utc<=?1",
        params![now],
    )?;
    transaction.execute(
        "UPDATE wrapper_agent_assignments SET state='expired',assignment_revision=assignment_revision+1,updated_at_utc=?1 WHERE state IN ('active','suspended') AND expires_at_utc<=?1",
        params![now],
    )?;
    transaction.execute(
        "UPDATE agent_capability_bindings SET state='expired',updated_at_utc=?1 WHERE state IN ('active','suspended') AND expires_at_utc<=?1",
        params![now],
    )?;
    transaction.execute(
        "UPDATE agent_execution_policies SET state='expired',updated_at_utc=?1 WHERE state IN ('active','suspended') AND expires_at_utc<=?1",
        params![now],
    )?;
    transaction.execute(
        "UPDATE agent_action_approvals SET state='expired' WHERE state IN ('pending','approved') AND expires_at_utc<=?1",
        params![now],
    )?;
    transaction.execute(
        "UPDATE agent_action_proposals SET state='expired',failure_code='proposal_expired',completed_at_utc=?1,updated_at_utc=?1 WHERE state IN ('proposed','awaiting_approval','approved','executing') AND expires_at_utc<=?1",
        params![now],
    )?;
    transaction.execute(
        "UPDATE agent_action_proposals SET state='cancelled',failure_code='authority_changed',completed_at_utc=?1,updated_at_utc=?1 WHERE state IN ('proposed','awaiting_approval','approved','executing') AND NOT EXISTS (SELECT 1 FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.assignment_id=agent_action_proposals.assignment_id JOIN agent_execution_policies p ON p.policy_id=agent_action_proposals.policy_id JOIN wrapper_capability_grants g ON g.grant_id=agent_action_proposals.grant_id JOIN wrapper_connections c ON c.connection_id=agent_action_proposals.connection_id WHERE a.agent_id=agent_action_proposals.agent_id AND a.state='active' AND a.revision=agent_action_proposals.agent_revision AND x.state='active' AND x.assignment_revision=agent_action_proposals.assignment_revision AND p.state='active' AND p.policy_revision=agent_action_proposals.policy_revision AND g.state='active' AND g.grant_revision=agent_action_proposals.grant_revision AND c.lifecycle_state IN ('active','offline','grace') AND c.grant_revision=agent_action_proposals.connection_authority_revision)",
        params![now],
    )?;
    transaction.execute(
        "UPDATE agent_action_approvals SET state='cancelled' WHERE state IN ('pending','approved') AND proposal_id IN (SELECT proposal_id FROM agent_action_proposals WHERE state IN ('cancelled','expired','rejected'))",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

fn expire_stops_tx(transaction: &Transaction<'_>) -> Result<()> {
    transaction.execute(
        "UPDATE agent_emergency_stops SET state='expired' WHERE state='active' AND expires_at_utc IS NOT NULL AND expires_at_utc<=?1",
        params![now_utc()],
    )?;
    Ok(())
}

fn cancel_agent_proposals_tx(
    transaction: &Transaction<'_>,
    agent_id: &str,
    code: &str,
) -> Result<()> {
    transaction.execute(
        "UPDATE agent_action_proposals SET state='cancelled',failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE agent_id=?3 AND state IN ('proposed','awaiting_approval','approved','executing')",
        params![code,now_utc(),agent_id],
    )?;
    transaction.execute(
        "UPDATE agent_action_approvals SET state='cancelled' WHERE state IN ('pending','approved') AND proposal_id IN (SELECT proposal_id FROM agent_action_proposals WHERE agent_id=?1 AND state='cancelled')",
        params![agent_id],
    )?;
    Ok(())
}

fn cancel_connection_proposals_tx(
    transaction: &Transaction<'_>,
    connection_id: &str,
    code: &str,
) -> Result<()> {
    transaction.execute(
        "UPDATE agent_action_proposals SET state='cancelled',failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE connection_id=?3 AND state IN ('proposed','awaiting_approval','approved','executing')",
        params![code,now_utc(),connection_id],
    )?;
    Ok(())
}

fn proposal_authority_tx(
    transaction: &Transaction<'_>,
    proposal_id: &str,
) -> Result<ProposalAuthority> {
    let proposal = transaction.query_row(
        "SELECT proposal_id,agent_id,agent_revision,assignment_id,assignment_revision,job_id,wrapper_id,connection_id,grant_id,grant_revision,connection_authority_revision,authorization_decision_id,policy_id,policy_revision,action_type,risk_class,title,rationale,safe_summary_json,payload_hash,plan_hash,state,approval_required,expires_at_utc,failure_code,created_at_utc,updated_at_utc,completed_at_utc FROM agent_action_proposals WHERE proposal_id=?1",
        params![proposal_id],
        proposal_from_row,
    )?;
    let (tool_adapter, autonomy_level): (String, i64) = transaction.query_row(
        "SELECT p.tool_adapter,a.autonomy_level FROM agent_execution_policies p JOIN homeserver_agents a ON a.agent_id=p.agent_id WHERE p.policy_id=?1",
        params![proposal.policy_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ProposalAuthority {
        proposal,
        tool_adapter,
        autonomy_level: autonomy_level.max(0) as u8,
    })
}

fn proposal_authority_is_current_tx(
    transaction: &Transaction<'_>,
    proposal: &ProposalSummary,
) -> Result<bool> {
    let context: Option<ProposalAuthorityRow> = transaction.query_row(
        "SELECT a.state,a.revision,x.state,x.assignment_revision,p.state,p.policy_revision,g.state,g.grant_revision,c.lifecycle_state,c.grant_revision,w.state,j.state,j.authorization_decision_id,r.outcome FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.assignment_id=?2 AND x.agent_id=a.agent_id JOIN agent_execution_policies p ON p.policy_id=?3 AND p.agent_id=a.agent_id JOIN wrapper_capability_grants g ON g.grant_id=?4 JOIN wrapper_connections c ON c.connection_id=?5 AND c.wrapper_id=?6 JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id JOIN wrapper_jobs j ON j.job_id=?7 JOIN wrapper_authorization_receipts r ON r.decision_id=j.authorization_decision_id WHERE a.agent_id=?1",
        params![proposal.agent_id,proposal.assignment_id,proposal.policy_id,proposal.grant_id,proposal.connection_id,proposal.wrapper_id,proposal.job_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?,row.get(12)?,row.get(13)?)),
    ).optional()?;
    let Some(context) = context else {
        return Ok(false);
    };
    Ok(context.0 == "active"
        && context.1.max(0) as u64 == proposal.agent_revision
        && context.2 == "active"
        && context.3.max(0) as u64 == proposal.assignment_revision
        && context.4 == "active"
        && context.5.max(0) as u64 == proposal.policy_revision
        && context.6 == "active"
        && context.7.max(0) as u64 == proposal.grant_revision
        && matches!(context.8.as_str(), "active" | "offline" | "grace")
        && context.9.max(0) as u64 == proposal.connection_authority_revision
        && context.10 == "active"
        && context.11 == "completed"
        && context.12 == proposal.authorization_decision_id
        && context.13 == "allowed"
        && !emergency_stop_active_tx(transaction, &proposal.agent_id, &proposal.connection_id)?)
}

fn emergency_stop_active(
    connection: &Connection,
    agent_id: &str,
    connection_id: &str,
) -> Result<bool> {
    let transaction = connection.unchecked_transaction()?;
    expire_stops_tx(&transaction)?;
    let active = emergency_stop_active_tx(&transaction, agent_id, connection_id)?;
    transaction.commit()?;
    Ok(active)
}

fn emergency_stop_active_tx(
    transaction: &Transaction<'_>,
    agent_id: &str,
    connection_id: &str,
) -> Result<bool> {
    let wrapper_id: Option<String> = transaction
        .query_row(
            "SELECT wrapper_id FROM wrapper_connections WHERE connection_id=?1",
            params![connection_id],
            |row| row.get(0),
        )
        .optional()?;
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_emergency_stops WHERE state='active' AND (expires_at_utc IS NULL OR expires_at_utc>?4) AND (scope_type='global' OR (scope_type='agent' AND agent_id=?1) OR (scope_type='connection' AND connection_id=?2) OR (scope_type='wrapper' AND wrapper_id=?3))",
        params![agent_id,connection_id,wrapper_id,now_utc()],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn validate_stop_scope(
    scope: &str,
    agent_id: Option<&str>,
    wrapper_id: Option<&str>,
    connection_id: Option<&str>,
) -> Result<()> {
    let valid = match scope {
        "global" => agent_id.is_none() && wrapper_id.is_none() && connection_id.is_none(),
        "agent" => agent_id.is_some() && wrapper_id.is_none() && connection_id.is_none(),
        "wrapper" => agent_id.is_none() && wrapper_id.is_some() && connection_id.is_none(),
        "connection" => agent_id.is_none() && wrapper_id.is_none() && connection_id.is_some(),
        _ => false,
    };
    ensure!(valid, "emergency-stop scope identifiers are invalid");
    Ok(())
}

fn snapshot(state: &AppState, connection_id: Option<&str>) -> Result<AgentRegistrySnapshot> {
    let connection = state.connection()?;
    snapshot_with_connection(&connection, connection_id)
}

fn snapshot_with_connection(
    connection: &Connection,
    connection_id: Option<&str>,
) -> Result<AgentRegistrySnapshot> {
    expire_and_reconcile(connection)?;
    let agents = read_agents(connection, connection_id)?;
    let assignments = read_assignments(connection, connection_id)?;
    let policies = read_policies(connection, connection_id)?;
    let proposals = read_proposals(connection, connection_id)?;
    let approvals = read_approvals(connection, connection_id)?;
    let receipts = read_receipts(connection, connection_id)?;
    let emergency_stops = read_stops(connection, connection_id)?;
    Ok(AgentRegistrySnapshot {
        schema: "homeserver.wrapper-agents.v1".to_owned(),
        connection_id: connection_id.map(str::to_owned),
        agents,
        assignments,
        policies,
        proposals,
        approvals,
        receipts,
        emergency_stops,
        private_payloads_exposed: false,
        private_results_exposed: false,
        pairing_implies_agent_authority: false,
    })
}

fn read_agents(connection: &Connection, connection_id: Option<&str>) -> Result<Vec<AgentSummary>> {
    let sql = if connection_id.is_some() {
        "SELECT DISTINCT a.agent_id,a.owner_user_id,a.display_name,a.purpose,a.description,a.state,a.autonomy_level,a.revision,a.allowed_job_types_json,a.model_restrictions_json,a.tool_restrictions_json,a.expires_at_utc,a.activated_at_utc,a.suspended_at_utc,a.revoked_at_utc,a.created_at_utc,a.updated_at_utc FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.agent_id=a.agent_id WHERE x.connection_id=?1 ORDER BY a.updated_at_utc DESC,a.agent_id"
    } else {
        "SELECT agent_id,owner_user_id,display_name,purpose,description,state,autonomy_level,revision,allowed_job_types_json,model_restrictions_json,tool_restrictions_json,expires_at_utc,activated_at_utc,suspended_at_utc,revoked_at_utc,created_at_utc,updated_at_utc FROM homeserver_agents ORDER BY updated_at_utc DESC,agent_id"
    };
    let mut statement = connection.prepare(sql)?;
    let rows = if let Some(value) = connection_id {
        statement.query_map(params![value], agent_from_row)?
    } else {
        statement.query_map([], agent_from_row)?
    };
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_assignments(
    connection: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<AssignmentSummary>> {
    let sql = if connection_id.is_some() {
        "SELECT assignment_id,agent_id,wrapper_id,connection_id,assignment_revision,state,assigned_by_user_id,purpose,allowed_job_types_json,expires_at_utc,revoked_at_utc,created_at_utc,updated_at_utc FROM wrapper_agent_assignments WHERE connection_id=?1 ORDER BY updated_at_utc DESC,assignment_id"
    } else {
        "SELECT assignment_id,agent_id,wrapper_id,connection_id,assignment_revision,state,assigned_by_user_id,purpose,allowed_job_types_json,expires_at_utc,revoked_at_utc,created_at_utc,updated_at_utc FROM wrapper_agent_assignments ORDER BY updated_at_utc DESC,assignment_id"
    };
    let mut statement = connection.prepare(sql)?;
    let rows = if let Some(value) = connection_id {
        statement.query_map(params![value], assignment_from_row)?
    } else {
        statement.query_map([], assignment_from_row)?
    };
    let mut results = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for assignment in &mut results {
        let mut grants = connection.prepare(
            "SELECT grant_id FROM agent_capability_bindings WHERE assignment_id=?1 ORDER BY grant_id",
        )?;
        assignment.grant_ids = grants
            .query_map(params![assignment.assignment_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
    }
    Ok(results)
}

fn read_policies(
    connection: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<PolicySummary>> {
    let sql = if connection_id.is_some() {
        "SELECT DISTINCT p.policy_id,p.agent_id,p.policy_revision,p.action_type,p.risk_class,p.approval_mode,p.tool_adapter,p.max_executions,p.window_seconds,p.state,p.not_before_utc,p.expires_at_utc,p.created_by_user_id,p.created_at_utc,p.updated_at_utc,p.revoked_at_utc FROM agent_execution_policies p JOIN wrapper_agent_assignments x ON x.agent_id=p.agent_id WHERE x.connection_id=?1 ORDER BY p.updated_at_utc DESC,p.policy_id"
    } else {
        "SELECT policy_id,agent_id,policy_revision,action_type,risk_class,approval_mode,tool_adapter,max_executions,window_seconds,state,not_before_utc,expires_at_utc,created_by_user_id,created_at_utc,updated_at_utc,revoked_at_utc FROM agent_execution_policies ORDER BY updated_at_utc DESC,policy_id"
    };
    let mut statement = connection.prepare(sql)?;
    let rows = if let Some(value) = connection_id {
        statement.query_map(params![value], policy_from_row)?
    } else {
        statement.query_map([], policy_from_row)?
    };
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_proposals(
    connection: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<ProposalSummary>> {
    let base = "SELECT proposal_id,agent_id,agent_revision,assignment_id,assignment_revision,job_id,wrapper_id,connection_id,grant_id,grant_revision,connection_authority_revision,authorization_decision_id,policy_id,policy_revision,action_type,risk_class,title,rationale,safe_summary_json,payload_hash,plan_hash,state,approval_required,expires_at_utc,failure_code,created_at_utc,updated_at_utc,completed_at_utc FROM agent_action_proposals";
    let sql = if connection_id.is_some() {
        format!("{base} WHERE connection_id=?1 ORDER BY created_at_utc DESC,proposal_id")
    } else {
        format!("{base} ORDER BY created_at_utc DESC,proposal_id")
    };
    let mut statement = connection.prepare(&sql)?;
    let rows = if let Some(value) = connection_id {
        statement.query_map(params![value], proposal_from_row)?
    } else {
        statement.query_map([], proposal_from_row)?
    };
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_approvals(
    connection: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<ActionApprovalSummary>> {
    let base = "SELECT a.approval_id,a.proposal_id,a.plan_hash,a.payload_hash,a.agent_revision,a.assignment_revision,a.policy_revision,a.grant_revision,a.connection_authority_revision,a.state,a.requested_by_user_id,a.decided_by_user_id,a.decision_reason,a.requested_at_utc,a.decided_at_utc,a.consumed_at_utc,a.expires_at_utc FROM agent_action_approvals a";
    let sql = if connection_id.is_some() {
        format!("{base} JOIN agent_action_proposals p ON p.proposal_id=a.proposal_id WHERE p.connection_id=?1 ORDER BY a.requested_at_utc DESC,a.approval_id")
    } else {
        format!("{base} ORDER BY a.requested_at_utc DESC,a.approval_id")
    };
    let mut statement = connection.prepare(&sql)?;
    let rows = if let Some(value) = connection_id {
        statement.query_map(params![value], approval_from_row)?
    } else {
        statement.query_map([], approval_from_row)?
    };
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_receipts(
    connection: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<ActionReceiptSummary>> {
    let base = "SELECT receipt_id,proposal_id,attempt_id,agent_id,agent_revision,assignment_id,assignment_revision,job_id,wrapper_id,connection_id,grant_id,grant_revision,connection_authority_revision,authorization_decision_id,policy_id,policy_revision,approval_id,plan_hash,payload_hash,action_type,risk_class,tool_adapter,outcome,result_code,safe_result_hash,receipt_hash,completed_at_utc FROM agent_action_receipts";
    let sql = if connection_id.is_some() {
        format!("{base} WHERE connection_id=?1 ORDER BY completed_at_utc DESC,receipt_id")
    } else {
        format!("{base} ORDER BY completed_at_utc DESC,receipt_id")
    };
    let mut statement = connection.prepare(&sql)?;
    let rows = if let Some(value) = connection_id {
        statement.query_map(params![value], receipt_from_row)?
    } else {
        statement.query_map([], receipt_from_row)?
    };
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_stops(
    connection: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<EmergencyStopSummary>> {
    let sql = if connection_id.is_some() {
        "SELECT stop_id,scope_type,agent_id,wrapper_id,connection_id,state,reason,stop_hash,activated_by_user_id,activated_at_utc,expires_at_utc,released_by_user_id,released_at_utc,release_reason FROM agent_emergency_stops WHERE scope_type='global' OR connection_id=?1 OR wrapper_id=(SELECT wrapper_id FROM wrapper_connections WHERE connection_id=?1) OR agent_id IN (SELECT agent_id FROM wrapper_agent_assignments WHERE connection_id=?1) ORDER BY activated_at_utc DESC,stop_id"
    } else {
        "SELECT stop_id,scope_type,agent_id,wrapper_id,connection_id,state,reason,stop_hash,activated_by_user_id,activated_at_utc,expires_at_utc,released_by_user_id,released_at_utc,release_reason FROM agent_emergency_stops ORDER BY activated_at_utc DESC,stop_id"
    };
    let mut statement = connection.prepare(sql)?;
    let rows = if let Some(value) = connection_id {
        statement.query_map(params![value], stop_from_row)?
    } else {
        statement.query_map([], stop_from_row)?
    };
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_approval_tx(
    transaction: &Transaction<'_>,
    proposal_id: &str,
) -> Result<ActionApprovalSummary> {
    transaction.query_row(
        "SELECT approval_id,proposal_id,plan_hash,payload_hash,agent_revision,assignment_revision,policy_revision,grant_revision,connection_authority_revision,state,requested_by_user_id,decided_by_user_id,decision_reason,requested_at_utc,decided_at_utc,consumed_at_utc,expires_at_utc FROM agent_action_approvals WHERE proposal_id=?1",
        params![proposal_id],
        approval_from_row,
    ).map_err(Into::into)
}

fn read_receipt_by_proposal(
    connection: &Connection,
    proposal_id: &str,
) -> Result<Option<ActionReceiptSummary>> {
    connection.query_row(
        "SELECT receipt_id,proposal_id,attempt_id,agent_id,agent_revision,assignment_id,assignment_revision,job_id,wrapper_id,connection_id,grant_id,grant_revision,connection_authority_revision,authorization_decision_id,policy_id,policy_revision,approval_id,plan_hash,payload_hash,action_type,risk_class,tool_adapter,outcome,result_code,safe_result_hash,receipt_hash,completed_at_utc FROM agent_action_receipts WHERE proposal_id=?1",
        params![proposal_id],
        receipt_from_row,
    ).optional().map_err(Into::into)
}

fn agent_from_row(row: &Row<'_>) -> rusqlite::Result<AgentSummary> {
    let jobs: String = row.get(8)?;
    let models: String = row.get(9)?;
    let tools: String = row.get(10)?;
    Ok(AgentSummary {
        agent_id: row.get(0)?,
        owner_user_id: row.get(1)?,
        display_name: row.get(2)?,
        purpose: row.get(3)?,
        description: row.get(4)?,
        state: row.get(5)?,
        autonomy_level: nonnegative_u8(row.get(6)?),
        revision: nonnegative_u64(row.get(7)?),
        allowed_job_types: parse_string_list(&jobs),
        model_restrictions: serde_json::from_str(&models).unwrap_or_else(|_| json!({})),
        tool_restrictions: serde_json::from_str(&tools).unwrap_or_else(|_| json!({})),
        expires_at_utc: row.get(11)?,
        activated_at_utc: row.get(12)?,
        suspended_at_utc: row.get(13)?,
        revoked_at_utc: row.get(14)?,
        created_at_utc: row.get(15)?,
        updated_at_utc: row.get(16)?,
    })
}

fn assignment_from_row(row: &Row<'_>) -> rusqlite::Result<AssignmentSummary> {
    let jobs: String = row.get(8)?;
    Ok(AssignmentSummary {
        assignment_id: row.get(0)?,
        agent_id: row.get(1)?,
        wrapper_id: row.get(2)?,
        connection_id: row.get(3)?,
        assignment_revision: nonnegative_u64(row.get(4)?),
        state: row.get(5)?,
        assigned_by_user_id: row.get(6)?,
        purpose: row.get(7)?,
        allowed_job_types: parse_string_list(&jobs),
        grant_ids: Vec::new(),
        expires_at_utc: row.get(9)?,
        revoked_at_utc: row.get(10)?,
        created_at_utc: row.get(11)?,
        updated_at_utc: row.get(12)?,
    })
}

fn policy_from_row(row: &Row<'_>) -> rusqlite::Result<PolicySummary> {
    Ok(PolicySummary {
        policy_id: row.get(0)?,
        agent_id: row.get(1)?,
        policy_revision: nonnegative_u64(row.get(2)?),
        action_type: row.get(3)?,
        risk_class: row.get(4)?,
        approval_mode: row.get(5)?,
        tool_adapter: row.get(6)?,
        max_executions: nonnegative_u32(row.get(7)?),
        window_seconds: nonnegative_u32(row.get(8)?),
        state: row.get(9)?,
        not_before_utc: row.get(10)?,
        expires_at_utc: row.get(11)?,
        created_by_user_id: row.get(12)?,
        created_at_utc: row.get(13)?,
        updated_at_utc: row.get(14)?,
        revoked_at_utc: row.get(15)?,
    })
}

fn proposal_from_row(row: &Row<'_>) -> rusqlite::Result<ProposalSummary> {
    let safe: String = row.get(18)?;
    Ok(ProposalSummary {
        proposal_id: row.get(0)?,
        agent_id: row.get(1)?,
        agent_revision: nonnegative_u64(row.get(2)?),
        assignment_id: row.get(3)?,
        assignment_revision: nonnegative_u64(row.get(4)?),
        job_id: row.get(5)?,
        wrapper_id: row.get(6)?,
        connection_id: row.get(7)?,
        grant_id: row.get(8)?,
        grant_revision: nonnegative_u64(row.get(9)?),
        connection_authority_revision: nonnegative_u64(row.get(10)?),
        authorization_decision_id: row.get(11)?,
        policy_id: row.get(12)?,
        policy_revision: nonnegative_u64(row.get(13)?),
        action_type: row.get(14)?,
        risk_class: row.get(15)?,
        title: row.get(16)?,
        rationale: row.get(17)?,
        safe_summary: serde_json::from_str(&safe).unwrap_or_else(|_| json!({})),
        payload_hash: row.get(19)?,
        plan_hash: row.get(20)?,
        state: row.get(21)?,
        approval_required: row.get::<_, i64>(22)? == 1,
        expires_at_utc: row.get(23)?,
        failure_code: row.get(24)?,
        created_at_utc: row.get(25)?,
        updated_at_utc: row.get(26)?,
        completed_at_utc: row.get(27)?,
    })
}

fn approval_from_row(row: &Row<'_>) -> rusqlite::Result<ActionApprovalSummary> {
    Ok(ActionApprovalSummary {
        approval_id: row.get(0)?,
        proposal_id: row.get(1)?,
        plan_hash: row.get(2)?,
        payload_hash: row.get(3)?,
        agent_revision: nonnegative_u64(row.get(4)?),
        assignment_revision: nonnegative_u64(row.get(5)?),
        policy_revision: nonnegative_u64(row.get(6)?),
        grant_revision: nonnegative_u64(row.get(7)?),
        connection_authority_revision: nonnegative_u64(row.get(8)?),
        state: row.get(9)?,
        requested_by_user_id: row.get(10)?,
        decided_by_user_id: row.get(11)?,
        decision_reason: row.get(12)?,
        requested_at_utc: row.get(13)?,
        decided_at_utc: row.get(14)?,
        consumed_at_utc: row.get(15)?,
        expires_at_utc: row.get(16)?,
    })
}

fn receipt_from_row(row: &Row<'_>) -> rusqlite::Result<ActionReceiptSummary> {
    Ok(ActionReceiptSummary {
        receipt_id: row.get(0)?,
        proposal_id: row.get(1)?,
        attempt_id: row.get(2)?,
        agent_id: row.get(3)?,
        agent_revision: nonnegative_u64(row.get(4)?),
        assignment_id: row.get(5)?,
        assignment_revision: nonnegative_u64(row.get(6)?),
        job_id: row.get(7)?,
        wrapper_id: row.get(8)?,
        connection_id: row.get(9)?,
        grant_id: row.get(10)?,
        grant_revision: nonnegative_u64(row.get(11)?),
        connection_authority_revision: nonnegative_u64(row.get(12)?),
        authorization_decision_id: row.get(13)?,
        policy_id: row.get(14)?,
        policy_revision: nonnegative_u64(row.get(15)?),
        approval_id: row.get(16)?,
        plan_hash: row.get(17)?,
        payload_hash: row.get(18)?,
        action_type: row.get(19)?,
        risk_class: row.get(20)?,
        tool_adapter: row.get(21)?,
        outcome: row.get(22)?,
        result_code: row.get(23)?,
        safe_result_hash: row.get(24)?,
        receipt_hash: row.get(25)?,
        completed_at_utc: row.get(26)?,
    })
}

fn stop_from_row(row: &Row<'_>) -> rusqlite::Result<EmergencyStopSummary> {
    Ok(EmergencyStopSummary {
        stop_id: row.get(0)?,
        scope_type: row.get(1)?,
        agent_id: row.get(2)?,
        wrapper_id: row.get(3)?,
        connection_id: row.get(4)?,
        state: row.get(5)?,
        reason: row.get(6)?,
        stop_hash: row.get(7)?,
        activated_by_user_id: row.get(8)?,
        activated_at_utc: row.get(9)?,
        expires_at_utc: row.get(10)?,
        released_by_user_id: row.get(11)?,
        released_at_utc: row.get(12)?,
        release_reason: row.get(13)?,
    })
}

struct EventEvidence<'a> {
    agent_id: Option<&'a str>,
    wrapper_id: Option<&'a str>,
    connection_id: Option<&'a str>,
    assignment_id: Option<&'a str>,
    proposal_id: Option<&'a str>,
    event_type: &'a str,
    outcome: &'a str,
    actor_type: &'a str,
    actor_id: &'a str,
    detail_code: &'a str,
    metadata: Value,
}

fn record_event_tx(transaction: &Transaction<'_>, evidence: EventEvidence<'_>) -> Result<()> {
    let event_id = Uuid::new_v4().to_string();
    let created = now_utc();
    let event_hash = hash_json(&json!({
        "event_id": event_id,
        "agent_id": evidence.agent_id,
        "wrapper_id": evidence.wrapper_id,
        "connection_id": evidence.connection_id,
        "assignment_id": evidence.assignment_id,
        "proposal_id": evidence.proposal_id,
        "event_type": evidence.event_type,
        "outcome": evidence.outcome,
        "actor_type": evidence.actor_type,
        "actor_id": evidence.actor_id,
        "detail_code": evidence.detail_code,
        "metadata": evidence.metadata.clone(),
        "created_at_utc": created
    }))?;
    transaction.execute(
        "INSERT INTO agent_lifecycle_events (event_id,agent_id,wrapper_id,connection_id,assignment_id,proposal_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            event_id,evidence.agent_id,evidence.wrapper_id,evidence.connection_id,evidence.assignment_id,evidence.proposal_id,
            evidence.event_type,evidence.outcome,evidence.actor_type,evidence.actor_id,evidence.detail_code,
            json_text(&evidence.metadata)?,event_hash,created
        ],
    )?;
    Ok(())
}

fn validate_restrictions(value: &Value, label: &str) -> Result<()> {
    ensure!(value.is_object(), "{label} must be a JSON object");
    let text = json_text(value)?;
    ensure!(text.len() <= 16 * 1024, "{label} exceeds the size limit");
    Ok(())
}

pub(crate) fn validate_safe_summary(value: &Value) -> Result<()> {
    ensure_safe_value(value, 0)
}

fn ensure_safe_value(value: &Value, depth: usize) -> Result<()> {
    ensure!(depth <= 8, "safe result exceeds maximum depth");
    match value {
        Value::Object(map) => {
            ensure!(map.len() <= 128, "safe result contains too many fields");
            for (key, child) in map {
                let normalized = key.to_ascii_lowercase();
                ensure!(
                    !FORBIDDEN_SAFE_KEYS
                        .iter()
                        .any(|item| normalized.contains(item)),
                    "safe result contains forbidden private field {key}"
                );
                ensure_safe_value(child, depth + 1)?;
            }
        }
        Value::Array(items) => {
            ensure!(items.len() <= 256, "safe result contains too many items");
            for item in items {
                ensure_safe_value(item, depth + 1)?;
            }
        }
        Value::String(text) => ensure!(text.len() <= 8_000, "safe result string exceeds the limit"),
        _ => {}
    }
    Ok(())
}

fn validate_symbol_list(
    values: Vec<String>,
    maximum: usize,
    max_length: usize,
    label: &str,
) -> Result<Vec<String>> {
    ensure!(
        !values.is_empty() && values.len() <= maximum,
        "{label} list has invalid length"
    );
    let mut unique = BTreeSet::new();
    for value in values {
        unique.insert(validate_symbol(&value, max_length, label)?);
    }
    Ok(unique.into_iter().collect())
}

fn validate_symbol(value: &str, max_length: usize, label: &str) -> Result<String> {
    let value = bounded_text(value, 1, max_length, label)?;
    ensure!(
        value
            .chars()
            .all(|character| character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | '-' | ':' | '/')),
        "{label} contains invalid characters"
    );
    Ok(value)
}

fn validate_enum(value: &str, allowed: &[&str], label: &str) -> Result<String> {
    ensure!(allowed.contains(&value), "invalid {label}");
    Ok(value.to_owned())
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    let parsed = Uuid::parse_str(value).with_context(|| format!("{label} is invalid"))?;
    Ok(parsed.to_string())
}

fn validate_sha256(value: &str, label: &str) -> Result<String> {
    ensure!(
        value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()),
        "{label} is invalid"
    );
    Ok(value.to_ascii_lowercase())
}

fn bounded_text(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let trimmed = value.trim();
    ensure!(
        (minimum..=maximum).contains(&trimmed.len()),
        "{label} has invalid length"
    );
    Ok(trimmed.to_owned())
}

fn parse_utc(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{label} is invalid"))
        .map(|value| value.with_timezone(&Utc))
}

fn now_utc() -> String {
    timestamp(Utc::now())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn empty_object() -> Value {
    json!({})
}

fn json_text(value: &Value) -> Result<String> {
    serde_json::to_string(&canonical_json(value)).map_err(Into::into)
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut ordered = Map::new();
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(child) = map.get(key) {
                    ordered.insert(key.clone(), canonical_json(child));
                }
            }
            Value::Object(ordered)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonical_json).collect()),
        _ => value.clone(),
    }
}

fn hash_json(value: &Value) -> Result<String> {
    Ok(hash_text(&json_text(value)?))
}

fn hash_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn parse_string_list(value: &str) -> Vec<String> {
    serde_json::from_str(value).unwrap_or_default()
}

fn nonnegative_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn nonnegative_u32(value: i64) -> u32 {
    value.max(0).min(i64::from(u32::MAX)) as u32
}

fn nonnegative_u8(value: i64) -> u8 {
    value.max(0).min(i64::from(u8::MAX)) as u8
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
