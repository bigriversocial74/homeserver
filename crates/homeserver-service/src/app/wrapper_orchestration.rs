use super::{wrapper_agents, wrapper_jobs, wrapper_runtime};
use crate::AppState;
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration as StdDuration};
use tokio::sync::watch;
use tracing::{error, warn};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0026_supervised_action_orchestration.sql");
const MIGRATION_KEY: &str = "0026_supervised_action_orchestration";
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;
const MAX_CHECKPOINTS: i64 = 500;
const MAX_RECEIPTS: i64 = 50_000;
const MAX_EVENTS: i64 = 50_000;
const WORKER_NAME: &str = "HomeServer Supervised Action Orchestrator";
const SUPERVISED_TOOL_KEY: &str = "action.supervised";
const SUPERVISED_JOB_TYPE: &str = "action.propose";
const ORCHESTRATOR_ACTOR: &str = "agent_orchestrator";

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupervisedCheckpointSummary {
    pub checkpoint_id: String,
    pub plan_id: String,
    pub step_id: String,
    pub sequence_number: u32,
    pub job_id: String,
    pub agent_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub proposal_id: String,
    pub approval_id: Option<String>,
    pub policy_id: String,
    pub action_type: String,
    pub risk_class: String,
    pub tool_adapter: String,
    pub title: String,
    pub rationale: String,
    pub safe_summary: Value,
    pub state: String,
    pub approval_state: Option<String>,
    pub proposal_state: String,
    pub compensation_mode: String,
    pub compensation_supported: bool,
    pub compensation_state: String,
    pub runtime_plan_hash: String,
    pub proposal_plan_hash: String,
    pub payload_hash: String,
    pub failure_code: Option<String>,
    pub expires_at_utc: String,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupervisedReceiptSummary {
    pub receipt_id: String,
    pub checkpoint_id: String,
    pub plan_id: String,
    pub step_id: String,
    pub job_id: String,
    pub proposal_id: String,
    pub approval_id: Option<String>,
    pub action_receipt_id: Option<String>,
    pub action_receipt_hash: Option<String>,
    pub wrapper_job_receipt_id: String,
    pub wrapper_job_receipt_hash: String,
    pub runtime_receipt_id: String,
    pub runtime_receipt_hash: String,
    pub runtime_plan_hash: String,
    pub proposal_plan_hash: String,
    pub payload_hash: String,
    pub outcome: String,
    pub result_code: String,
    pub safe_result_hash: Option<String>,
    pub phase16e_detail_code: String,
    pub receipt_hash: String,
    pub completed_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompensationReceiptSummary {
    pub compensation_receipt_id: String,
    pub checkpoint_id: String,
    pub action_receipt_id: String,
    pub adapter_key: String,
    pub outcome: String,
    pub result_code: String,
    pub target_hash: String,
    pub receipt_hash: String,
    pub completed_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SupervisedOrchestrationSnapshot {
    pub schema: String,
    pub state: String,
    pub worker_id: String,
    pub checkpoints: Vec<SupervisedCheckpointSummary>,
    pub receipts: Vec<SupervisedReceiptSummary>,
    pub compensation_receipts: Vec<CompensationReceiptSummary>,
    pub private_payloads_exposed: bool,
    pub private_results_exposed: bool,
    pub approval_hashes_revalidated: bool,
    pub approval_consumed_once: bool,
    pub phase16e_egress_required: bool,
    pub sensitive_runtime_bypass_allowed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateSupervisedPlanRequest {
    pub plan: wrapper_runtime::CreateRuntimePlanRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RollbackCheckpointRequest {
    pub checkpoint_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct ProposalJobContext {
    plan_id: String,
    step_id: String,
    sequence_number: u32,
    runtime_plan_hash: String,
    agent_id: String,
    assignment_id: String,
    wrapper_id: String,
    connection_id: String,
    grant_id: String,
    grant_revision: u64,
    connection_authority_revision: u64,
    authorization_decision_id: String,
    job_id: String,
}

#[derive(Debug, Clone)]
struct ProposalInput {
    policy_id: String,
    title: String,
    rationale: String,
    safe_summary: Value,
    private_payload: Value,
    requested_by_user_id: String,
    expires_minutes: u32,
    compensation_mode: String,
}

#[derive(Debug, Clone)]
struct CheckpointRecord {
    checkpoint_id: String,
    plan_id: String,
    step_id: String,
    job_id: String,
    agent_id: String,
    wrapper_id: String,
    connection_id: String,
    proposal_id: String,
    approval_id: Option<String>,
    policy_id: String,
    action_type: String,
    risk_class: String,
    tool_adapter: String,
    runtime_plan_hash: String,
    proposal_plan_hash: String,
    payload_hash: String,
    agent_revision: u64,
    assignment_id: String,
    assignment_revision: u64,
    policy_revision: u64,
    grant_id: String,
    grant_revision: u64,
    connection_authority_revision: u64,
    expires_at_utc: String,
}

#[derive(Debug)]
struct ProposalCheckpointRow {
    proposal_plan_hash: String,
    payload_hash: String,
    agent_revision: i64,
    assignment_id: String,
    assignment_revision: i64,
    policy_id: String,
    policy_revision: i64,
    grant_revision: i64,
    connection_authority_revision: i64,
    action_type: String,
    risk_class: String,
    tool_adapter: String,
    proposal_state: String,
    expires_at_utc: String,
    job_id: String,
    authorization_decision_id: String,
    approval_id: Option<String>,
}

fn proposal_checkpoint_from_row(row: &Row<'_>) -> rusqlite::Result<ProposalCheckpointRow> {
    Ok(ProposalCheckpointRow {
        proposal_plan_hash: row.get(0)?,
        payload_hash: row.get(1)?,
        agent_revision: row.get(2)?,
        assignment_id: row.get(3)?,
        assignment_revision: row.get(4)?,
        policy_id: row.get(5)?,
        policy_revision: row.get(6)?,
        grant_revision: row.get(7)?,
        connection_authority_revision: row.get(8)?,
        action_type: row.get(9)?,
        risk_class: row.get(10)?,
        tool_adapter: row.get(11)?,
        proposal_state: row.get(12)?,
        expires_at_utc: row.get(13)?,
        job_id: row.get(14)?,
        authorization_decision_id: row.get(15)?,
        approval_id: row.get(16)?,
    })
}

#[derive(Debug)]
struct CheckpointAuthorityRow {
    proposal_plan_hash: String,
    payload_hash: String,
    agent_revision: i64,
    assignment_id: String,
    assignment_revision: i64,
    policy_id: String,
    policy_revision: i64,
    grant_revision: i64,
    proposal_state: String,
    approval_id: Option<String>,
    approval_plan_hash: Option<String>,
    approval_payload_hash: Option<String>,
    approval_agent_revision: Option<i64>,
    approval_assignment_revision: Option<i64>,
    approval_policy_revision: Option<i64>,
    approval_grant_revision: Option<i64>,
    approval_connection_authority_revision: Option<i64>,
    approval_state: Option<String>,
    runtime_plan_hash: String,
    connection_authority_revision: i64,
    grant_id: String,
    risk_class: String,
    action_type: String,
    tool_adapter: String,
}

fn checkpoint_authority_from_row(row: &Row<'_>) -> rusqlite::Result<CheckpointAuthorityRow> {
    Ok(CheckpointAuthorityRow {
        proposal_plan_hash: row.get(0)?,
        payload_hash: row.get(1)?,
        agent_revision: row.get(2)?,
        assignment_id: row.get(3)?,
        assignment_revision: row.get(4)?,
        policy_id: row.get(5)?,
        policy_revision: row.get(6)?,
        grant_revision: row.get(7)?,
        proposal_state: row.get(8)?,
        approval_id: row.get(9)?,
        approval_plan_hash: row.get(10)?,
        approval_payload_hash: row.get(11)?,
        approval_agent_revision: row.get(12)?,
        approval_assignment_revision: row.get(13)?,
        approval_policy_revision: row.get(14)?,
        approval_grant_revision: row.get(15)?,
        approval_connection_authority_revision: row.get(16)?,
        approval_state: row.get(17)?,
        runtime_plan_hash: row.get(18)?,
        connection_authority_revision: row.get(19)?,
        grant_id: row.get(20)?,
        risk_class: row.get(21)?,
        action_type: row.get(22)?,
        tool_adapter: row.get(23)?,
    })
}

#[derive(Debug)]
struct ActionEvidenceRow {
    receipt_id: String,
    receipt_hash: String,
    outcome: String,
    result_code: String,
    safe_result_hash: Option<String>,
    proposal_id: String,
    approval_id: Option<String>,
    plan_hash: String,
    payload_hash: String,
    tool_adapter: String,
}

fn action_evidence_from_row(row: &Row<'_>) -> rusqlite::Result<ActionEvidenceRow> {
    Ok(ActionEvidenceRow {
        receipt_id: row.get(0)?,
        receipt_hash: row.get(1)?,
        outcome: row.get(2)?,
        result_code: row.get(3)?,
        safe_result_hash: row.get(4)?,
        proposal_id: row.get(5)?,
        approval_id: row.get(6)?,
        plan_hash: row.get(7)?,
        payload_hash: row.get(8)?,
        tool_adapter: row.get(9)?,
    })
}

#[derive(Debug)]
struct WrapperReceiptEvidence {
    receipt_id: String,
    receipt_hash: String,
    result_code: String,
    safe_result_hash: Option<String>,
}

fn wrapper_receipt_from_row(row: &Row<'_>) -> rusqlite::Result<WrapperReceiptEvidence> {
    Ok(WrapperReceiptEvidence {
        receipt_id: row.get(0)?,
        receipt_hash: row.get(1)?,
        result_code: row.get(2)?,
        safe_result_hash: row.get(3)?,
    })
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    ensure_worker(connection)?;
    reconcile(connection)?;
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
        "supervised action orchestration migration is not registered exactly once"
    );
    for table in [
        "agent_supervised_action_checkpoints",
        "agent_supervised_action_receipts",
        "agent_supervised_compensation_receipts",
        "agent_supervised_action_events",
        "agent_supervised_action_state",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let tool_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_tool_catalog WHERE tool_key='action.supervised' AND adapter_key='action.supervised' AND approval_requirement='proposal' AND risk_class IN ('reversible','external_side_effect','high_risk') AND state='active'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        tool_count == 1,
        "supervised action tool catalog entry is invalid"
    );
    let incomplete: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_supervised_action_checkpoints c LEFT JOIN agent_supervised_action_receipts r ON r.checkpoint_id=c.checkpoint_id WHERE c.state IN ('completed','failed') AND r.checkpoint_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        incomplete == 0,
        "completed supervised checkpoints are missing immutable receipts"
    );
    let broken_links: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_supervised_action_checkpoints c JOIN agent_runtime_plan_steps s ON s.step_id=c.step_id JOIN agent_runtime_plans p ON p.plan_id=c.plan_id JOIN agent_action_proposals a ON a.proposal_id=c.proposal_id WHERE s.plan_id<>c.plan_id OR s.job_id<>c.job_id OR p.agent_id<>c.agent_id OR a.job_id<>c.job_id OR a.agent_id<>c.agent_id",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        broken_links == 0,
        "supervised checkpoint evidence chain is inconsistent"
    );
    let unconsumed: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_supervised_action_checkpoints c JOIN agent_action_approvals a ON a.approval_id=c.approval_id WHERE c.state='completed' AND a.state<>'consumed'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        unconsumed == 0,
        "completed supervised action retained an unconsumed approval"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    reconcile(connection)?;
    connection.execute(
        "DELETE FROM agent_supervised_action_events WHERE created_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM agent_supervised_action_events WHERE event_id NOT IN (SELECT event_id FROM agent_supervised_action_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1)",
        params![MAX_EVENTS],
    )?;
    let receipts: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_supervised_action_receipts",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        receipts <= MAX_RECEIPTS,
        "supervised action receipt retention requires archival"
    );
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/action-orchestration", get(snapshot_handler))
        .route(
            "/v1/action-orchestration/plans/create",
            post(create_plan_handler),
        )
        .route("/v1/action-orchestration/run-once", post(run_once_handler))
        .route(
            "/v1/action-orchestration/checkpoints/rollback",
            post(rollback_handler),
        )
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now() + StdDuration::from_secs(4);
    let mut interval = tokio::time::interval_at(start, StdDuration::from_secs(2));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let cycle_state = state.clone();
                match tokio::task::spawn_blocking(move || process_cycle(&cycle_state)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => warn!(?error, "supervised action orchestration cycle failed"),
                    Err(error) => error!(?error, "supervised action orchestration task failed"),
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

async fn snapshot_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<SupervisedOrchestrationSnapshot> {
    run_blocking(
        move || snapshot(&state),
        "action_orchestration_snapshot_failed",
    )
    .await
}

async fn create_plan_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateSupervisedPlanRequest>,
) -> ApiResult<SupervisedOrchestrationSnapshot> {
    run_blocking(
        move || {
            create_supervised_plan(&state, request)?;
            snapshot(&state)
        },
        "action_orchestration_plan_create_failed",
    )
    .await
}

async fn run_once_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<SupervisedOrchestrationSnapshot> {
    run_blocking(
        move || {
            process_cycle(&state)?;
            snapshot(&state)
        },
        "action_orchestration_cycle_failed",
    )
    .await
}

async fn rollback_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RollbackCheckpointRequest>,
) -> ApiResult<SupervisedOrchestrationSnapshot> {
    run_blocking(
        move || {
            rollback_checkpoint(&state, request, "local_control_center")?;
            snapshot(&state)
        },
        "action_orchestration_rollback_failed",
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
        .map_err(|error| api_error(code, anyhow::anyhow!("orchestration task failed: {error}")))?
        .map(Json)
        .map_err(|error| api_error(code, error))
}

fn create_supervised_plan(
    state: &AppState,
    request: CreateSupervisedPlanRequest,
) -> Result<String> {
    let supervised_steps = request
        .plan
        .steps
        .iter()
        .filter(|step| step.tool_key == SUPERVISED_TOOL_KEY)
        .collect::<Vec<_>>();
    ensure!(
        !supervised_steps.is_empty(),
        "a supervised plan must contain at least one action.supervised checkpoint"
    );
    for step in supervised_steps {
        ensure!(
            step.action_type == SUPERVISED_TOOL_KEY,
            "supervised step action type must match the catalog tool"
        );
        ensure!(
            step.job.job_type == SUPERVISED_JOB_TYPE
                && step.job.capability_key == "action.propose"
                && step.job.operation == "propose",
            "supervised steps must use the certified action.propose job contract"
        );
        ensure!(
            step.job.approval_id.is_none() && step.job.plan_hash.is_none(),
            "approval identity is created only by the Phase 16D proposal lifecycle"
        );
        parse_proposal_input(&step.job.private_input)?;
    }
    wrapper_runtime::create_plan(state, request.plan)
}

fn ensure_worker(connection: &Connection) -> Result<String> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT s.worker_id FROM agent_supervised_action_state s JOIN wrapper_job_workers w ON w.worker_id=s.worker_id WHERE s.singleton_id=1 AND s.state='active' AND w.state='active'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(worker_id) = existing {
        return Ok(worker_id);
    }
    let worker = wrapper_jobs::register_worker(
        connection,
        wrapper_jobs::RegisterWorkerRequest {
            worker_kind: "agent".to_owned(),
            display_name: WORKER_NAME.to_owned(),
            allowed_job_types: vec![SUPERVISED_JOB_TYPE.to_owned()],
            max_concurrent_jobs: 1,
        },
    )?;
    let now = now_utc();
    connection.execute(
        "INSERT INTO agent_supervised_action_state (singleton_id,worker_id,orchestration_revision,state,created_at_utc,updated_at_utc) VALUES (1,?1,1,'active',?2,?2) ON CONFLICT(singleton_id) DO UPDATE SET worker_id=excluded.worker_id,orchestration_revision=agent_supervised_action_state.orchestration_revision+1,state='active',last_error_code=NULL,updated_at_utc=excluded.updated_at_utc",
        params![worker.worker_id, now],
    )?;
    Ok(worker.worker_id)
}

fn process_cycle(state: &AppState) -> Result<usize> {
    {
        let connection = state.connection()?;
        reconcile(&connection)?;
    }
    let mut completed = process_existing_checkpoints(state)?;
    let worker_id = {
        let connection = state.connection()?;
        ensure_worker(&connection)?
    };
    let claimed = {
        let connection = state.connection()?;
        wrapper_jobs::claim_jobs(
            &connection,
            wrapper_jobs::ClaimJobsRequest {
                worker_id: worker_id.clone(),
                limit: Some(1),
            },
        )?
    };
    let mut last_error: Option<String> = None;
    for leased in claimed.jobs {
        match process_proposal_job(state, &worker_id, leased) {
            Ok(()) => completed += 1,
            Err(error) => {
                last_error = Some("proposal_checkpoint_failed".to_owned());
                warn!(?error, "supervised proposal checkpoint failed");
            }
        }
    }
    process_automatic_compensations(state)?;
    let connection = state.connection()?;
    connection.execute(
        "UPDATE agent_supervised_action_state SET last_cycle_at_utc=?1,last_error_code=?2,updated_at_utc=?1 WHERE singleton_id=1",
        params![now_utc(), last_error],
    )?;
    Ok(completed)
}

fn process_proposal_job(
    state: &AppState,
    worker_id: &str,
    leased: wrapper_jobs::LeasedJob,
) -> Result<()> {
    {
        let connection = state.connection()?;
        wrapper_jobs::start_job(
            &connection,
            wrapper_jobs::WorkerLeaseRequest {
                worker_id: worker_id.to_owned(),
                job_id: leased.job.job_id.clone(),
                lease_token: leased.lease_token.clone(),
            },
        )?;
    }
    let context = match proposal_job_context(state, &leased.job) {
        Ok(value) => value,
        Err(error) => {
            fail_leased_job(state, worker_id, &leased, "supervised_authority_denied")?;
            return Err(error);
        }
    };
    let input = match parse_proposal_input(&leased.private_input) {
        Ok(value) => value,
        Err(error) => {
            fail_leased_job(state, worker_id, &leased, "supervised_payload_invalid")?;
            return Err(error);
        }
    };
    let proposal_payload_hash = hash_json(&input.private_payload)?;
    let job_receipt = {
        let connection = state.connection()?;
        wrapper_jobs::complete_job(
            &connection,
            wrapper_jobs::CompleteJobRequest {
                worker_id: worker_id.to_owned(),
                job_id: leased.job.job_id.clone(),
                lease_token: leased.lease_token,
                private_result: json!({
                    "proposal_ready": true,
                    "safe_summary": input.safe_summary.clone(),
                    "private_payload_hash": proposal_payload_hash.clone(),
                    "phase16d_proposal_required": true
                }),
                private_provenance: json!({
                    "source": "supervised_action_orchestrator",
                    "private_payload_exposed": false
                }),
                source_count: 0,
                source_types: Vec::new(),
                evidence_hash: Some(proposal_payload_hash.clone()),
                actual_token_count: Some(0),
                result_code: "action_proposal_job_completed".to_owned(),
            },
        )?
    };
    let proposal_id = {
        let connection = state.connection()?;
        match wrapper_agents::create_proposal(
            &connection,
            wrapper_agents::CreateProposalRequest {
                agent_id: context.agent_id.clone(),
                assignment_id: context.assignment_id.clone(),
                job_id: context.job_id.clone(),
                policy_id: input.policy_id.clone(),
                title: input.title.clone(),
                rationale: input.rationale.clone(),
                safe_summary: input.safe_summary.clone(),
                private_payload: input.private_payload,
                requested_by_user_id: input.requested_by_user_id.clone(),
                expires_minutes: input.expires_minutes,
            },
        ) {
            Ok(proposal_id) => proposal_id,
            Err(error) => {
                fail_completed_proposal_job(
                    state,
                    &context,
                    None,
                    &job_receipt,
                    "proposal_creation_failed",
                    &error,
                )?;
                return Err(error).context("Phase 16D proposal creation failed");
            }
        }
    };
    if let Err(error) = create_checkpoint(
        state,
        &context,
        &proposal_id,
        &input.compensation_mode,
        &job_receipt,
    ) {
        fail_completed_proposal_job(
            state,
            &context,
            Some(&proposal_id),
            &job_receipt,
            "checkpoint_creation_failed",
            &error,
        )?;
        return Err(error).context("supervised checkpoint creation failed");
    }
    Ok(())
}

fn fail_completed_proposal_job(
    state: &AppState,
    context: &ProposalJobContext,
    proposal_id: Option<&str>,
    job_receipt: &wrapper_jobs::ExecutionReceiptSummary,
    failure_code: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let now = now_utc();
    if let Some(proposal_id) = proposal_id {
        transaction.execute(
            "UPDATE agent_action_proposals SET state='cancelled',failure_code=?1,completed_at_utc=COALESCE(completed_at_utc,?2),updated_at_utc=?2 WHERE proposal_id=?3 AND state IN ('proposed','awaiting_approval','approved','executing')",
            params![failure_code, now, proposal_id],
        )?;
        transaction.execute(
            "UPDATE agent_action_approvals SET state='cancelled' WHERE proposal_id=?1 AND state IN ('pending','approved')",
            params![proposal_id],
        )?;
    }
    let runtime_receipt_id = Uuid::new_v4().to_string();
    let runtime_document = json!({
        "schema": "homeserver.agent-runtime-supervised-receipt.v1",
        "receipt_id": runtime_receipt_id,
        "plan_id": context.plan_id,
        "step_id": context.step_id,
        "job_id": context.job_id,
        "agent_id": context.agent_id,
        "wrapper_id": context.wrapper_id,
        "connection_id": context.connection_id,
        "tool_key": SUPERVISED_TOOL_KEY,
        "adapter_key": SUPERVISED_TOOL_KEY,
        "outcome": "failed",
        "result_code": failure_code,
        "wrapper_job_receipt_id": job_receipt.receipt_id,
        "wrapper_job_receipt_hash": job_receipt.receipt_hash,
        "completed_at_utc": now
    });
    let runtime_receipt_hash = hash_json(&runtime_document)?;
    transaction.execute(
        "INSERT OR IGNORE INTO agent_runtime_receipts (receipt_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,tool_key,adapter_key,outcome,result_code,job_receipt_id,job_receipt_hash,runtime_receipt_hash,completed_at_utc,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'action.supervised','action.supervised','failed',?8,?9,?10,?11,?12,?12)",
        params![runtime_receipt_id,context.plan_id,context.step_id,context.job_id,context.agent_id,context.wrapper_id,context.connection_id,failure_code,job_receipt.receipt_id,job_receipt.receipt_hash,runtime_receipt_hash,now],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plan_steps SET state='failed',failure_code=?1,result_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE step_id=?3 AND state IN ('queued','leased','running')",
        params![failure_code, now, context.step_id],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plans SET state='failed',failure_code=?1,completed_at_utc=COALESCE(completed_at_utc,?2),updated_at_utc=?2 WHERE plan_id=?3 AND state IN ('queued','running')",
        params![failure_code, now, context.plan_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            checkpoint_id: None,
            plan_id: Some(&context.plan_id),
            step_id: Some(&context.step_id),
            job_id: Some(&context.job_id),
            proposal_id,
            event_type: "agent.supervised_checkpoint_failed",
            outcome: "error",
            actor_type: "system",
            actor_id: ORCHESTRATOR_ACTOR,
            detail_code: failure_code,
            metadata: json!({
                "error_hash": hash_text(&error.to_string()),
                "runtime_receipt_hash": runtime_receipt_hash,
                "fail_closed": true
            }),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn proposal_job_context(
    state: &AppState,
    job: &wrapper_jobs::JobSummary,
) -> Result<ProposalJobContext> {
    ensure!(
        job.job_type == SUPERVISED_JOB_TYPE
            && job.capability_key == "action.propose"
            && job.operation == "propose",
        "job is not a supervised action proposal"
    );
    ensure!(
        job.submitted_by_type == "agent",
        "supervised action job is not agent submitted"
    );
    let connection = state.connection()?;
    let context = connection.query_row(
        "SELECT s.plan_id,s.step_id,s.sequence_number,p.plan_hash,p.agent_id,b.assignment_id,j.wrapper_id,j.connection_id,j.grant_id,j.grant_revision,a.connection_authority_revision,j.authorization_decision_id FROM agent_runtime_plan_steps s JOIN agent_runtime_plans p ON p.plan_id=s.plan_id JOIN agent_job_bindings b ON b.job_id=s.job_id JOIN wrapper_jobs j ON j.job_id=s.job_id JOIN wrapper_job_authority_snapshots a ON a.job_id=j.job_id WHERE s.job_id=?1 AND s.tool_key='action.supervised' AND s.state IN ('queued','leased','running') AND p.state IN ('queued','running')",
        params![job.job_id],
        |row| {
            Ok(ProposalJobContext {
                plan_id: row.get(0)?,
                step_id: row.get(1)?,
                sequence_number: row.get::<_, i64>(2)?.max(1) as u32,
                runtime_plan_hash: row.get(3)?,
                agent_id: row.get(4)?,
                assignment_id: row.get(5)?,
                wrapper_id: row.get(6)?,
                connection_id: row.get(7)?,
                grant_id: row.get(8)?,
                grant_revision: row.get::<_, i64>(9)?.max(0) as u64,
                connection_authority_revision: row.get::<_, i64>(10)?.max(0) as u64,
                authorization_decision_id: row.get(11)?,
                job_id: job.job_id.clone(),
            })
        },
    )?;
    ensure!(
        context.agent_id == job.submitted_by_id,
        "supervised plan agent does not match job submitter"
    );
    let incomplete: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runtime_plan_steps WHERE plan_id=?1 AND sequence_number<?2 AND state<>'completed'",
        params![context.plan_id, i64::from(context.sequence_number)],
        |row| row.get(0),
    )?;
    ensure!(
        incomplete == 0,
        "supervised action predecessor is incomplete"
    );
    Ok(context)
}

fn parse_proposal_input(value: &Value) -> Result<ProposalInput> {
    let object = value
        .as_object()
        .context("supervised action input must be an object")?;
    let policy_id = validate_uuid(
        object
            .get("policy_id")
            .and_then(Value::as_str)
            .context("supervised action policy_id is required")?,
        "policy ID",
    )?;
    let title = bounded_text(
        object
            .get("title")
            .and_then(Value::as_str)
            .context("supervised action title is required")?,
        1,
        180,
        "title",
    )?;
    let rationale = bounded_text(
        object
            .get("rationale")
            .and_then(Value::as_str)
            .context("supervised action rationale is required")?,
        1,
        4000,
        "rationale",
    )?;
    let requested_by_user_id = bounded_text(
        object
            .get("requested_by_user_id")
            .and_then(Value::as_str)
            .context("supervised action requested_by_user_id is required")?,
        1,
        160,
        "requested-by user ID",
    )?;
    let expires_minutes = object
        .get("expires_minutes")
        .and_then(Value::as_u64)
        .unwrap_or(60)
        .clamp(1, 10_080) as u32;
    let compensation_mode = object
        .get("compensation_mode")
        .and_then(Value::as_str)
        .unwrap_or("manual");
    ensure!(
        matches!(compensation_mode, "manual" | "automatic" | "disabled"),
        "compensation mode is invalid"
    );
    let safe_summary = object
        .get("safe_summary")
        .cloned()
        .unwrap_or_else(|| json!({}));
    wrapper_agents::validate_safe_summary(&safe_summary)?;
    Ok(ProposalInput {
        policy_id,
        title,
        rationale,
        safe_summary,
        private_payload: object
            .get("private_payload")
            .cloned()
            .context("supervised action private_payload is required")?,
        requested_by_user_id,
        expires_minutes,
        compensation_mode: compensation_mode.to_owned(),
    })
}

fn create_checkpoint(
    state: &AppState,
    context: &ProposalJobContext,
    proposal_id: &str,
    compensation_mode: &str,
    job_receipt: &wrapper_jobs::ExecutionReceiptSummary,
) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let row = transaction.query_row(
        "SELECT p.plan_hash,p.payload_hash,p.agent_revision,p.assignment_id,p.assignment_revision,p.policy_id,p.policy_revision,p.grant_revision,p.connection_authority_revision,p.action_type,p.risk_class,e.tool_adapter,p.state,p.expires_at_utc,p.job_id,p.authorization_decision_id,a.approval_id FROM agent_action_proposals p JOIN agent_execution_policies e ON e.policy_id=p.policy_id LEFT JOIN agent_action_approvals a ON a.proposal_id=p.proposal_id WHERE p.proposal_id=?1",
        params![proposal_id],
        proposal_checkpoint_from_row,
    )?;
    ensure!(
        row.job_id == context.job_id,
        "proposal job identity changed"
    );
    ensure!(
        row.authorization_decision_id == context.authorization_decision_id,
        "proposal authorization decision changed"
    );
    ensure!(
        row.grant_revision.max(0) as u64 == context.grant_revision,
        "proposal grant revision changed"
    );
    ensure!(
        row.connection_authority_revision.max(0) as u64 == context.connection_authority_revision,
        "proposal connection authority revision changed"
    );
    ensure!(
        row.assignment_id == context.assignment_id,
        "proposal assignment identity changed"
    );
    ensure!(
        row.approval_id.is_some(),
        "supervised action did not create an approval checkpoint"
    );
    ensure!(
        matches!(
            row.risk_class.as_str(),
            "reversible" | "external_side_effect" | "high_risk"
        ),
        "supervised action risk class is not approval-gated"
    );
    ensure!(
        job_receipt.job_id == context.job_id
            && job_receipt.result_code == "action_proposal_job_completed"
            && job_receipt.safe_result_hash.is_some(),
        "wrapper proposal-job receipt evidence is incomplete"
    );
    let checkpoint_id = Uuid::new_v4().to_string();
    let state_value = if row.proposal_state == "approved" {
        "approved"
    } else {
        "awaiting_approval"
    };
    let compensation_supported = row.tool_adapter == "report.save";
    let compensation_state = if compensation_mode == "disabled" {
        "disabled"
    } else if compensation_supported {
        "available"
    } else {
        "not_supported"
    };
    let now = now_utc();
    transaction.execute(
        "INSERT INTO agent_supervised_action_checkpoints (checkpoint_id,plan_id,step_id,sequence_number,job_id,agent_id,wrapper_id,connection_id,proposal_id,approval_id,policy_id,action_type,risk_class,tool_adapter,state,compensation_mode,compensation_supported,compensation_state,runtime_plan_hash,proposal_plan_hash,payload_hash,agent_revision,assignment_id,assignment_revision,policy_revision,grant_id,grant_revision,connection_authority_revision,authorization_decision_id,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?31)",
        params![
            checkpoint_id,
            context.plan_id,
            context.step_id,
            i64::from(context.sequence_number),
            context.job_id,
            context.agent_id,
            context.wrapper_id,
            context.connection_id,
            proposal_id,
            row.approval_id,
            row.policy_id,
            row.action_type,
            row.risk_class,
            row.tool_adapter,
            state_value,
            compensation_mode,
            i64::from(compensation_supported),
            compensation_state,
            context.runtime_plan_hash,
            row.proposal_plan_hash,
            row.payload_hash,
            row.agent_revision,
            row.assignment_id,
            row.assignment_revision,
            row.policy_revision,
            context.grant_id,
            row.grant_revision,
            row.connection_authority_revision,
            context.authorization_decision_id,
            row.expires_at_utc,
            now
        ],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plans SET state='running',updated_at_utc=?1 WHERE plan_id=?2 AND state='queued'",
        params![now, context.plan_id],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plan_steps SET state='running',started_at_utc=COALESCE(started_at_utc,?1),result_code='awaiting_supervised_approval',updated_at_utc=?1 WHERE step_id=?2",
        params![now, context.step_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            checkpoint_id: Some(&checkpoint_id),
            plan_id: Some(&context.plan_id),
            step_id: Some(&context.step_id),
            job_id: Some(&context.job_id),
            proposal_id: Some(proposal_id),
            event_type: "agent.supervised_checkpoint_created",
            outcome: "success",
            actor_type: "worker",
            actor_id: ORCHESTRATOR_ACTOR,
            detail_code: state_value,
            metadata: json!({
                "runtime_plan_hash": context.runtime_plan_hash,
                "proposal_plan_hash": row.proposal_plan_hash,
                "payload_hash": row.payload_hash,
                "wrapper_job_receipt_hash": job_receipt.receipt_hash,
                "approval_id": row.approval_id,
                "private_payload_exposed": false
            }),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn process_existing_checkpoints(state: &AppState) -> Result<usize> {
    let checkpoint_ids = {
        let connection = state.connection()?;
        let mut statement = connection.prepare(
            "SELECT checkpoint_id FROM agent_supervised_action_checkpoints WHERE state IN ('awaiting_approval','approved','executing') ORDER BY created_at_utc,checkpoint_id LIMIT 100",
        )?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut completed = 0;
    for checkpoint_id in checkpoint_ids {
        match process_checkpoint(state, &checkpoint_id) {
            Ok(true) => completed += 1,
            Ok(false) => {}
            Err(error) => {
                let checkpoint = {
                    let connection = state.connection()?;
                    read_checkpoint(&connection, &checkpoint_id)?
                };
                fail_checkpoint(
                    state,
                    &checkpoint,
                    "checkpoint_authority_denied",
                    "authority_revalidation_failed",
                )?;
                warn!(error_hash = %hash_text(&error.to_string()), "supervised checkpoint failed closed");
                completed += 1;
            }
        }
    }
    Ok(completed)
}

fn process_checkpoint(state: &AppState, checkpoint_id: &str) -> Result<bool> {
    let checkpoint = {
        let connection = state.connection()?;
        read_checkpoint(&connection, checkpoint_id)?
    };
    if parse_utc(&checkpoint.expires_at_utc, "checkpoint expiration")? <= Utc::now() {
        fail_checkpoint(state, &checkpoint, "checkpoint_expired", "approval_expired")?;
        return Ok(true);
    }
    let status: (String, Option<String>, String) = {
        let connection = state.connection()?;
        connection.query_row(
            "SELECT p.state,a.state,r.state FROM agent_action_proposals p LEFT JOIN agent_action_approvals a ON a.proposal_id=p.proposal_id JOIN agent_runtime_plans r ON r.plan_id=?2 WHERE p.proposal_id=?1",
            params![checkpoint.proposal_id, checkpoint.plan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?
    };
    if matches!(status.2.as_str(), "failed" | "cancelled" | "expired") {
        fail_checkpoint(
            state,
            &checkpoint,
            "runtime_plan_no_longer_active",
            status.2.as_str(),
        )?;
        return Ok(true);
    }
    match status.0.as_str() {
        "awaiting_approval" => {
            ensure!(
                status.1.as_deref() == Some("pending"),
                "supervised action approval is not pending"
            );
            ensure_checkpoint_authority(state, &checkpoint, "pending")?;
            Ok(false)
        }
        "approved" => {
            ensure!(
                status.1.as_deref() == Some("approved"),
                "supervised action approval is not approved"
            );
            ensure_checkpoint_authority(state, &checkpoint, "approved")?;
            execute_checkpoint(state, &checkpoint)?;
            Ok(true)
        }
        "completed" | "failed" => {
            finalize_checkpoint(state, &checkpoint)?;
            Ok(true)
        }
        "rejected" => {
            fail_checkpoint(state, &checkpoint, "action_rejected", "user_rejected")?;
            Ok(true)
        }
        "cancelled" => {
            fail_checkpoint(
                state,
                &checkpoint,
                "action_cancelled",
                "authority_cancelled",
            )?;
            Ok(true)
        }
        "expired" => {
            fail_checkpoint(state, &checkpoint, "action_expired", "approval_expired")?;
            Ok(true)
        }
        other => bail!("unsupported supervised proposal state: {other}"),
    }
}

fn ensure_checkpoint_authority(
    state: &AppState,
    checkpoint: &CheckpointRecord,
    expected_approval_state: &str,
) -> Result<()> {
    let connection = state.connection()?;
    let current = connection.query_row(
        "SELECT p.plan_hash,p.payload_hash,p.agent_revision,p.assignment_id,p.assignment_revision,p.policy_id,p.policy_revision,p.grant_revision,p.state,a.approval_id,a.plan_hash,a.payload_hash,a.agent_revision,a.assignment_revision,a.policy_revision,a.grant_revision,a.connection_authority_revision,a.state,r.plan_hash,p.connection_authority_revision,p.grant_id,p.risk_class,p.action_type,e.tool_adapter FROM agent_action_proposals p LEFT JOIN agent_action_approvals a ON a.proposal_id=p.proposal_id JOIN agent_runtime_plans r ON r.plan_id=?2 JOIN agent_execution_policies e ON e.policy_id=p.policy_id WHERE p.proposal_id=?1",
        params![checkpoint.proposal_id, checkpoint.plan_id],
        checkpoint_authority_from_row,
    )?;
    ensure!(
        current.proposal_plan_hash == checkpoint.proposal_plan_hash
            && current.payload_hash == checkpoint.payload_hash
            && current.agent_revision.max(0) as u64 == checkpoint.agent_revision
            && current.assignment_id == checkpoint.assignment_id
            && current.assignment_revision.max(0) as u64 == checkpoint.assignment_revision
            && current.policy_id == checkpoint.policy_id
            && current.policy_revision.max(0) as u64 == checkpoint.policy_revision
            && current.grant_revision.max(0) as u64 == checkpoint.grant_revision
            && current.runtime_plan_hash == checkpoint.runtime_plan_hash
            && current.connection_authority_revision.max(0) as u64
                == checkpoint.connection_authority_revision
            && current.grant_id == checkpoint.grant_id
            && current.risk_class == checkpoint.risk_class
            && current.action_type == checkpoint.action_type
            && current.tool_adapter == checkpoint.tool_adapter,
        "supervised checkpoint authority changed"
    );
    ensure!(
        matches!(
            current.proposal_state.as_str(),
            "awaiting_approval" | "approved"
        ),
        "supervised proposal is not resumable"
    );
    let approval_id = checkpoint
        .approval_id
        .as_deref()
        .context("supervised checkpoint approval identity is missing")?;
    ensure!(
        current.approval_id.as_deref() == Some(approval_id)
            && current.approval_plan_hash.as_deref()
                == Some(checkpoint.proposal_plan_hash.as_str())
            && current.approval_payload_hash.as_deref() == Some(checkpoint.payload_hash.as_str())
            && current
                .approval_agent_revision
                .map(|value| value.max(0) as u64)
                == Some(checkpoint.agent_revision)
            && current
                .approval_assignment_revision
                .map(|value| value.max(0) as u64)
                == Some(checkpoint.assignment_revision)
            && current
                .approval_policy_revision
                .map(|value| value.max(0) as u64)
                == Some(checkpoint.policy_revision)
            && current
                .approval_grant_revision
                .map(|value| value.max(0) as u64)
                == Some(checkpoint.grant_revision)
            && current
                .approval_connection_authority_revision
                .map(|value| value.max(0) as u64)
                == Some(checkpoint.connection_authority_revision)
            && current.approval_state.as_deref() == Some(expected_approval_state),
        "supervised approval evidence changed"
    );
    Ok(())
}

fn execute_checkpoint(state: &AppState, checkpoint: &CheckpointRecord) -> Result<()> {
    {
        let connection = state.connection()?;
        connection.execute(
            "UPDATE agent_supervised_action_checkpoints SET state='executing',updated_at_utc=?1 WHERE checkpoint_id=?2 AND state IN ('awaiting_approval','approved')",
            params![now_utc(), checkpoint.checkpoint_id],
        )?;
    }
    let receipt = {
        let connection = state.connection()?;
        wrapper_agents::execute_proposal_as_orchestrator(
            &connection,
            wrapper_agents::ExecuteProposalRequest {
                proposal_id: checkpoint.proposal_id.clone(),
                plan_hash: checkpoint.proposal_plan_hash.clone(),
                actor_user_id: ORCHESTRATOR_ACTOR.to_owned(),
                idempotency_key: format!("phase18-{}", checkpoint.checkpoint_id),
            },
        )?
    };
    ensure!(
        receipt.proposal_id == checkpoint.proposal_id,
        "executed proposal receipt identity changed"
    );
    finalize_checkpoint(state, checkpoint)
}

fn finalize_checkpoint(state: &AppState, checkpoint: &CheckpointRecord) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let action = transaction.query_row(
        "SELECT receipt_id,receipt_hash,outcome,result_code,safe_result_hash,proposal_id,approval_id,plan_hash,payload_hash,tool_adapter FROM agent_action_receipts WHERE proposal_id=?1",
        params![checkpoint.proposal_id],
        action_evidence_from_row,
    )?;
    ensure!(
        action.proposal_id == checkpoint.proposal_id
            && action.approval_id == checkpoint.approval_id
            && action.plan_hash == checkpoint.proposal_plan_hash
            && action.payload_hash == checkpoint.payload_hash
            && action.tool_adapter == checkpoint.tool_adapter,
        "action receipt evidence chain changed"
    );
    let job_receipt = read_wrapper_receipt_tx(&transaction, &checkpoint.job_id)?;
    ensure!(
        job_receipt.result_code == "action_proposal_job_completed",
        "proposal-job receipt result changed"
    );
    let private_result_hash: String = transaction.query_row(
        "SELECT private_result_hash FROM wrapper_job_private_results WHERE job_id=?1",
        params![checkpoint.job_id],
        |row| row.get(0),
    )?;
    let completed = now_utc();
    let step_outcome = if action.outcome == "completed" {
        "completed"
    } else {
        "failed"
    };
    let stored_runtime = write_terminal_receipts_tx(
        &transaction,
        checkpoint,
        &job_receipt,
        Some(&action),
        step_outcome,
        &action.result_code,
        action.safe_result_hash.as_deref(),
        &completed,
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plan_steps SET state=?1,private_result_hash=?2,safe_result_hash=?3,result_code=?4,failure_code=CASE WHEN ?1='failed' THEN ?4 ELSE NULL END,completed_at_utc=?5,updated_at_utc=?5 WHERE step_id=?6",
        params![step_outcome,private_result_hash,action.safe_result_hash,action.result_code,completed,checkpoint.step_id],
    )?;
    transaction.execute(
        "UPDATE agent_supervised_action_checkpoints SET state=?1,failure_code=CASE WHEN ?1='failed' THEN ?2 ELSE NULL END,completed_at_utc=?3,updated_at_utc=?3 WHERE checkpoint_id=?4",
        params![step_outcome,action.result_code,completed,checkpoint.checkpoint_id],
    )?;
    if step_outcome == "completed" {
        transaction.execute(
            "UPDATE wrapper_jobs SET available_at_utc=?1,updated_at_utc=?1 WHERE job_id=(SELECT next.job_id FROM agent_runtime_plan_steps current JOIN agent_runtime_plan_steps next ON next.plan_id=current.plan_id AND next.sequence_number=current.sequence_number+1 WHERE current.step_id=?2) AND state='queued'",
            params![completed, checkpoint.step_id],
        )?;
    }
    refresh_plan_state_tx(&transaction, &checkpoint.plan_id)?;
    record_event_tx(
        &transaction,
        EventEvidence {
            checkpoint_id: Some(&checkpoint.checkpoint_id),
            plan_id: Some(&checkpoint.plan_id),
            step_id: Some(&checkpoint.step_id),
            job_id: Some(&checkpoint.job_id),
            proposal_id: Some(&checkpoint.proposal_id),
            event_type: "agent.supervised_action_finalized",
            outcome: if step_outcome == "completed" {
                "success"
            } else {
                "error"
            },
            actor_type: "system",
            actor_id: ORCHESTRATOR_ACTOR,
            detail_code: &action.result_code,
            metadata: json!({
                "action_receipt_hash": action.receipt_hash,
                "runtime_receipt_hash": stored_runtime.1,
                "approval_consumed_once": true,
                "private_result_exposed": false
            }),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn read_wrapper_receipt_tx(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<WrapperReceiptEvidence> {
    transaction
        .query_row(
            "SELECT receipt_id,receipt_hash,result_code,safe_result_hash FROM wrapper_job_execution_receipts WHERE job_id=?1",
            params![job_id],
            wrapper_receipt_from_row,
        )
        .context("proposal-job execution receipt is missing")
}

#[allow(clippy::too_many_arguments)]
fn write_terminal_receipts_tx(
    transaction: &Transaction<'_>,
    checkpoint: &CheckpointRecord,
    job_receipt: &WrapperReceiptEvidence,
    action: Option<&ActionEvidenceRow>,
    outcome: &str,
    result_code: &str,
    safe_result_hash: Option<&str>,
    completed: &str,
) -> Result<(String, String)> {
    ensure!(
        matches!(outcome, "completed" | "failed" | "cancelled" | "denied"),
        "supervised terminal outcome is invalid"
    );
    let runtime_receipt_id = Uuid::new_v4().to_string();
    let runtime_document = json!({
        "schema": "homeserver.agent-runtime-supervised-receipt.v1",
        "receipt_id": runtime_receipt_id,
        "plan_id": checkpoint.plan_id,
        "step_id": checkpoint.step_id,
        "job_id": checkpoint.job_id,
        "agent_id": checkpoint.agent_id,
        "wrapper_id": checkpoint.wrapper_id,
        "connection_id": checkpoint.connection_id,
        "tool_key": SUPERVISED_TOOL_KEY,
        "adapter_key": SUPERVISED_TOOL_KEY,
        "outcome": outcome,
        "result_code": result_code,
        "wrapper_job_receipt_id": job_receipt.receipt_id,
        "wrapper_job_receipt_hash": job_receipt.receipt_hash,
        "action_receipt_id": action.map(|value| value.receipt_id.as_str()),
        "action_receipt_hash": action.map(|value| value.receipt_hash.as_str()),
        "safe_result_hash": safe_result_hash,
        "phase16e_detail_code": "proposal_job_egress_enforced",
        "completed_at_utc": completed
    });
    let runtime_receipt_hash = hash_json(&runtime_document)?;
    transaction.execute(
        "INSERT OR IGNORE INTO agent_runtime_receipts (receipt_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,tool_key,adapter_key,outcome,result_code,job_receipt_id,job_receipt_hash,safe_result_hash,runtime_receipt_hash,completed_at_utc,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'action.supervised','action.supervised',?8,?9,?10,?11,?12,?13,?14,?14)",
        params![runtime_receipt_id,checkpoint.plan_id,checkpoint.step_id,checkpoint.job_id,checkpoint.agent_id,checkpoint.wrapper_id,checkpoint.connection_id,outcome,result_code,job_receipt.receipt_id,job_receipt.receipt_hash,safe_result_hash,runtime_receipt_hash,completed],
    )?;
    let stored_runtime: (String, String) = transaction.query_row(
        "SELECT receipt_id,runtime_receipt_hash FROM agent_runtime_receipts WHERE step_id=?1",
        params![checkpoint.step_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let orchestration_receipt_id = Uuid::new_v4().to_string();
    let action_receipt_id = action.map(|value| value.receipt_id.as_str());
    let action_receipt_hash = action.map(|value| value.receipt_hash.as_str());
    let orchestration_document = json!({
        "schema": "homeserver.supervised-action-receipt.v1",
        "receipt_id": orchestration_receipt_id,
        "checkpoint_id": checkpoint.checkpoint_id,
        "plan_id": checkpoint.plan_id,
        "step_id": checkpoint.step_id,
        "job_id": checkpoint.job_id,
        "proposal_id": checkpoint.proposal_id,
        "approval_id": checkpoint.approval_id,
        "action_receipt_id": action_receipt_id,
        "action_receipt_hash": action_receipt_hash,
        "wrapper_job_receipt_id": job_receipt.receipt_id,
        "wrapper_job_receipt_hash": job_receipt.receipt_hash,
        "runtime_receipt_id": stored_runtime.0,
        "runtime_receipt_hash": stored_runtime.1,
        "runtime_plan_hash": checkpoint.runtime_plan_hash,
        "proposal_plan_hash": checkpoint.proposal_plan_hash,
        "payload_hash": checkpoint.payload_hash,
        "outcome": outcome,
        "result_code": result_code,
        "safe_result_hash": safe_result_hash,
        "phase16e_detail_code": "proposal_job_egress_enforced",
        "completed_at_utc": completed
    });
    let receipt_hash = hash_json(&orchestration_document)?;
    transaction.execute(
        "INSERT OR IGNORE INTO agent_supervised_action_receipts (receipt_id,checkpoint_id,plan_id,step_id,job_id,proposal_id,approval_id,action_receipt_id,action_receipt_hash,wrapper_job_receipt_id,wrapper_job_receipt_hash,runtime_receipt_id,runtime_receipt_hash,runtime_plan_hash,proposal_plan_hash,payload_hash,outcome,result_code,safe_result_hash,phase16e_detail_code,receipt_hash,completed_at_utc,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,'proposal_job_egress_enforced',?20,?21,?21)",
        params![orchestration_receipt_id,checkpoint.checkpoint_id,checkpoint.plan_id,checkpoint.step_id,checkpoint.job_id,checkpoint.proposal_id,checkpoint.approval_id,action_receipt_id,action_receipt_hash,job_receipt.receipt_id,job_receipt.receipt_hash,stored_runtime.0,stored_runtime.1,checkpoint.runtime_plan_hash,checkpoint.proposal_plan_hash,checkpoint.payload_hash,outcome,result_code,safe_result_hash,receipt_hash,completed],
    )?;
    Ok(stored_runtime)
}

fn fail_checkpoint(
    state: &AppState,
    checkpoint: &CheckpointRecord,
    failure_code: &str,
    detail: &str,
) -> Result<()> {
    cancel_remaining_jobs(
        state,
        &checkpoint.plan_id,
        &checkpoint.step_id,
        failure_code,
    )?;
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let now = now_utc();
    transaction.execute(
        "UPDATE agent_action_proposals SET state='cancelled',failure_code=COALESCE(failure_code,?1),completed_at_utc=COALESCE(completed_at_utc,?2),updated_at_utc=?2 WHERE proposal_id=?3 AND state IN ('proposed','awaiting_approval','approved','executing')",
        params![failure_code, now, checkpoint.proposal_id],
    )?;
    transaction.execute(
        "UPDATE agent_action_approvals SET state='cancelled' WHERE proposal_id=?1 AND state IN ('pending','approved')",
        params![checkpoint.proposal_id],
    )?;
    let job_receipt = read_wrapper_receipt_tx(&transaction, &checkpoint.job_id)?;
    let stored_runtime = write_terminal_receipts_tx(
        &transaction,
        checkpoint,
        &job_receipt,
        None,
        "failed",
        failure_code,
        None,
        &now,
    )?;
    transaction.execute(
        "UPDATE agent_supervised_action_checkpoints SET state='failed',failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE checkpoint_id=?3 AND state NOT IN ('completed','failed','cancelled','expired')",
        params![failure_code, now, checkpoint.checkpoint_id],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plan_steps SET state='failed',failure_code=?1,result_code=?2,completed_at_utc=?3,updated_at_utc=?3 WHERE step_id=?4 AND state IN ('queued','leased','running')",
        params![failure_code, detail, now, checkpoint.step_id],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plans SET state='failed',failure_code=?1,completed_at_utc=COALESCE(completed_at_utc,?2),updated_at_utc=?2 WHERE plan_id=?3 AND state IN ('queued','running')",
        params![failure_code, now, checkpoint.plan_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            checkpoint_id: Some(&checkpoint.checkpoint_id),
            plan_id: Some(&checkpoint.plan_id),
            step_id: Some(&checkpoint.step_id),
            job_id: Some(&checkpoint.job_id),
            proposal_id: Some(&checkpoint.proposal_id),
            event_type: "agent.supervised_action_failed",
            outcome: "error",
            actor_type: "system",
            actor_id: ORCHESTRATOR_ACTOR,
            detail_code: failure_code,
            metadata: json!({
                "detail": detail,
                "runtime_receipt_hash": stored_runtime.1,
                "fail_closed": true
            }),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn fail_leased_job(
    state: &AppState,
    worker_id: &str,
    leased: &wrapper_jobs::LeasedJob,
    failure_code: &str,
) -> Result<()> {
    let connection = state.connection()?;
    let _ = wrapper_jobs::fail_job(
        &connection,
        wrapper_jobs::FailJobRequest {
            worker_id: worker_id.to_owned(),
            job_id: leased.job.job_id.clone(),
            lease_token: leased.lease_token.clone(),
            failure_code: failure_code.to_owned(),
            retryable: false,
        },
    )?;
    connection.execute(
        "UPDATE agent_runtime_plan_steps SET state='failed',failure_code=?1,result_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE job_id=?3",
        params![failure_code, now_utc(), leased.job.job_id],
    )?;
    connection.execute(
        "UPDATE agent_runtime_plans SET state='failed',failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE plan_id=(SELECT plan_id FROM agent_runtime_plan_steps WHERE job_id=?3) AND state IN ('queued','running')",
        params![failure_code, now_utc(), leased.job.job_id],
    )?;
    Ok(())
}

fn cancel_remaining_jobs(
    state: &AppState,
    plan_id: &str,
    current_step_id: &str,
    reason: &str,
) -> Result<()> {
    let jobs = {
        let connection = state.connection()?;
        let sequence: i64 = connection.query_row(
            "SELECT sequence_number FROM agent_runtime_plan_steps WHERE step_id=?1",
            params![current_step_id],
            |row| row.get(0),
        )?;
        let mut statement = connection.prepare(
            "SELECT j.job_id,j.connection_id FROM agent_runtime_plan_steps s JOIN wrapper_jobs j ON j.job_id=s.job_id WHERE s.plan_id=?1 AND s.sequence_number>?2 AND j.state IN ('queued','leased','running','waiting') ORDER BY s.sequence_number",
        )?;
        let rows = statement
            .query_map(params![plan_id, sequence], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (job_id, connection_id) in jobs {
        let connection = state.connection()?;
        let _ = wrapper_jobs::cancel_job(
            &connection,
            wrapper_jobs::CancelJobRequest {
                connection_id,
                job_id: job_id.clone(),
                actor_type: "system".to_owned(),
                actor_id: ORCHESTRATOR_ACTOR.to_owned(),
                confirmation: format!("CANCEL JOB {job_id}"),
                reason: reason.to_owned(),
            },
        );
    }
    Ok(())
}

fn process_automatic_compensations(state: &AppState) -> Result<()> {
    let checkpoint_ids = {
        let connection = state.connection()?;
        let mut statement = connection.prepare(
            "SELECT c.checkpoint_id FROM agent_supervised_action_checkpoints c JOIN agent_runtime_plans p ON p.plan_id=c.plan_id WHERE c.state='completed' AND c.compensation_mode='automatic' AND c.compensation_supported=1 AND c.compensation_state='available' AND p.state IN ('failed','cancelled','expired') ORDER BY c.completed_at_utc DESC LIMIT 50",
        )?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for checkpoint_id in checkpoint_ids {
        compensate_checkpoint(
            state,
            &checkpoint_id,
            ORCHESTRATOR_ACTOR,
            "automatic_plan_failure",
        )?;
    }
    Ok(())
}

fn rollback_checkpoint(
    state: &AppState,
    request: RollbackCheckpointRequest,
    actor: &str,
) -> Result<()> {
    let checkpoint_id = validate_uuid(&request.checkpoint_id, "checkpoint ID")?;
    ensure!(
        request.confirmation == format!("ROLLBACK ACTION {checkpoint_id}"),
        "action rollback confirmation is invalid"
    );
    let reason = bounded_text(&request.reason, 1, 500, "rollback reason")?;
    compensate_checkpoint(state, &checkpoint_id, actor, &reason)
}

fn compensate_checkpoint(
    state: &AppState,
    checkpoint_id: &str,
    actor: &str,
    reason: &str,
) -> Result<()> {
    let checkpoint_id = validate_uuid(checkpoint_id, "checkpoint ID")?;
    let actor = bounded_text(actor, 1, 160, "compensation actor")?;
    let reason = bounded_text(reason, 1, 500, "compensation reason")?;
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let existing_compensation_receipt: Option<String> = transaction
        .query_row(
            "SELECT compensation_receipt_id FROM agent_supervised_compensation_receipts WHERE checkpoint_id=?1",
            params![&checkpoint_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if existing_compensation_receipt.is_some() {
        transaction.commit()?;
        return Ok(());
    }
    let row: (String, String, String, String, String) = transaction.query_row(
        "SELECT c.tool_adapter,c.state,c.compensation_state,r.action_receipt_id,a.safe_result_json FROM agent_supervised_action_checkpoints c JOIN agent_supervised_action_receipts r ON r.checkpoint_id=c.checkpoint_id JOIN agent_action_receipts ar ON ar.receipt_id=r.action_receipt_id JOIN agent_action_attempts a ON a.attempt_id=ar.attempt_id WHERE c.checkpoint_id=?1",
        params![&checkpoint_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    )?;
    ensure!(
        row.1 == "completed",
        "only completed actions can be rolled back"
    );
    ensure!(
        row.2 == "available" && row.0 == "report.save",
        "this action has no available compensating adapter"
    );
    transaction.execute(
        "UPDATE agent_supervised_action_checkpoints SET compensation_state='running',updated_at_utc=?1 WHERE checkpoint_id=?2 AND compensation_state='available'",
        params![now_utc(), &checkpoint_id],
    )?;
    let safe_result: Value = serde_json::from_str(&row.4)?;
    let report_id = safe_result
        .get("report_id")
        .and_then(Value::as_str)
        .context("saved report identity is unavailable")?;
    let target_hash = hash_text(report_id);
    let deleted = transaction.execute(
        "DELETE FROM agent_reports WHERE report_id=?1",
        params![report_id],
    )?;
    ensure!(
        deleted <= 1,
        "compensation affected an unexpected number of reports"
    );
    let completed = now_utc();
    let compensation_receipt_id = Uuid::new_v4().to_string();
    let document = json!({
        "schema":"homeserver.supervised-compensation-receipt.v1",
        "compensation_receipt_id":compensation_receipt_id,
        "checkpoint_id":checkpoint_id,
        "action_receipt_id":row.3,
        "adapter_key":"report.delete",
        "outcome":"completed",
        "result_code": if deleted == 1 { "report_removed" } else { "report_already_absent" },
        "target_hash":target_hash,
        "actor":actor,
        "reason_hash":hash_text(&reason),
        "completed_at_utc":completed
    });
    let receipt_hash = hash_json(&document)?;
    transaction.execute(
        "INSERT INTO agent_supervised_compensation_receipts (compensation_receipt_id,checkpoint_id,action_receipt_id,adapter_key,outcome,result_code,target_hash,receipt_hash,completed_at_utc,created_at_utc) VALUES (?1,?2,?3,'report.delete','completed',?4,?5,?6,?7,?7)",
        params![compensation_receipt_id,checkpoint_id,row.3,if deleted == 1 {"report_removed"} else {"report_already_absent"},target_hash,receipt_hash,completed],
    )?;
    transaction.execute(
        "UPDATE agent_supervised_action_checkpoints SET compensation_state='completed',updated_at_utc=?1 WHERE checkpoint_id=?2",
        params![completed, checkpoint_id],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            checkpoint_id: Some(&checkpoint_id),
            plan_id: None,
            step_id: None,
            job_id: None,
            proposal_id: None,
            event_type: "agent.supervised_action_compensated",
            outcome: "success",
            actor_type: if actor == ORCHESTRATOR_ACTOR {
                "system"
            } else {
                "local_user"
            },
            actor_id: &actor,
            detail_code: if deleted == 1 {
                "report_removed"
            } else {
                "report_already_absent"
            },
            metadata: json!({"target_hash":target_hash,"receipt_hash":receipt_hash,"reason_hash":hash_text(&reason)}),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn reconcile(connection: &Connection) -> Result<()> {
    let broken_active: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_supervised_action_checkpoints c LEFT JOIN agent_action_proposals p ON p.proposal_id=c.proposal_id LEFT JOIN agent_runtime_plan_steps s ON s.step_id=c.step_id WHERE c.state IN ('awaiting_approval','approved','executing') AND (p.proposal_id IS NULL OR s.step_id IS NULL)",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        broken_active == 0,
        "active supervised checkpoint lost its proposal or runtime step"
    );
    Ok(())
}

fn refresh_plan_state_tx(transaction: &Transaction<'_>, plan_id: &str) -> Result<()> {
    let (total, completed, failed, cancelled): (i64, i64, i64, i64) = transaction.query_row(
        "SELECT COUNT(*),SUM(CASE WHEN state='completed' THEN 1 ELSE 0 END),SUM(CASE WHEN state='failed' THEN 1 ELSE 0 END),SUM(CASE WHEN state IN ('cancelled','expired') THEN 1 ELSE 0 END) FROM agent_runtime_plan_steps WHERE plan_id=?1",
        params![plan_id],
        |row| Ok((row.get(0)?,row.get::<_, Option<i64>>(1)?.unwrap_or(0),row.get::<_, Option<i64>>(2)?.unwrap_or(0),row.get::<_, Option<i64>>(3)?.unwrap_or(0))),
    )?;
    let now = now_utc();
    if total > 0 && completed == total {
        transaction.execute(
            "UPDATE agent_runtime_plans SET state='completed',completed_step_count=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE plan_id=?3 AND state IN ('queued','running')",
            params![completed, now, plan_id],
        )?;
    } else if failed > 0 {
        transaction.execute(
            "UPDATE agent_runtime_plans SET state='failed',completed_step_count=?1,failure_code=COALESCE(failure_code,'supervised_step_failed'),completed_at_utc=COALESCE(completed_at_utc,?2),updated_at_utc=?2 WHERE plan_id=?3 AND state IN ('queued','running')",
            params![completed, now, plan_id],
        )?;
    } else if cancelled > 0 && completed + cancelled == total {
        transaction.execute(
            "UPDATE agent_runtime_plans SET state='cancelled',completed_step_count=?1,completed_at_utc=COALESCE(completed_at_utc,?2),updated_at_utc=?2 WHERE plan_id=?3 AND state IN ('queued','running')",
            params![completed, now, plan_id],
        )?;
    } else {
        transaction.execute(
            "UPDATE agent_runtime_plans SET completed_step_count=?1,updated_at_utc=?2 WHERE plan_id=?3",
            params![completed, now, plan_id],
        )?;
    }
    Ok(())
}

fn read_checkpoint(connection: &Connection, checkpoint_id: &str) -> Result<CheckpointRecord> {
    connection
        .query_row(
            "SELECT checkpoint_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,proposal_id,approval_id,policy_id,action_type,risk_class,tool_adapter,runtime_plan_hash,proposal_plan_hash,payload_hash,agent_revision,assignment_id,assignment_revision,policy_revision,grant_id,grant_revision,connection_authority_revision,expires_at_utc FROM agent_supervised_action_checkpoints WHERE checkpoint_id=?1",
            params![checkpoint_id],
            |row| {
                Ok(CheckpointRecord {
                    checkpoint_id: row.get(0)?,
                    plan_id: row.get(1)?,
                    step_id: row.get(2)?,
                    job_id: row.get(3)?,
                    agent_id: row.get(4)?,
                    wrapper_id: row.get(5)?,
                    connection_id: row.get(6)?,
                    proposal_id: row.get(7)?,
                    approval_id: row.get(8)?,
                    policy_id: row.get(9)?,
                    action_type: row.get(10)?,
                    risk_class: row.get(11)?,
                    tool_adapter: row.get(12)?,
                    runtime_plan_hash: row.get(13)?,
                    proposal_plan_hash: row.get(14)?,
                    payload_hash: row.get(15)?,
                    agent_revision: row.get::<_, i64>(16)?.max(0) as u64,
                    assignment_id: row.get(17)?,
                    assignment_revision: row.get::<_, i64>(18)?.max(0) as u64,
                    policy_revision: row.get::<_, i64>(19)?.max(0) as u64,
                    grant_id: row.get(20)?,
                    grant_revision: row.get::<_, i64>(21)?.max(0) as u64,
                    connection_authority_revision: row.get::<_, i64>(22)?.max(0) as u64,
                    expires_at_utc: row.get(23)?,
                })
            },
        )
        .context("supervised checkpoint was not found")
}

fn snapshot(state: &AppState) -> Result<SupervisedOrchestrationSnapshot> {
    let connection = state.connection()?;
    reconcile(&connection)?;
    let (worker_id, state_value): (String, String) = connection.query_row(
        "SELECT worker_id,state FROM agent_supervised_action_state WHERE singleton_id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(SupervisedOrchestrationSnapshot {
        schema: "homeserver.supervised-action-orchestration.v1".to_owned(),
        state: state_value,
        worker_id,
        checkpoints: read_checkpoints(&connection)?,
        receipts: read_receipts(&connection)?,
        compensation_receipts: read_compensation_receipts(&connection)?,
        private_payloads_exposed: false,
        private_results_exposed: false,
        approval_hashes_revalidated: true,
        approval_consumed_once: true,
        phase16e_egress_required: true,
        sensitive_runtime_bypass_allowed: false,
    })
}

fn read_checkpoints(connection: &Connection) -> Result<Vec<SupervisedCheckpointSummary>> {
    let mut statement = connection.prepare(
        "SELECT c.checkpoint_id,c.plan_id,c.step_id,c.sequence_number,c.job_id,c.agent_id,c.wrapper_id,c.connection_id,c.proposal_id,c.approval_id,c.policy_id,c.action_type,c.risk_class,c.tool_adapter,p.title,p.rationale,p.safe_summary_json,c.state,a.state,p.state,c.compensation_mode,c.compensation_supported,c.compensation_state,c.runtime_plan_hash,c.proposal_plan_hash,c.payload_hash,c.failure_code,c.expires_at_utc,c.created_at_utc,c.updated_at_utc,c.completed_at_utc FROM agent_supervised_action_checkpoints c JOIN agent_action_proposals p ON p.proposal_id=c.proposal_id LEFT JOIN agent_action_approvals a ON a.proposal_id=c.proposal_id ORDER BY c.created_at_utc DESC,c.checkpoint_id DESC LIMIT ?1",
    )?;
    let values = statement
        .query_map(params![MAX_CHECKPOINTS], |row| {
            let safe_summary_text: String = row.get(16)?;
            Ok(SupervisedCheckpointSummary {
                checkpoint_id: row.get(0)?,
                plan_id: row.get(1)?,
                step_id: row.get(2)?,
                sequence_number: row.get::<_, i64>(3)?.max(0) as u32,
                job_id: row.get(4)?,
                agent_id: row.get(5)?,
                wrapper_id: row.get(6)?,
                connection_id: row.get(7)?,
                proposal_id: row.get(8)?,
                approval_id: row.get(9)?,
                policy_id: row.get(10)?,
                action_type: row.get(11)?,
                risk_class: row.get(12)?,
                tool_adapter: row.get(13)?,
                title: row.get(14)?,
                rationale: row.get(15)?,
                safe_summary: serde_json::from_str(&safe_summary_text)
                    .unwrap_or_else(|_| json!({})),
                state: row.get(17)?,
                approval_state: row.get(18)?,
                proposal_state: row.get(19)?,
                compensation_mode: row.get(20)?,
                compensation_supported: row.get::<_, i64>(21)? != 0,
                compensation_state: row.get(22)?,
                runtime_plan_hash: row.get(23)?,
                proposal_plan_hash: row.get(24)?,
                payload_hash: row.get(25)?,
                failure_code: row.get(26)?,
                expires_at_utc: row.get(27)?,
                created_at_utc: row.get(28)?,
                updated_at_utc: row.get(29)?,
                completed_at_utc: row.get(30)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(values)
}

fn read_receipts(connection: &Connection) -> Result<Vec<SupervisedReceiptSummary>> {
    let mut statement = connection.prepare(
        "SELECT receipt_id,checkpoint_id,plan_id,step_id,job_id,proposal_id,approval_id,action_receipt_id,action_receipt_hash,wrapper_job_receipt_id,wrapper_job_receipt_hash,runtime_receipt_id,runtime_receipt_hash,runtime_plan_hash,proposal_plan_hash,payload_hash,outcome,result_code,safe_result_hash,phase16e_detail_code,receipt_hash,completed_at_utc FROM agent_supervised_action_receipts ORDER BY completed_at_utc DESC,receipt_id DESC LIMIT 500",
    )?;
    let values = statement
        .query_map([], |row| {
            Ok(SupervisedReceiptSummary {
                receipt_id: row.get(0)?,
                checkpoint_id: row.get(1)?,
                plan_id: row.get(2)?,
                step_id: row.get(3)?,
                job_id: row.get(4)?,
                proposal_id: row.get(5)?,
                approval_id: row.get(6)?,
                action_receipt_id: row.get(7)?,
                action_receipt_hash: row.get(8)?,
                wrapper_job_receipt_id: row.get(9)?,
                wrapper_job_receipt_hash: row.get(10)?,
                runtime_receipt_id: row.get(11)?,
                runtime_receipt_hash: row.get(12)?,
                runtime_plan_hash: row.get(13)?,
                proposal_plan_hash: row.get(14)?,
                payload_hash: row.get(15)?,
                outcome: row.get(16)?,
                result_code: row.get(17)?,
                safe_result_hash: row.get(18)?,
                phase16e_detail_code: row.get(19)?,
                receipt_hash: row.get(20)?,
                completed_at_utc: row.get(21)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(values)
}

fn read_compensation_receipts(connection: &Connection) -> Result<Vec<CompensationReceiptSummary>> {
    let mut statement = connection.prepare(
        "SELECT compensation_receipt_id,checkpoint_id,action_receipt_id,adapter_key,outcome,result_code,target_hash,receipt_hash,completed_at_utc FROM agent_supervised_compensation_receipts ORDER BY completed_at_utc DESC,compensation_receipt_id DESC LIMIT 500",
    )?;
    let values = statement
        .query_map([], |row| {
            Ok(CompensationReceiptSummary {
                compensation_receipt_id: row.get(0)?,
                checkpoint_id: row.get(1)?,
                action_receipt_id: row.get(2)?,
                adapter_key: row.get(3)?,
                outcome: row.get(4)?,
                result_code: row.get(5)?,
                target_hash: row.get(6)?,
                receipt_hash: row.get(7)?,
                completed_at_utc: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(values)
}

struct EventEvidence<'a> {
    checkpoint_id: Option<&'a str>,
    plan_id: Option<&'a str>,
    step_id: Option<&'a str>,
    job_id: Option<&'a str>,
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
    let created_at = now_utc();
    let document = json!({
        "event_id":event_id,
        "checkpoint_id":evidence.checkpoint_id,
        "plan_id":evidence.plan_id,
        "step_id":evidence.step_id,
        "job_id":evidence.job_id,
        "proposal_id":evidence.proposal_id,
        "event_type":evidence.event_type,
        "outcome":evidence.outcome,
        "actor_type":evidence.actor_type,
        "actor_id":evidence.actor_id,
        "detail_code":evidence.detail_code,
        "metadata":evidence.metadata,
        "created_at_utc":created_at
    });
    let event_hash = hash_json(&document)?;
    transaction.execute(
        "INSERT INTO agent_supervised_action_events (event_id,checkpoint_id,plan_id,step_id,job_id,proposal_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![event_id,evidence.checkpoint_id,evidence.plan_id,evidence.step_id,evidence.job_id,evidence.proposal_id,evidence.event_type,evidence.outcome,evidence.actor_type,evidence.actor_id,evidence.detail_code,serde_json::to_string(&evidence.metadata)?,event_hash,created_at],
    )?;
    Ok(())
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

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    Uuid::parse_str(value).with_context(|| format!("{label} is invalid"))?;
    Ok(value.to_owned())
}

fn now_utc() -> String {
    timestamp(Utc::now())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_utc(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{label} is invalid"))
        .map(|value| value.with_timezone(&Utc))
}

fn hash_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn hash_json(value: &Value) -> Result<String> {
    Ok(hash_text(&serde_json::to_string(value)?))
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
