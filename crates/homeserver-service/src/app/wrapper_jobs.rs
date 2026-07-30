use super::wrapper_grants::{self, AuthorizeRequest};
use crate::AppState;
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0022_wrapper_jobs_events_receipts.sql");
const AUTHORITY_MIGRATION: &str =
    include_str!("../../../../database/migrations/0022a_wrapper_job_authority_snapshots.sql");
const MIGRATION_KEY: &str = "0022_wrapper_jobs_events_receipts";
const AUTHORITY_MIGRATION_KEY: &str = "0022a_wrapper_job_authority_snapshots";
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;
const MAX_PRIVATE_INPUT_BYTES: usize = 1024 * 1024;
const MAX_PRIVATE_RESULT_BYTES: usize = 1024 * 1024;
const MAX_JOBS_PER_SNAPSHOT: i64 = 250;
const MAX_EVENTS_PER_SNAPSHOT: i64 = 500;
const MAX_DELIVERIES_PER_POLL: i64 = 100;
const MAX_WORKER_CLAIM: i64 = 16;
const MAX_JOB_EVENTS: i64 = 50_000;
const MAX_RECEIPTS: i64 = 50_000;
const FILTER_VERSION: &str = "wrapper-safe-projection-v1";
const TERMINAL_STATES: &[&str] = &["completed", "failed", "cancelled", "expired", "dead_letter"];
const WORKER_KINDS: &[&str] = &["agent", "model", "tool", "connector", "media", "system"];
const SUBMITTER_TYPES: &[&str] = &["wrapper", "local_user", "agent", "system"];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerSummary {
    pub worker_id: String,
    pub worker_kind: String,
    pub display_name: String,
    pub allowed_job_types: Vec<String>,
    pub max_concurrent_jobs: u32,
    pub state: String,
    pub revision: u64,
    pub last_seen_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafeResultSummary {
    pub job_id: String,
    pub result_policy: String,
    pub safe_result: Value,
    pub safe_result_hash: String,
    pub provenance_summary: Value,
    pub provenance_summary_hash: String,
    pub filter_version: String,
    pub result_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionReceiptSummary {
    pub receipt_id: String,
    pub job_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub connection_authority_revision: u64,
    pub authorization_decision_id: String,
    pub capability_key: String,
    pub operation: String,
    pub job_type: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub payload_hash: String,
    pub approval_id: Option<String>,
    pub plan_hash: Option<String>,
    pub correlation_id: String,
    pub causation_id: Option<String>,
    pub outcome: String,
    pub result_code: String,
    pub safe_result_hash: Option<String>,
    pub provenance_summary_hash: Option<String>,
    pub worker_id: Option<String>,
    pub worker_kind: Option<String>,
    pub attempt_count: u32,
    pub started_at_utc: Option<String>,
    pub completed_at_utc: String,
    pub receipt_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobEventSummary {
    pub event_id: String,
    pub job_id: String,
    pub sequence_number: u64,
    pub event_type: String,
    pub previous_state: Option<String>,
    pub current_state: String,
    pub outcome: String,
    pub detail_code: String,
    pub actor_type: String,
    pub actor_id: String,
    pub metadata: Value,
    pub event_hash: String,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliverySummary {
    pub delivery_id: String,
    pub job_id: String,
    pub receipt_id: String,
    pub connection_id: String,
    pub state: String,
    pub payload_hash: String,
    pub attempt_count: u32,
    pub next_attempt_at_utc: String,
    pub acknowledged_at_utc: Option<String>,
    pub expires_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobSummary {
    pub job_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub connection_authority_revision: u64,
    pub authorization_decision_id: String,
    pub capability_key: String,
    pub operation: String,
    pub job_type: String,
    pub state: String,
    pub priority: u8,
    pub idempotency_key: String,
    pub request_hash: String,
    pub payload_hash: String,
    pub scope_kind: Option<String>,
    pub scope_value: Option<String>,
    pub result_policy: String,
    pub allowed_result_fields: Vec<String>,
    pub max_result_bytes: u64,
    pub max_execution_seconds: u32,
    pub max_attempts: u8,
    pub attempt_count: u8,
    pub approval_id: Option<String>,
    pub plan_hash: Option<String>,
    pub correlation_id: String,
    pub causation_id: Option<String>,
    pub submitted_by_type: String,
    pub submitted_by_id: String,
    pub available_at_utc: String,
    pub expires_at_utc: String,
    pub lease_owner_id: Option<String>,
    pub lease_expires_at_utc: Option<String>,
    pub started_at_utc: Option<String>,
    pub completed_at_utc: Option<String>,
    pub cancelled_at_utc: Option<String>,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub safe_result: Option<SafeResultSummary>,
    pub receipt: Option<ExecutionReceiptSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectionJobSnapshot {
    pub schema: String,
    pub connection_id: String,
    pub jobs: Vec<JobSummary>,
    pub events: Vec<JobEventSummary>,
    pub pending_deliveries: Vec<DeliverySummary>,
    pub queued_jobs: u64,
    pub active_jobs: u64,
    pub terminal_jobs: u64,
    pub private_inputs_exposed: bool,
    pub private_results_exposed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmittedJobResponse {
    pub job_id: String,
    pub state: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub payload_hash: String,
    pub authorization_decision_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub connection_authority_revision: u64,
    pub result_policy: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LeasedJob {
    pub job: JobSummary,
    pub lease_token: String,
    pub private_input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClaimJobsResponse {
    pub worker: WorkerSummary,
    pub jobs: Vec<LeasedJob>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotRequest {
    pub connection_id: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubmitJobRequest {
    pub connection_id: String,
    pub capability_key: String,
    pub operation: String,
    pub job_type: String,
    pub idempotency_key: String,
    pub private_input: Value,
    pub scope_kind: Option<String>,
    pub scope_value: Option<String>,
    pub estimated_result_bytes: Option<u64>,
    pub estimated_token_count: Option<u64>,
    pub approval_id: Option<String>,
    pub plan_hash: Option<String>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub submitted_by_type: String,
    pub submitted_by_id: String,
    pub priority: Option<u8>,
    pub expires_minutes: u32,
    pub max_attempts: Option<u8>,
    pub available_at_utc: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegisterWorkerRequest {
    pub worker_kind: String,
    pub display_name: String,
    pub allowed_job_types: Vec<String>,
    pub max_concurrent_jobs: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClaimJobsRequest {
    pub worker_id: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerLeaseRequest {
    pub worker_id: String,
    pub job_id: String,
    pub lease_token: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompleteJobRequest {
    pub worker_id: String,
    pub job_id: String,
    pub lease_token: String,
    pub private_result: Value,
    #[serde(default)]
    pub private_provenance: Value,
    pub source_count: u32,
    #[serde(default)]
    pub source_types: Vec<String>,
    pub evidence_hash: Option<String>,
    pub actual_token_count: Option<u64>,
    pub result_code: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FailJobRequest {
    pub worker_id: String,
    pub job_id: String,
    pub lease_token: String,
    pub failure_code: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelJobRequest {
    pub connection_id: String,
    pub job_id: String,
    pub actor_type: String,
    pub actor_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PollDeliveriesRequest {
    pub connection_id: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeliveryEnvelope {
    pub delivery: DeliverySummary,
    pub job: JobSummary,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AckDeliveryRequest {
    pub connection_id: String,
    pub delivery_id: String,
    pub receipt_hash: String,
}

#[derive(Debug, Clone)]
struct JobRecord {
    job_id: String,
    wrapper_id: String,
    connection_id: String,
    grant_id: String,
    grant_revision: u64,
    connection_authority_revision: u64,
    authorization_decision_id: String,
    capability_key: String,
    operation: String,
    job_type: String,
    state: String,
    priority: u8,
    idempotency_key: String,
    request_hash: String,
    payload_hash: String,
    scope_kind: Option<String>,
    scope_value: Option<String>,
    result_policy: String,
    allowed_result_fields: Vec<String>,
    max_result_bytes: u64,
    max_execution_seconds: u32,
    max_attempts: u8,
    attempt_count: u8,
    approval_id: Option<String>,
    plan_hash: Option<String>,
    correlation_id: String,
    causation_id: Option<String>,
    submitted_by_type: String,
    submitted_by_id: String,
    available_at_utc: String,
    expires_at_utc: String,
    lease_owner_id: Option<String>,
    lease_token_hash: Option<String>,
    lease_expires_at_utc: Option<String>,
    started_at_utc: Option<String>,
    completed_at_utc: Option<String>,
    cancelled_at_utc: Option<String>,
    failure_code: Option<String>,
    created_at_utc: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    connection.execute_batch(AUTHORITY_MIGRATION)?;
    reconcile_authority(connection)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    for key in [MIGRATION_KEY, AUTHORITY_MIGRATION_KEY] {
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
            params![key],
            |row| row.get(0),
        )?;
        ensure!(
            count == 1,
            "wrapper job migration is not registered exactly once"
        );
    }
    for table in [
        "wrapper_job_workers",
        "wrapper_jobs",
        "wrapper_job_inputs",
        "wrapper_job_events",
        "wrapper_job_private_results",
        "wrapper_job_safe_results",
        "wrapper_job_execution_receipts",
        "wrapper_job_deliveries",
        "wrapper_job_authority_snapshots",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let orphan_authority: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_jobs j LEFT JOIN wrapper_job_authority_snapshots a ON a.job_id=j.job_id WHERE a.job_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        orphan_authority == 0,
        "wrapper jobs are missing authority snapshots"
    );
    let cross_wrapper_jobs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_jobs j LEFT JOIN wrapper_connections c ON c.connection_id=j.connection_id AND c.wrapper_id=j.wrapper_id LEFT JOIN wrapper_capability_grants g ON g.grant_id=j.grant_id AND g.connection_id=j.connection_id AND g.wrapper_id=j.wrapper_id WHERE c.connection_id IS NULL OR g.grant_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        cross_wrapper_jobs == 0,
        "wrapper jobs contain cross-wrapper authority"
    );
    let incomplete_terminal: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_jobs j LEFT JOIN wrapper_job_execution_receipts r ON r.job_id=j.job_id WHERE j.state IN ('completed','failed','cancelled','expired','dead_letter') AND r.job_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        incomplete_terminal == 0,
        "terminal wrapper jobs are missing receipts"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    reconcile_authority(connection)?;
    connection.execute(
        "UPDATE wrapper_job_deliveries SET state='expired',updated_at_utc=?1 WHERE state IN ('pending','in_flight') AND expires_at_utc<=?1",
        params![now_utc()],
    )?;
    connection.execute(
        "DELETE FROM wrapper_job_events WHERE created_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM wrapper_job_events WHERE event_id NOT IN (SELECT event_id FROM wrapper_job_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1)",
        params![MAX_JOB_EVENTS],
    )?;
    let receipt_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_job_execution_receipts",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        receipt_count <= MAX_RECEIPTS,
        "wrapper job receipt retention requires archival"
    );
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/wrapper-jobs", get(status_handler))
        .route("/v1/wrapper-jobs/snapshot", post(snapshot_handler))
        .route("/v1/wrapper-jobs/submit", post(submit_handler))
        .route("/v1/wrapper-jobs/cancel", post(cancel_handler))
        .route(
            "/v1/wrapper-jobs/deliveries/poll",
            post(poll_deliveries_handler),
        )
        .route(
            "/v1/wrapper-jobs/deliveries/ack",
            post(ack_delivery_handler),
        )
        .route(
            "/v1/internal/wrapper-jobs/workers/register",
            post(register_worker_handler),
        )
        .route("/v1/internal/wrapper-jobs/claim", post(claim_jobs_handler))
        .route("/v1/internal/wrapper-jobs/start", post(start_job_handler))
        .route(
            "/v1/internal/wrapper-jobs/heartbeat",
            post(heartbeat_job_handler),
        )
        .route(
            "/v1/internal/wrapper-jobs/complete",
            post(complete_job_handler),
        )
        .route("/v1/internal/wrapper-jobs/fail", post(fail_job_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn status_handler() -> Json<Value> {
    Json(json!({
        "schema": "homeserver.wrapper-jobs.v1",
        "private_inputs_exposed": false,
        "private_results_exposed": false,
        "authority_required": true,
        "offline_delivery_supported": true,
        "result_filter_version": FILTER_VERSION
    }))
}

async fn snapshot_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SnapshotRequest>,
) -> ApiResult<ConnectionJobSnapshot> {
    run_blocking(
        move || snapshot(&state, request),
        "wrapper_job_snapshot_failed",
    )
    .await
}

async fn submit_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SubmitJobRequest>,
) -> ApiResult<SubmittedJobResponse> {
    run_blocking(
        move || submit_job(&state, request),
        "wrapper_job_submit_failed",
    )
    .await
}

async fn cancel_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CancelJobRequest>,
) -> ApiResult<ConnectionJobSnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            cancel_job(&connection, request)?;
            snapshot_with_connection(
                &connection,
                SnapshotRequest {
                    connection_id: String::new(),
                    limit: None,
                },
            )
        },
        "wrapper_job_cancel_failed",
    )
    .await
}

async fn register_worker_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RegisterWorkerRequest>,
) -> ApiResult<WorkerSummary> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            register_worker(&connection, request)
        },
        "wrapper_job_worker_register_failed",
    )
    .await
}

async fn claim_jobs_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ClaimJobsRequest>,
) -> ApiResult<ClaimJobsResponse> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            claim_jobs(&connection, request)
        },
        "wrapper_job_claim_failed",
    )
    .await
}

async fn start_job_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkerLeaseRequest>,
) -> ApiResult<JobSummary> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            start_job(&connection, request)
        },
        "wrapper_job_start_failed",
    )
    .await
}

async fn heartbeat_job_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<WorkerLeaseRequest>,
) -> ApiResult<JobSummary> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            heartbeat_job(&connection, request)
        },
        "wrapper_job_heartbeat_failed",
    )
    .await
}

async fn complete_job_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CompleteJobRequest>,
) -> ApiResult<ExecutionReceiptSummary> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            complete_job(&connection, request)
        },
        "wrapper_job_complete_failed",
    )
    .await
}

async fn fail_job_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<FailJobRequest>,
) -> ApiResult<JobSummary> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            fail_job(&connection, request)
        },
        "wrapper_job_fail_failed",
    )
    .await
}

async fn poll_deliveries_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PollDeliveriesRequest>,
) -> ApiResult<Vec<DeliveryEnvelope>> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            poll_deliveries(&connection, request)
        },
        "wrapper_job_delivery_poll_failed",
    )
    .await
}

async fn ack_delivery_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AckDeliveryRequest>,
) -> ApiResult<DeliverySummary> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            acknowledge_delivery(&connection, request)
        },
        "wrapper_job_delivery_ack_failed",
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
        .map_err(|error| api_error(code, anyhow::anyhow!("wrapper job task failed: {error}")))?
        .map(Json)
        .map_err(|error| api_error(code, error))
}

pub fn snapshot(state: &AppState, request: SnapshotRequest) -> Result<ConnectionJobSnapshot> {
    let connection = state.connection()?;
    snapshot_with_connection(&connection, request)
}

include!("wrapper_jobs_submit.rs");
include!("wrapper_jobs_workers.rs");
include!("wrapper_jobs_completion.rs");
include!("wrapper_jobs_delivery.rs");
include!("wrapper_jobs_read.rs");
include!("wrapper_jobs_projection.rs");
include!("wrapper_jobs_reconcile.rs");
include!("wrapper_jobs_support.rs");
