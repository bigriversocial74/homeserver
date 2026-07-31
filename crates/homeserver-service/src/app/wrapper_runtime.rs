use super::{wrapper_agents, wrapper_jobs};
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
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc, time::Duration as StdDuration};
use tokio::sync::watch;
use tracing::{error, warn};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0025_authorized_agent_tool_runtime.sql");
const MIGRATION_KEY: &str = "0025_authorized_agent_tool_runtime";
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;
const MAX_EVENTS: i64 = 50_000;
const MAX_RECEIPTS: i64 = 50_000;
const MAX_PLANS: i64 = 250;
const MAX_STEPS: usize = 32;
const RUNTIME_WORKER_NAME: &str = "HomeServer Authorized Agent Tool Runtime";
const RUNTIME_JOB_TYPES: &[&str] = &[
    "runtime.wrapper_status",
    "runtime.receipt_read",
    "runtime.audit_record",
    "runtime.result_compose",
];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolSummary {
    pub tool_key: String,
    pub adapter_key: String,
    pub version: String,
    pub description: String,
    pub risk_class: String,
    pub approval_requirement: String,
    pub allowed_job_types: Vec<String>,
    pub max_execution_seconds: u32,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePlanSummary {
    pub plan_id: String,
    pub agent_id: String,
    pub requested_by_user_id: String,
    pub title: String,
    pub objective: String,
    pub state: String,
    pub step_count: u32,
    pub completed_step_count: u32,
    pub correlation_id: String,
    pub plan_hash: String,
    pub expires_at_utc: String,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeStepSummary {
    pub step_id: String,
    pub plan_id: String,
    pub sequence_number: u32,
    pub job_id: String,
    pub tool_key: String,
    pub adapter_key: String,
    pub action_type: String,
    pub state: String,
    pub idempotency_key: String,
    pub argument_hash: String,
    pub private_result_hash: Option<String>,
    pub safe_result_hash: Option<String>,
    pub result_code: Option<String>,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub started_at_utc: Option<String>,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeReceiptSummary {
    pub receipt_id: String,
    pub plan_id: String,
    pub step_id: String,
    pub job_id: String,
    pub agent_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub tool_key: String,
    pub adapter_key: String,
    pub outcome: String,
    pub result_code: String,
    pub job_receipt_id: Option<String>,
    pub job_receipt_hash: Option<String>,
    pub safe_result_hash: Option<String>,
    pub runtime_receipt_hash: String,
    pub completed_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSnapshot {
    pub schema: String,
    pub runtime_state: String,
    pub worker_id: String,
    pub tools: Vec<ToolSummary>,
    pub plans: Vec<RuntimePlanSummary>,
    pub steps: Vec<RuntimeStepSummary>,
    pub receipts: Vec<RuntimeReceiptSummary>,
    pub private_inputs_exposed: bool,
    pub private_results_exposed: bool,
    pub direct_tool_bypass_allowed: bool,
    pub phase16e_egress_required: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimePlanStepRequest {
    pub tool_key: String,
    pub action_type: String,
    pub job: wrapper_jobs::SubmitJobRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRuntimePlanRequest {
    pub agent_id: String,
    pub requested_by_user_id: String,
    pub title: String,
    pub objective: String,
    pub steps: Vec<RuntimePlanStepRequest>,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuntimePlanReferenceRequest {
    pub plan_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
struct ToolRecord {
    tool_key: String,
    adapter_key: String,
    risk_class: String,
    approval_requirement: String,
    allowed_job_types: Vec<String>,
    max_execution_seconds: u32,
    state: String,
}

#[derive(Debug, Clone)]
struct ExecutionContext {
    plan_id: String,
    step_id: String,
    agent_id: String,
    wrapper_id: String,
    connection_id: String,
    tool: ToolRecord,
    action_type: String,
    attempt_id: String,
    attempt_number: u32,
}

#[derive(Debug)]
struct AdapterExecution {
    private_result: Value,
    private_provenance: Value,
    source_count: u32,
    source_types: Vec<String>,
    result_code: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    reconcile(connection)?;
    ensure_runtime_worker(connection)?;
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
        "authorized agent tool runtime migration is not registered exactly once"
    );
    for table in [
        "agent_tool_catalog",
        "agent_runtime_plans",
        "agent_runtime_plan_steps",
        "agent_runtime_attempts",
        "agent_runtime_receipts",
        "agent_runtime_events",
        "agent_runtime_audit_records",
        "agent_runtime_state",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let invalid_worker: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runtime_state s LEFT JOIN wrapper_job_workers w ON w.worker_id=s.worker_id WHERE s.singleton_id=1 AND (w.worker_id IS NULL OR w.worker_kind<>'tool')",
        [],
        |row| row.get(0),
    )?;
    ensure!(invalid_worker == 0, "agent runtime worker registration is invalid");
    let bypass: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runtime_plan_steps s JOIN wrapper_jobs j ON j.job_id=s.job_id LEFT JOIN agent_job_bindings b ON b.job_id=j.job_id WHERE j.submitted_by_type<>'agent' OR b.job_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(bypass == 0, "agent runtime step bypassed agent-bound job authority");
    let incomplete_receipts: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runtime_plan_steps s LEFT JOIN agent_runtime_receipts r ON r.step_id=s.step_id WHERE s.state='completed' AND r.step_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        incomplete_receipts == 0,
        "completed agent runtime steps are missing immutable receipts"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    reconcile(connection)?;
    connection.execute(
        "DELETE FROM agent_runtime_events WHERE created_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM agent_runtime_events WHERE event_id NOT IN (SELECT event_id FROM agent_runtime_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1)",
        params![MAX_EVENTS],
    )?;
    let receipt_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runtime_receipts",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        receipt_count <= MAX_RECEIPTS,
        "agent runtime receipt retention requires archival"
    );
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/agent-runtime", get(snapshot_handler))
        .route("/v1/agent-runtime/plans/create", post(create_plan_handler))
        .route("/v1/agent-runtime/plans/cancel", post(cancel_plan_handler))
        .route("/v1/agent-runtime/run-once", post(run_once_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now() + StdDuration::from_secs(3);
    let mut interval = tokio::time::interval_at(start, StdDuration::from_secs(2));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let cycle_state = state.clone();
                match tokio::task::spawn_blocking(move || process_cycle(&cycle_state)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => warn!(?error, "authorized agent runtime cycle failed"),
                    Err(error) => error!(?error, "authorized agent runtime task failed"),
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

async fn snapshot_handler(State(state): State<Arc<AppState>>) -> ApiResult<RuntimeSnapshot> {
    run_blocking(move || snapshot(&state), "agent_runtime_snapshot_failed").await
}

async fn create_plan_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateRuntimePlanRequest>,
) -> ApiResult<RuntimeSnapshot> {
    run_blocking(
        move || {
            create_plan(&state, request)?;
            snapshot(&state)
        },
        "agent_runtime_plan_create_failed",
    )
    .await
}

async fn cancel_plan_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RuntimePlanReferenceRequest>,
) -> ApiResult<RuntimeSnapshot> {
    run_blocking(
        move || {
            cancel_plan(&state, request)?;
            snapshot(&state)
        },
        "agent_runtime_plan_cancel_failed",
    )
    .await
}

async fn run_once_handler(State(state): State<Arc<AppState>>) -> ApiResult<RuntimeSnapshot> {
    run_blocking(
        move || {
            process_cycle(&state)?;
            snapshot(&state)
        },
        "agent_runtime_cycle_failed",
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
        .map_err(|error| api_error(code, anyhow::anyhow!("agent runtime task failed: {error}")))?
        .map(Json)
        .map_err(|error| api_error(code, error))
}

fn ensure_runtime_worker(connection: &Connection) -> Result<String> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT s.worker_id FROM agent_runtime_state s JOIN wrapper_job_workers w ON w.worker_id=s.worker_id WHERE s.singleton_id=1 AND s.state='active' AND w.state='active'",
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
            worker_kind: "tool".to_owned(),
            display_name: RUNTIME_WORKER_NAME.to_owned(),
            allowed_job_types: RUNTIME_JOB_TYPES.iter().map(|value| (*value).to_owned()).collect(),
            max_concurrent_jobs: 1,
        },
    )?;
    let now = now_utc();
    connection.execute(
        "INSERT INTO agent_runtime_state (singleton_id,worker_id,runtime_revision,state,created_at_utc,updated_at_utc) VALUES (1,?1,1,'active',?2,?2) ON CONFLICT(singleton_id) DO UPDATE SET worker_id=excluded.worker_id,runtime_revision=agent_runtime_state.runtime_revision+1,state='active',last_error_code=NULL,updated_at_utc=excluded.updated_at_utc",
        params![worker.worker_id, now],
    )?;
    Ok(worker.worker_id)
}

fn create_plan(state: &AppState, request: CreateRuntimePlanRequest) -> Result<String> {
    ensure!(
        !request.steps.is_empty() && request.steps.len() <= MAX_STEPS,
        "runtime plan must contain between one and 32 steps"
    );
    ensure!(
        (1..=10_080).contains(&request.expires_minutes),
        "runtime plan expiration must be between one minute and seven days"
    );
    let agent_id = validate_uuid(&request.agent_id, "agent ID")?;
    let actor = bounded_text(
        &request.requested_by_user_id,
        1,
        160,
        "requested-by user ID",
    )?;
    let title = bounded_text(&request.title, 1, 180, "plan title")?;
    let objective = bounded_text(&request.objective, 1, 4000, "plan objective")?;
    {
        let connection = state.connection()?;
        let active: i64 = connection.query_row(
            "SELECT COUNT(*) FROM homeserver_agents WHERE agent_id=?1 AND state='active' AND expires_at_utc>?2",
            params![agent_id, now_utc()],
            |row| row.get(0),
        )?;
        ensure!(active == 1, "runtime plan agent is not active");
    }
    let plan_id = Uuid::new_v4().to_string();
    let correlation_id = Uuid::new_v4().to_string();
    let expires_at = timestamp(Utc::now() + Duration::minutes(i64::from(request.expires_minutes)));
    let plan_document = json!({
        "schema": "homeserver.agent-runtime-plan.v1",
        "plan_id": plan_id,
        "agent_id": agent_id,
        "requested_by_user_id": actor,
        "title": title,
        "objective": objective,
        "correlation_id": correlation_id,
        "expires_at_utc": expires_at,
        "steps": request.steps.iter().map(|step| json!({
            "tool_key": step.tool_key,
            "action_type": step.action_type,
            "job": step.job
        })).collect::<Vec<_>>()
    });
    let plan_hash = hash_json(&plan_document)?;
    let now = now_utc();
    {
        let connection = state.connection()?;
        connection.execute(
            "INSERT INTO agent_runtime_plans (plan_id,agent_id,requested_by_user_id,title,objective,state,step_count,completed_step_count,correlation_id,plan_hash,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,'queued',?6,0,?7,?8,?9,?10,?10)",
            params![
                plan_id,
                agent_id,
                actor,
                title,
                objective,
                request.steps.len() as i64,
                correlation_id,
                plan_hash,
                expires_at,
                now
            ],
        )?;
    }

    let mut submitted = Vec::<(String, String)>::new();
    let mut previous_job_id: Option<String> = None;
    for (index, mut step) in request.steps.into_iter().enumerate() {
        let tool_key = validate_symbol(&step.tool_key, 120, "tool key")?;
        let action_type = validate_symbol(&step.action_type, 120, "action type")?;
        ensure!(
            action_type == tool_key,
            "runtime action type must exactly match the tool key"
        );
        let tool = {
            let connection = state.connection()?;
            read_tool(&connection, &tool_key)?
        };
        ensure!(tool.state == "active", "runtime tool is not active");
        ensure!(
            tool.allowed_job_types
                .iter()
                .any(|value| value == &step.job.job_type),
            "runtime tool does not accept the requested job type"
        );
        step.job.submitted_by_type = "agent".to_owned();
        step.job.submitted_by_id = agent_id.clone();
        step.job.correlation_id = Some(correlation_id.clone());
        step.job.causation_id = previous_job_id.clone();
        step.job.expires_minutes = step.job.expires_minutes.min(request.expires_minutes);
        let connection_id = step.job.connection_id.clone();
        let response = match wrapper_jobs::submit_job(state, step.job) {
            Ok(response) => response,
            Err(error) => {
                mark_plan_build_failed(state, &plan_id, &agent_id, &submitted, &error)?;
                return Err(error).context("runtime plan step submission failed");
            }
        };
        let step_id = Uuid::new_v4().to_string();
        {
            let connection = state.connection()?;
            connection.execute(
                "INSERT INTO agent_runtime_plan_steps (step_id,plan_id,sequence_number,job_id,tool_key,adapter_key,action_type,state,idempotency_key,argument_hash,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'queued',?8,?9,?10,?10)",
                params![
                    step_id,
                    plan_id,
                    (index + 1) as i64,
                    response.job_id,
                    tool.tool_key,
                    tool.adapter_key,
                    action_type,
                    response.idempotency_key,
                    response.payload_hash,
                    now_utc()
                ],
            )?;
        }
        submitted.push((response.job_id.clone(), connection_id));
        previous_job_id = Some(response.job_id);
    }
    let connection = state.connection()?;
    record_event(
        &connection,
        EventEvidence {
            plan_id: Some(&plan_id),
            step_id: None,
            job_id: None,
            agent_id: Some(&agent_id),
            event_type: "agent.runtime_plan_queued",
            outcome: "success",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: "authorized_steps_queued",
            metadata: json!({"step_count": submitted.len(), "plan_hash": plan_hash}),
        },
    )?;
    Ok(plan_id)
}

fn mark_plan_build_failed(
    state: &AppState,
    plan_id: &str,
    agent_id: &str,
    submitted: &[(String, String)],
    error: &anyhow::Error,
) -> Result<()> {
    for (job_id, connection_id) in submitted {
        let connection = state.connection()?;
        let _ = wrapper_jobs::cancel_job(
            &connection,
            wrapper_jobs::CancelJobRequest {
                connection_id: connection_id.clone(),
                job_id: job_id.clone(),
                actor_type: "system".to_owned(),
                actor_id: "agent_runtime".to_owned(),
                confirmation: format!("CANCEL JOB {job_id}"),
                reason: "runtime plan construction failed".to_owned(),
            },
        );
    }
    let connection = state.connection()?;
    connection.execute(
        "UPDATE agent_runtime_plans SET state='failed',failure_code='plan_construction_failed',completed_at_utc=?1,updated_at_utc=?1 WHERE plan_id=?2",
        params![now_utc(), plan_id],
    )?;
    record_event(
        &connection,
        EventEvidence {
            plan_id: Some(plan_id),
            step_id: None,
            job_id: None,
            agent_id: Some(agent_id),
            event_type: "agent.runtime_plan_failed",
            outcome: "error",
            actor_type: "system",
            actor_id: "agent_runtime",
            detail_code: "plan_construction_failed",
            metadata: json!({"error_hash": hash_text(&error.to_string())}),
        },
    )
}

fn cancel_plan(state: &AppState, request: RuntimePlanReferenceRequest) -> Result<()> {
    let plan_id = validate_uuid(&request.plan_id, "plan ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 500, "cancellation reason")?;
    ensure!(
        request.confirmation == format!("CANCEL PLAN {plan_id}"),
        "runtime plan cancellation confirmation is invalid"
    );
    let (agent_id, state_value): (String, String) = {
        let connection = state.connection()?;
        connection.query_row(
            "SELECT agent_id,state FROM agent_runtime_plans WHERE plan_id=?1",
            params![plan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    };
    ensure!(
        matches!(state_value.as_str(), "queued" | "running"),
        "runtime plan is not cancellable"
    );
    cancel_remaining_jobs(state, &plan_id, "runtime plan cancelled")?;
    let connection = state.connection()?;
    let now = now_utc();
    connection.execute(
        "UPDATE agent_runtime_plan_steps SET state='cancelled',failure_code='plan_cancelled',completed_at_utc=?1,updated_at_utc=?1 WHERE plan_id=?2 AND state IN ('queued','leased','running')",
        params![now, plan_id],
    )?;
    connection.execute(
        "UPDATE agent_runtime_plans SET state='cancelled',failure_code='cancelled_by_authority',completed_at_utc=?1,updated_at_utc=?1 WHERE plan_id=?2",
        params![now, plan_id],
    )?;
    record_event(
        &connection,
        EventEvidence {
            plan_id: Some(&plan_id),
            step_id: None,
            job_id: None,
            agent_id: Some(&agent_id),
            event_type: "agent.runtime_plan_cancelled",
            outcome: "warning",
            actor_type: "local_user",
            actor_id: &actor,
            detail_code: "cancelled_by_authority",
            metadata: json!({"reason": reason}),
        },
    )
}

fn process_cycle(state: &AppState) -> Result<usize> {
    let worker_id = {
        let connection = state.connection()?;
        ensure_runtime_worker(&connection)?
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
    let mut completed = 0_usize;
    let mut last_error: Option<String> = None;
    for leased in claimed.jobs {
        match process_leased_job(state, &worker_id, leased) {
            Ok(()) => completed += 1,
            Err(error) => {
                last_error = Some("runtime_job_failed".to_owned());
                warn!(?error, "authorized agent runtime job failed");
            }
        }
    }
    let connection = state.connection()?;
    connection.execute(
        "UPDATE agent_runtime_state SET last_cycle_at_utc=?1,last_error_code=?2,updated_at_utc=?1 WHERE singleton_id=1",
        params![now_utc(), last_error],
    )?;
    Ok(completed)
}

fn process_leased_job(
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
    let context = match prepare_execution(state, worker_id, &leased.job) {
        Ok(context) => context,
        Err(error) => {
            fail_runtime_job(
                state,
                worker_id,
                &leased,
                "runtime_authority_denied",
                &error,
            )?;
            return Err(error);
        }
    };
    let execution = match execute_adapter(state, &context, &leased.private_input) {
        Ok(execution) => execution,
        Err(error) => {
            fail_runtime_job(
                state,
                worker_id,
                &leased,
                "runtime_adapter_failed",
                &error,
            )?;
            return Err(error);
        }
    };
    let private_result_hash = hash_json(&execution.private_result)?;
    let receipt = {
        let connection = state.connection()?;
        wrapper_jobs::complete_job(
            &connection,
            wrapper_jobs::CompleteJobRequest {
                worker_id: worker_id.to_owned(),
                job_id: leased.job.job_id.clone(),
                lease_token: leased.lease_token,
                private_result: execution.private_result,
                private_provenance: execution.private_provenance,
                source_count: execution.source_count,
                source_types: execution.source_types,
                evidence_hash: None,
                actual_token_count: Some(0),
                result_code: execution.result_code,
            },
        )?
    };
    finalize_success(state, &context, &leased.job, &private_result_hash, &receipt)
}

fn prepare_execution(
    state: &AppState,
    worker_id: &str,
    job: &wrapper_jobs::JobSummary,
) -> Result<ExecutionContext> {
    ensure!(job.submitted_by_type == "agent", "runtime job is not agent-submitted");
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    ensure!(
        wrapper_agents::agent_job_authority_is_current_tx(&transaction, &job.job_id)?,
        "runtime job agent authority changed"
    );
    let row: (String, String, String, String, String, String, i64, String, String, String, String, String, String, String, String) = transaction.query_row(
        "SELECT s.plan_id,s.step_id,p.agent_id,j.wrapper_id,j.connection_id,s.tool_key,s.sequence_number,s.adapter_key,s.action_type,a.tool_restrictions_json,e.policy_id,e.risk_class,e.approval_mode,e.tool_adapter,e.state FROM agent_runtime_plan_steps s JOIN agent_runtime_plans p ON p.plan_id=s.plan_id JOIN wrapper_jobs j ON j.job_id=s.job_id JOIN agent_job_bindings b ON b.job_id=j.job_id JOIN homeserver_agents a ON a.agent_id=b.agent_id JOIN agent_execution_policies e ON e.agent_id=a.agent_id AND e.action_type=s.action_type WHERE s.job_id=?1 AND s.state IN ('queued','leased') AND p.state IN ('queued','running') AND e.state='active' AND e.not_before_utc<=?2 AND e.expires_at_utc>?2",
        params![job.job_id, now_utc()],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?,row.get(12)?,row.get(13)?,row.get(14)?)),
    )?;
    ensure!(row.2 == job.submitted_by_id, "runtime plan agent does not match the job submitter");
    let tool = read_tool_tx(&transaction, &row.5)?;
    ensure!(tool.state == "active", "runtime tool is not active");
    ensure!(tool.adapter_key == row.7, "runtime step adapter changed");
    ensure!(row.13 == tool.adapter_key, "agent policy adapter does not match the runtime tool");
    ensure!(row.11 == tool.risk_class, "agent policy risk class does not match the runtime tool");
    ensure!(row.14 == "active", "agent execution policy is not active");
    ensure!(
        tool.allowed_job_types.iter().any(|value| value == &job.job_type),
        "runtime tool does not allow this job type"
    );
    enforce_tool_restrictions(&row.9, &tool.adapter_key)?;
    enforce_autonomy_and_approval(
        &transaction,
        job,
        &row.2,
        &row.10,
        &row.11,
        &row.12,
        &tool.approval_requirement,
    )?;
    let attempt_number: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(attempt_number),0)+1 FROM agent_runtime_attempts WHERE step_id=?1",
        params![row.1],
        |result| result.get(0),
    )?;
    let attempt_id = Uuid::new_v4().to_string();
    let now = now_utc();
    transaction.execute(
        "UPDATE agent_runtime_plans SET state='running',updated_at_utc=?1 WHERE plan_id=?2 AND state='queued'",
        params![now, row.0],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plan_steps SET state='running',started_at_utc=COALESCE(started_at_utc,?1),updated_at_utc=?1 WHERE step_id=?2",
        params![now, row.1],
    )?;
    transaction.execute(
        "INSERT INTO agent_runtime_attempts (attempt_id,plan_id,step_id,job_id,worker_id,attempt_number,state,started_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'running',?7)",
        params![attempt_id, row.0, row.1, job.job_id, worker_id, attempt_number, now],
    )?;
    record_event_tx(
        &transaction,
        EventEvidence {
            plan_id: Some(&row.0),
            step_id: Some(&row.1),
            job_id: Some(&job.job_id),
            agent_id: Some(&row.2),
            event_type: "agent.runtime_step_started",
            outcome: "success",
            actor_type: "worker",
            actor_id: worker_id,
            detail_code: "authority_revalidated",
            metadata: json!({
                "tool_key": tool.tool_key,
                "adapter_key": tool.adapter_key,
                "policy_id": row.10,
                "sequence_number": row.6,
                "private_input_exposed": false
            }),
        },
    )?;
    transaction.commit()?;
    Ok(ExecutionContext {
        plan_id: row.0,
        step_id: row.1,
        agent_id: row.2,
        wrapper_id: row.3,
        connection_id: row.4,
        tool,
        action_type: row.8,
        attempt_id,
        attempt_number: attempt_number.max(1) as u32,
    })
}

fn enforce_autonomy_and_approval(
    transaction: &Transaction<'_>,
    job: &wrapper_jobs::JobSummary,
    agent_id: &str,
    policy_id: &str,
    risk_class: &str,
    approval_mode: &str,
    approval_requirement: &str,
) -> Result<()> {
    let autonomy: i64 = transaction.query_row(
        "SELECT autonomy_level FROM homeserver_agents WHERE agent_id=?1",
        params![agent_id],
        |row| row.get(0),
    )?;
    let required_autonomy = match risk_class {
        "read_only" => 1,
        "reversible" => 2,
        "external_side_effect" => 3,
        "high_risk" => 4,
        _ => bail!("runtime risk class is invalid"),
    };
    ensure!(
        autonomy >= required_autonomy,
        "agent autonomy level is below the runtime tool risk class"
    );
    let requires_proposal = approval_requirement == "proposal"
        || matches!(risk_class, "external_side_effect" | "high_risk")
        || approval_mode != "none";
    if !requires_proposal {
        ensure!(
            approval_requirement == "none" || approval_requirement == "policy",
            "runtime approval requirement is invalid"
        );
        return Ok(());
    }
    let plan_hash = job
        .plan_hash
        .as_deref()
        .context("approved runtime tool use is missing a plan hash")?;
    let approval_id = job
        .approval_id
        .as_deref()
        .context("approved runtime tool use is missing an approval ID")?;
    let approved: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_action_proposals p JOIN agent_action_approvals a ON a.proposal_id=p.proposal_id WHERE p.job_id=?1 AND p.agent_id=?2 AND p.policy_id=?3 AND p.plan_hash=?4 AND p.state='approved' AND a.approval_id=?5 AND a.plan_hash=?4 AND a.state='approved' AND a.expires_at_utc>?6",
        params![job.job_id, agent_id, policy_id, plan_hash, approval_id, now_utc()],
        |row| row.get(0),
    )?;
    ensure!(approved == 1, "runtime tool approval is missing, stale, or mismatched");
    Ok(())
}

fn enforce_tool_restrictions(restrictions_json: &str, adapter_key: &str) -> Result<()> {
    let restrictions = serde_json::from_str::<Value>(restrictions_json).unwrap_or_else(|_| json!({}));
    let denied = string_array(restrictions.get("denied_adapters"));
    ensure!(
        !denied.iter().any(|value| value == adapter_key),
        "runtime adapter is denied by the agent definition"
    );
    let allowed = string_array(restrictions.get("allowed_adapters"));
    if !allowed.is_empty() {
        ensure!(
            allowed.iter().any(|value| value == adapter_key),
            "runtime adapter is not allowed by the agent definition"
        );
    }
    Ok(())
}

fn execute_adapter(
    state: &AppState,
    context: &ExecutionContext,
    private_input: &Value,
) -> Result<AdapterExecution> {
    match context.tool.adapter_key.as_str() {
        "wrapper.status.read" => {
            let connection = state.connection()?;
            let value: Value = connection.query_row(
                "SELECT json_object('wrapper_id',w.wrapper_id,'wrapper_key',w.wrapper_key,'display_name',w.display_name,'wrapper_kind',w.wrapper_kind,'protocol_version',w.protocol_version,'wrapper_state',w.state,'connection_id',c.connection_id,'contract_version',c.contract_version,'lifecycle_state',c.lifecycle_state,'grant_revision',c.grant_revision,'last_seen_at_utc',c.last_seen_at_utc,'updated_at_utc',c.updated_at_utc) FROM wrapper_connections c JOIN wrapper_identities w ON w.wrapper_id=c.wrapper_id WHERE c.connection_id=?1 AND c.wrapper_id=?2",
                params![context.connection_id, context.wrapper_id],
                |row| {
                    let text: String = row.get(0)?;
                    Ok(serde_json::from_str(&text).unwrap_or_else(|_| json!({})))
                },
            )?;
            Ok(AdapterExecution {
                private_result: json!({"wrapper_status": value}),
                private_provenance: json!({"source":"local_wrapper_registry","credential_data_included":false}),
                source_count: 1,
                source_types: vec!["local_wrapper_registry".to_owned()],
                result_code: "wrapper_status_read".to_owned(),
            })
        }
        "receipt.read" => {
            let connection = state.connection()?;
            let total: i64 = connection.query_row(
                "SELECT COUNT(*) FROM wrapper_job_execution_receipts WHERE connection_id=?1",
                params![context.connection_id],
                |row| row.get(0),
            )?;
            let latest: Option<(String, String, String)> = connection
                .query_row(
                    "SELECT outcome,result_code,completed_at_utc FROM wrapper_job_execution_receipts WHERE connection_id=?1 ORDER BY completed_at_utc DESC,receipt_id DESC LIMIT 1",
                    params![context.connection_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            Ok(AdapterExecution {
                private_result: json!({
                    "receipt_summary": {
                        "count": total.max(0),
                        "latest": latest.map(|value| json!({
                            "outcome": value.0,
                            "result_code": value.1,
                            "completed_at_utc": value.2
                        }))
                    }
                }),
                private_provenance: json!({"source":"local_execution_receipts"}),
                source_count: 1,
                source_types: vec!["local_execution_receipts".to_owned()],
                result_code: "receipt_summary_read".to_owned(),
            })
        }
        "audit.record" => {
            let label = private_input
                .get("label")
                .and_then(Value::as_str)
                .map(|value| bounded_text(value, 1, 180, "audit label"))
                .transpose()?
                .unwrap_or_else(|| "Agent runtime audit record".to_owned());
            let input_hash = hash_json(private_input)?;
            let audit_record_id = Uuid::new_v4().to_string();
            let connection = state.connection()?;
            connection.execute(
                "INSERT INTO agent_runtime_audit_records (audit_record_id,plan_id,step_id,job_id,agent_id,input_hash,label,created_at_utc) VALUES (?1,?2,?3,(SELECT job_id FROM agent_runtime_plan_steps WHERE step_id=?3),?4,?5,?6,?7)",
                params![audit_record_id, context.plan_id, context.step_id, context.agent_id, input_hash, label, now_utc()],
            )?;
            Ok(AdapterExecution {
                private_result: json!({
                    "audit_record": {
                        "audit_record_id": audit_record_id,
                        "label": label,
                        "input_hash": input_hash,
                        "raw_input_stored": false
                    }
                }),
                private_provenance: json!({"source":"local_agent_runtime_audit"}),
                source_count: 0,
                source_types: Vec::new(),
                result_code: "audit_recorded".to_owned(),
            })
        }
        "result.compose" => {
            let result = private_input
                .get("result")
                .cloned()
                .context("result.compose requires a result value")?;
            Ok(AdapterExecution {
                private_result: json!({
                    "result": result,
                    "runtime": {
                        "plan_id": context.plan_id,
                        "step_id": context.step_id,
                        "agent_id": context.agent_id,
                        "action_type": context.action_type,
                        "phase16e_egress_required": true
                    }
                }),
                private_provenance: json!({"source":"agent_runtime_composition","private_input_hash":hash_json(private_input)?}),
                source_count: 0,
                source_types: Vec::new(),
                result_code: "private_result_composed".to_owned(),
            })
        }
        _ => bail!("runtime adapter is not implemented"),
    }
}

fn finalize_success(
    state: &AppState,
    context: &ExecutionContext,
    job: &wrapper_jobs::JobSummary,
    private_result_hash: &str,
    receipt: &wrapper_jobs::ExecutionReceiptSummary,
) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let now = now_utc();
    transaction.execute(
        "UPDATE agent_runtime_attempts SET state='completed',result_code=?1,private_result_hash=?2,safe_result_hash=?3,completed_at_utc=?4 WHERE attempt_id=?5 AND state='running'",
        params![receipt.result_code, private_result_hash, receipt.safe_result_hash, now, context.attempt_id],
    )?;
    transaction.execute(
        "UPDATE agent_runtime_plan_steps SET state='completed',private_result_hash=?1,safe_result_hash=?2,result_code=?3,completed_at_utc=?4,updated_at_utc=?4 WHERE step_id=?5",
        params![private_result_hash, receipt.safe_result_hash, receipt.result_code, now, context.step_id],
    )?;
    let runtime_receipt_id = Uuid::new_v4().to_string();
    let receipt_document = json!({
        "schema":"homeserver.agent-runtime-receipt.v1",
        "receipt_id":runtime_receipt_id,
        "plan_id":context.plan_id,
        "step_id":context.step_id,
        "job_id":job.job_id,
        "agent_id":context.agent_id,
        "wrapper_id":context.wrapper_id,
        "connection_id":context.connection_id,
        "tool_key":context.tool.tool_key,
        "adapter_key":context.tool.adapter_key,
        "outcome":"completed",
        "result_code":receipt.result_code,
        "job_receipt_id":receipt.receipt_id,
        "job_receipt_hash":receipt.receipt_hash,
        "safe_result_hash":receipt.safe_result_hash,
        "completed_at_utc":receipt.completed_at_utc
    });
    let runtime_receipt_hash = hash_json(&receipt_document)?;
    transaction.execute(
        "INSERT INTO agent_runtime_receipts (receipt_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,tool_key,adapter_key,outcome,result_code,job_receipt_id,job_receipt_hash,safe_result_hash,runtime_receipt_hash,completed_at_utc,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'completed',?10,?11,?12,?13,?14,?15,?15)",
        params![runtime_receipt_id, context.plan_id, context.step_id, job.job_id, context.agent_id, context.wrapper_id, context.connection_id, context.tool.tool_key, context.tool.adapter_key, receipt.result_code, receipt.receipt_id, receipt.receipt_hash, receipt.safe_result_hash, runtime_receipt_hash, receipt.completed_at_utc],
    )?;
    refresh_plan_state_tx(&transaction, &context.plan_id)?;
    record_event_tx(
        &transaction,
        EventEvidence {
            plan_id: Some(&context.plan_id),
            step_id: Some(&context.step_id),
            job_id: Some(&job.job_id),
            agent_id: Some(&context.agent_id),
            event_type: "agent.runtime_step_completed",
            outcome: "success",
            actor_type: "worker",
            actor_id: "agent_runtime",
            detail_code: &receipt.result_code,
            metadata: json!({
                "attempt_number": context.attempt_number,
                "runtime_receipt_hash": runtime_receipt_hash,
                "job_receipt_hash": receipt.receipt_hash,
                "safe_result_hash": receipt.safe_result_hash,
                "private_result_exposed": false
            }),
        },
    )?;
    transaction.commit()?;
    Ok(())
}

fn fail_runtime_job(
    state: &AppState,
    worker_id: &str,
    leased: &wrapper_jobs::LeasedJob,
    failure_code: &str,
    failure: &anyhow::Error,
) -> Result<()> {
    let failed_job = {
        let connection = state.connection()?;
        wrapper_jobs::fail_job(
            &connection,
            wrapper_jobs::FailJobRequest {
                worker_id: worker_id.to_owned(),
                job_id: leased.job.job_id.clone(),
                lease_token: leased.lease_token.clone(),
                failure_code: failure_code.to_owned(),
                retryable: false,
            },
        )?
    };
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let context: Option<(String, String, String)> = transaction
        .query_row(
            "SELECT s.plan_id,s.step_id,p.agent_id FROM agent_runtime_plan_steps s JOIN agent_runtime_plans p ON p.plan_id=s.plan_id WHERE s.job_id=?1",
            params![leased.job.job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((plan_id, step_id, agent_id)) = context {
        let now = now_utc();
        transaction.execute(
            "UPDATE agent_runtime_attempts SET state='failed',result_code=?1,completed_at_utc=?2 WHERE step_id=?3 AND state='running'",
            params![failure_code, now, step_id],
        )?;
        transaction.execute(
            "UPDATE agent_runtime_plan_steps SET state='failed',failure_code=?1,result_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE step_id=?3",
            params![failure_code, now, step_id],
        )?;
        transaction.execute(
            "UPDATE agent_runtime_plans SET state='failed',failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE plan_id=?3",
            params![failure_code, now, plan_id],
        )?;
        let runtime_receipt_id = Uuid::new_v4().to_string();
        let job_receipt = failed_job.receipt.as_ref();
        let receipt_document = json!({
            "schema":"homeserver.agent-runtime-receipt.v1",
            "receipt_id":runtime_receipt_id,
            "plan_id":plan_id,
            "step_id":step_id,
            "job_id":leased.job.job_id,
            "agent_id":agent_id,
            "wrapper_id":leased.job.wrapper_id,
            "connection_id":leased.job.connection_id,
            "tool_key":transaction.query_row("SELECT tool_key FROM agent_runtime_plan_steps WHERE step_id=?1",params![step_id],|row| row.get::<_,String>(0))?,
            "outcome":"failed",
            "result_code":failure_code,
            "job_receipt_hash":job_receipt.map(|value| value.receipt_hash.clone()),
            "completed_at_utc":now
        });
        let runtime_receipt_hash = hash_json(&receipt_document)?;
        transaction.execute(
            "INSERT OR IGNORE INTO agent_runtime_receipts (receipt_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,tool_key,adapter_key,outcome,result_code,job_receipt_id,job_receipt_hash,safe_result_hash,runtime_receipt_hash,completed_at_utc,created_at_utc) SELECT ?1,?2,?3,?4,?5,?6,?7,s.tool_key,s.adapter_key,'failed',?8,?9,?10,?11,?12,?13,?13 FROM agent_runtime_plan_steps s WHERE s.step_id=?3",
            params![runtime_receipt_id, plan_id, step_id, leased.job.job_id, agent_id, leased.job.wrapper_id, leased.job.connection_id, failure_code, job_receipt.map(|value| value.receipt_id.clone()), job_receipt.map(|value| value.receipt_hash.clone()), job_receipt.and_then(|value| value.safe_result_hash.clone()), runtime_receipt_hash, now],
        )?;
        record_event_tx(
            &transaction,
            EventEvidence {
                plan_id: Some(&plan_id),
                step_id: Some(&step_id),
                job_id: Some(&leased.job.job_id),
                agent_id: Some(&agent_id),
                event_type: "agent.runtime_step_failed",
                outcome: "error",
                actor_type: "worker",
                actor_id: worker_id,
                detail_code: failure_code,
                metadata: json!({"failure_hash":hash_text(&failure.to_string()),"private_error_exposed":false}),
            },
        )?;
        transaction.commit()?;
        cancel_remaining_jobs(state, &plan_id, "runtime plan failed")?;
    } else {
        transaction.commit()?;
    }
    Ok(())
}

fn cancel_remaining_jobs(state: &AppState, plan_id: &str, reason: &str) -> Result<()> {
    let jobs: Vec<(String, String)> = {
        let connection = state.connection()?;
        let mut statement = connection.prepare(
            "SELECT j.job_id,j.connection_id FROM agent_runtime_plan_steps s JOIN wrapper_jobs j ON j.job_id=s.job_id WHERE s.plan_id=?1 AND j.state IN ('queued','leased','running','waiting') ORDER BY s.sequence_number",
        )?;
        let rows = statement.query_map(params![plan_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (job_id, connection_id) in jobs {
        let connection = state.connection()?;
        let _ = wrapper_jobs::cancel_job(
            &connection,
            wrapper_jobs::CancelJobRequest {
                connection_id,
                job_id: job_id.clone(),
                actor_type: "system".to_owned(),
                actor_id: "agent_runtime".to_owned(),
                confirmation: format!("CANCEL JOB {job_id}"),
                reason: reason.to_owned(),
            },
        );
    }
    Ok(())
}

fn snapshot(state: &AppState) -> Result<RuntimeSnapshot> {
    let connection = state.connection()?;
    reconcile(&connection)?;
    let (worker_id, runtime_state): (String, String) = connection.query_row(
        "SELECT worker_id,state FROM agent_runtime_state WHERE singleton_id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(RuntimeSnapshot {
        schema: "homeserver.agent-runtime.v1".to_owned(),
        runtime_state,
        worker_id,
        tools: read_tools(&connection)?,
        plans: read_plans(&connection)?,
        steps: read_steps(&connection)?,
        receipts: read_receipts(&connection)?,
        private_inputs_exposed: false,
        private_results_exposed: false,
        direct_tool_bypass_allowed: false,
        phase16e_egress_required: true,
    })
}

fn reconcile(connection: &Connection) -> Result<()> {
    let now = now_utc();
    connection.execute(
        "UPDATE agent_runtime_attempts SET state='failed',result_code='service_restarted',completed_at_utc=?1 WHERE state='running'",
        params![now],
    )?;
    connection.execute(
        "UPDATE agent_runtime_plan_steps SET state=(SELECT j.state FROM wrapper_jobs j WHERE j.job_id=agent_runtime_plan_steps.job_id),failure_code=COALESCE(failure_code,(SELECT j.failure_code FROM wrapper_jobs j WHERE j.job_id=agent_runtime_plan_steps.job_id)),completed_at_utc=COALESCE(completed_at_utc,(SELECT j.completed_at_utc FROM wrapper_jobs j WHERE j.job_id=agent_runtime_plan_steps.job_id)),updated_at_utc=?1 WHERE state IN ('queued','leased','running')",
        params![now],
    )?;
    connection.execute(
        "UPDATE agent_runtime_plans SET state='expired',failure_code='plan_expired',completed_at_utc=?1,updated_at_utc=?1 WHERE state IN ('queued','running') AND expires_at_utc<=?1",
        params![now],
    )?;
    let mut statement = connection.prepare(
        "SELECT plan_id FROM agent_runtime_plans WHERE state IN ('queued','running') ORDER BY created_at_utc LIMIT 500",
    )?;
    let plan_ids = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for plan_id in plan_ids {
        refresh_plan_state(connection, &plan_id)?;
    }
    Ok(())
}

fn refresh_plan_state(connection: &Connection, plan_id: &str) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    refresh_plan_state_tx(&transaction, plan_id)?;
    transaction.commit()?;
    Ok(())
}

fn refresh_plan_state_tx(transaction: &Transaction<'_>, plan_id: &str) -> Result<()> {
    let (total, completed, failed, cancelled): (i64, i64, i64, i64) = transaction.query_row(
        "SELECT COUNT(*),SUM(CASE WHEN state='completed' THEN 1 ELSE 0 END),SUM(CASE WHEN state='failed' THEN 1 ELSE 0 END),SUM(CASE WHEN state IN ('cancelled','expired') THEN 1 ELSE 0 END) FROM agent_runtime_plan_steps WHERE plan_id=?1",
        params![plan_id],
        |row| Ok((row.get(0)?, row.get::<_, Option<i64>>(1)?.unwrap_or(0), row.get::<_, Option<i64>>(2)?.unwrap_or(0), row.get::<_, Option<i64>>(3)?.unwrap_or(0))),
    )?;
    let now = now_utc();
    if total > 0 && completed == total {
        transaction.execute(
            "UPDATE agent_runtime_plans SET state='completed',completed_step_count=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE plan_id=?3 AND state IN ('queued','running')",
            params![completed, now, plan_id],
        )?;
    } else if failed > 0 {
        transaction.execute(
            "UPDATE agent_runtime_plans SET state='failed',completed_step_count=?1,failure_code=COALESCE(failure_code,'step_failed'),completed_at_utc=COALESCE(completed_at_utc,?2),updated_at_utc=?2 WHERE plan_id=?3 AND state IN ('queued','running')",
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

fn read_tools(connection: &Connection) -> Result<Vec<ToolSummary>> {
    let mut statement = connection.prepare(
        "SELECT tool_key,adapter_key,version,description,risk_class,approval_requirement,allowed_job_types_json,max_execution_seconds,state FROM agent_tool_catalog ORDER BY tool_key",
    )?;
    statement
        .query_map([], |row| {
            Ok(ToolSummary {
                tool_key: row.get(0)?,
                adapter_key: row.get(1)?,
                version: row.get(2)?,
                description: row.get(3)?,
                risk_class: row.get(4)?,
                approval_requirement: row.get(5)?,
                allowed_job_types: parse_string_list(&row.get::<_, String>(6)?),
                max_execution_seconds: row.get::<_, i64>(7)?.max(1) as u32,
                state: row.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_tool(connection: &Connection, tool_key: &str) -> Result<ToolRecord> {
    connection
        .query_row(
            "SELECT tool_key,adapter_key,risk_class,approval_requirement,allowed_job_types_json,max_execution_seconds,state FROM agent_tool_catalog WHERE tool_key=?1",
            params![tool_key],
            map_tool_record,
        )
        .context("runtime tool was not found")
}

fn read_tool_tx(transaction: &Transaction<'_>, tool_key: &str) -> Result<ToolRecord> {
    transaction
        .query_row(
            "SELECT tool_key,adapter_key,risk_class,approval_requirement,allowed_job_types_json,max_execution_seconds,state FROM agent_tool_catalog WHERE tool_key=?1",
            params![tool_key],
            map_tool_record,
        )
        .context("runtime tool was not found")
}

fn map_tool_record(row: &Row<'_>) -> rusqlite::Result<ToolRecord> {
    Ok(ToolRecord {
        tool_key: row.get(0)?,
        adapter_key: row.get(1)?,
        risk_class: row.get(2)?,
        approval_requirement: row.get(3)?,
        allowed_job_types: parse_string_list(&row.get::<_, String>(4)?),
        max_execution_seconds: row.get::<_, i64>(5)?.max(1) as u32,
        state: row.get(6)?,
    })
}

fn read_plans(connection: &Connection) -> Result<Vec<RuntimePlanSummary>> {
    let mut statement = connection.prepare(
        "SELECT plan_id,agent_id,requested_by_user_id,title,objective,state,step_count,completed_step_count,correlation_id,plan_hash,expires_at_utc,failure_code,created_at_utc,updated_at_utc,completed_at_utc FROM agent_runtime_plans ORDER BY created_at_utc DESC,plan_id DESC LIMIT ?1",
    )?;
    statement
        .query_map(params![MAX_PLANS], |row| {
            Ok(RuntimePlanSummary {
                plan_id: row.get(0)?,
                agent_id: row.get(1)?,
                requested_by_user_id: row.get(2)?,
                title: row.get(3)?,
                objective: row.get(4)?,
                state: row.get(5)?,
                step_count: row.get::<_, i64>(6)?.max(0) as u32,
                completed_step_count: row.get::<_, i64>(7)?.max(0) as u32,
                correlation_id: row.get(8)?,
                plan_hash: row.get(9)?,
                expires_at_utc: row.get(10)?,
                failure_code: row.get(11)?,
                created_at_utc: row.get(12)?,
                updated_at_utc: row.get(13)?,
                completed_at_utc: row.get(14)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_steps(connection: &Connection) -> Result<Vec<RuntimeStepSummary>> {
    let mut statement = connection.prepare(
        "SELECT step_id,plan_id,sequence_number,job_id,tool_key,adapter_key,action_type,state,idempotency_key,argument_hash,private_result_hash,safe_result_hash,result_code,failure_code,created_at_utc,started_at_utc,completed_at_utc FROM agent_runtime_plan_steps ORDER BY created_at_utc DESC,plan_id DESC,sequence_number LIMIT 1000",
    )?;
    statement
        .query_map([], |row| {
            Ok(RuntimeStepSummary {
                step_id: row.get(0)?,
                plan_id: row.get(1)?,
                sequence_number: row.get::<_, i64>(2)?.max(0) as u32,
                job_id: row.get(3)?,
                tool_key: row.get(4)?,
                adapter_key: row.get(5)?,
                action_type: row.get(6)?,
                state: row.get(7)?,
                idempotency_key: row.get(8)?,
                argument_hash: row.get(9)?,
                private_result_hash: row.get(10)?,
                safe_result_hash: row.get(11)?,
                result_code: row.get(12)?,
                failure_code: row.get(13)?,
                created_at_utc: row.get(14)?,
                started_at_utc: row.get(15)?,
                completed_at_utc: row.get(16)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_receipts(connection: &Connection) -> Result<Vec<RuntimeReceiptSummary>> {
    let mut statement = connection.prepare(
        "SELECT receipt_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,tool_key,adapter_key,outcome,result_code,job_receipt_id,job_receipt_hash,safe_result_hash,runtime_receipt_hash,completed_at_utc FROM agent_runtime_receipts ORDER BY completed_at_utc DESC,receipt_id DESC LIMIT 500",
    )?;
    statement
        .query_map([], |row| {
            Ok(RuntimeReceiptSummary {
                receipt_id: row.get(0)?,
                plan_id: row.get(1)?,
                step_id: row.get(2)?,
                job_id: row.get(3)?,
                agent_id: row.get(4)?,
                wrapper_id: row.get(5)?,
                connection_id: row.get(6)?,
                tool_key: row.get(7)?,
                adapter_key: row.get(8)?,
                outcome: row.get(9)?,
                result_code: row.get(10)?,
                job_receipt_id: row.get(11)?,
                job_receipt_hash: row.get(12)?,
                safe_result_hash: row.get(13)?,
                runtime_receipt_hash: row.get(14)?,
                completed_at_utc: row.get(15)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

struct EventEvidence<'a> {
    plan_id: Option<&'a str>,
    step_id: Option<&'a str>,
    job_id: Option<&'a str>,
    agent_id: Option<&'a str>,
    event_type: &'a str,
    outcome: &'a str,
    actor_type: &'a str,
    actor_id: &'a str,
    detail_code: &'a str,
    metadata: Value,
}

fn record_event(connection: &Connection, evidence: EventEvidence<'_>) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    record_event_tx(&transaction, evidence)?;
    transaction.commit()?;
    Ok(())
}

fn record_event_tx(transaction: &Transaction<'_>, evidence: EventEvidence<'_>) -> Result<()> {
    let event_id = Uuid::new_v4().to_string();
    let created_at = now_utc();
    let event_document = json!({
        "event_id": event_id,
        "plan_id": evidence.plan_id,
        "step_id": evidence.step_id,
        "job_id": evidence.job_id,
        "agent_id": evidence.agent_id,
        "event_type": evidence.event_type,
        "outcome": evidence.outcome,
        "actor_type": evidence.actor_type,
        "actor_id": evidence.actor_id,
        "detail_code": evidence.detail_code,
        "metadata": evidence.metadata,
        "created_at_utc": created_at
    });
    let event_hash = hash_json(&event_document)?;
    transaction.execute(
        "INSERT INTO agent_runtime_events (event_id,plan_id,step_id,job_id,agent_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![event_id,evidence.plan_id,evidence.step_id,evidence.job_id,evidence.agent_id,evidence.event_type,evidence.outcome,evidence.actor_type,evidence.actor_id,evidence.detail_code,serde_json::to_string(&evidence.metadata)?,event_hash,created_at],
    )?;
    Ok(())
}

fn parse_string_list(text: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(text).unwrap_or_default()
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
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

fn now_utc() -> String {
    timestamp(Utc::now())
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_job_types_are_unique() {
        let values = RUNTIME_JOB_TYPES.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(values.len(), RUNTIME_JOB_TYPES.len());
    }

    #[test]
    fn runtime_hashes_are_stable() {
        let value = json!({"agent":"local","authority":"bounded"});
        assert_eq!(hash_json(&value).unwrap(), hash_json(&value).unwrap());
    }
}
