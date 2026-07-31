use super::wrapper_runtime;
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
use std::{sync::Arc, time::Duration as StdDuration};
use tokio::sync::watch;
use tracing::{error, warn};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0027_authorized_agent_scheduling.sql");
const MIGRATION_KEY: &str = "0027_authorized_agent_scheduling";
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;
const MAX_TEMPLATE_BYTES: usize = 1024 * 1024;
const MAX_SAFE_EVENT_BYTES: usize = 16 * 1024;
const MAX_SCHEDULES: i64 = 1_000;
const MAX_RUNS: i64 = 100_000;
const MAX_EVENTS: i64 = 100_000;
const MAX_RECEIPTS: i64 = 100_000;
const MAX_AUDIT_EVENTS: i64 = 100_000;
const MAX_RUNS_PER_CYCLE: usize = 16;
const SCHEDULER_ACTOR: &str = "agent_scheduler";
const EVENT_TOPICS: &[&str] = &[
    "wrapper.job.completed",
    "runtime.plan.completed",
    "supervised.action.completed",
    "cloud.sync.completed",
];
const FORBIDDEN_EVENT_KEYS: &[&str] = &[
    "private_input",
    "private_result",
    "payload",
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
type NormalizedTrigger = (
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleSummary {
    pub schedule_id: String,
    pub agent_id: String,
    pub agent_revision: u64,
    pub assignment_id: String,
    pub assignment_revision: u64,
    pub wrapper_id: String,
    pub connection_id: String,
    pub connection_authority_revision: u64,
    pub created_by_user_id: String,
    pub title: String,
    pub description: String,
    pub state: String,
    pub trigger_kind: String,
    pub run_at_utc: Option<String>,
    pub interval_seconds: Option<u32>,
    pub event_topic: Option<String>,
    pub event_source_id: Option<String>,
    pub misfire_policy: String,
    pub overlap_policy: String,
    pub debounce_seconds: u32,
    pub max_runs: u32,
    pub run_count: u32,
    pub template_hash: String,
    pub authority_hash: String,
    pub next_fire_at_utc: Option<String>,
    pub last_fired_at_utc: Option<String>,
    pub expires_at_utc: String,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleRunSummary {
    pub run_id: String,
    pub schedule_id: String,
    pub trigger_kind: String,
    pub trigger_token: String,
    pub event_id: Option<String>,
    pub scheduled_for_utc: String,
    pub state: String,
    pub authority_hash: String,
    pub template_hash: String,
    pub plan_id: Option<String>,
    pub plan_hash: Option<String>,
    pub outcome: Option<String>,
    pub result_code: Option<String>,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub started_at_utc: Option<String>,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafeEventSummary {
    pub event_sequence: u64,
    pub event_id: String,
    pub topic: String,
    pub source_type: String,
    pub source_id: String,
    pub event_key: String,
    pub safe_metadata: Value,
    pub payload_hash: String,
    pub occurred_at_utc: String,
    pub received_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleReceiptSummary {
    pub receipt_id: String,
    pub schedule_id: String,
    pub run_id: String,
    pub agent_id: String,
    pub assignment_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub trigger_kind: String,
    pub trigger_token: String,
    pub event_id: Option<String>,
    pub outcome: String,
    pub result_code: String,
    pub authority_hash: String,
    pub template_hash: String,
    pub plan_id: Option<String>,
    pub plan_hash: Option<String>,
    pub receipt_hash: String,
    pub completed_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleSnapshot {
    pub schema: String,
    pub scheduler_state: String,
    pub scheduler_revision: u64,
    pub schedules: Vec<ScheduleSummary>,
    pub runs: Vec<ScheduleRunSummary>,
    pub events: Vec<SafeEventSummary>,
    pub receipts: Vec<ScheduleReceiptSummary>,
    pub allowed_event_topics: Vec<String>,
    pub private_templates_exposed: bool,
    pub private_event_payloads_exposed: bool,
    pub direct_execution_allowed: bool,
    pub phase17_runtime_required: bool,
    pub phase18_supervision_required: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TriggerInput {
    pub trigger_kind: String,
    pub run_at_utc: Option<String>,
    pub interval_seconds: Option<u32>,
    pub event_topic: Option<String>,
    pub event_source_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateScheduleRequest {
    pub created_by_user_id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub trigger: TriggerInput,
    pub misfire_policy: String,
    pub overlap_policy: String,
    pub debounce_seconds: Option<u32>,
    pub max_runs: Option<u32>,
    pub expires_minutes: u32,
    pub plan_template: wrapper_runtime::CreateRuntimePlanRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleReferenceRequest {
    pub schedule_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordSafeEventRequest {
    pub topic: String,
    pub source_type: String,
    pub source_id: String,
    pub event_key: String,
    #[serde(default = "empty_object")]
    pub safe_metadata: Value,
    pub occurred_at_utc: Option<String>,
}

#[derive(Debug, Clone)]
struct AuthorityCapture {
    agent_revision: i64,
    assignment_id: String,
    assignment_revision: i64,
    wrapper_id: String,
    connection_authority_revision: i64,
    bindings: Vec<BindingCapture>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BindingCapture {
    sequence_number: usize,
    capability_key: String,
    operation: String,
    action_type: String,
    binding_id: String,
    binding_grant_revision: i64,
    grant_id: String,
    grant_revision: i64,
    policy_id: String,
    policy_revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AuthorityDocument {
    schema: String,
    agent_id: String,
    agent_revision: i64,
    assignment_id: String,
    assignment_revision: i64,
    wrapper_id: String,
    connection_id: String,
    connection_authority_revision: i64,
    bindings: Vec<BindingCapture>,
}

#[derive(Debug, Clone)]
struct ScheduleRecord {
    schedule_id: String,
    agent_id: String,
    agent_revision: i64,
    assignment_id: String,
    assignment_revision: i64,
    wrapper_id: String,
    connection_id: String,
    connection_authority_revision: i64,
    state: String,
    trigger_kind: String,
    run_at_utc: Option<String>,
    interval_seconds: Option<i64>,
    event_topic: Option<String>,
    event_source_id: Option<String>,
    misfire_policy: String,
    overlap_policy: String,
    debounce_seconds: i64,
    max_runs: i64,
    run_count: i64,
    template_hash: String,
    authority_json: String,
    authority_hash: String,
    next_fire_at_utc: Option<String>,
    last_fired_at_utc: Option<String>,
    expires_at_utc: String,
}

#[derive(Debug, Clone)]
struct RunRecord {
    run_id: String,
    trigger_kind: String,
    trigger_token: String,
    event_id: Option<String>,
    authority_hash: String,
    template_hash: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    reconcile_interrupted_runs(connection)?;
    reconcile_schedules(connection)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "authorized scheduling migration is not registered exactly once"
    );
    for table in [
        "agent_schedule_definitions",
        "agent_schedule_private_templates",
        "agent_schedule_event_inbox",
        "agent_schedule_cursors",
        "agent_schedule_runs",
        "agent_schedule_receipts",
        "agent_schedule_audit_events",
        "agent_scheduler_state",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let missing_templates: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_schedule_definitions s LEFT JOIN agent_schedule_private_templates t ON t.schedule_id=s.schedule_id WHERE t.schedule_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        missing_templates == 0,
        "agent schedules are missing private plan templates"
    );
    let incomplete_terminal_runs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_schedule_runs r LEFT JOIN agent_schedule_receipts x ON x.run_id=r.run_id WHERE r.state IN ('completed','skipped','failed','interrupted') AND x.run_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        incomplete_terminal_runs == 0,
        "terminal schedule runs are missing immutable receipts"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    reconcile_schedules(connection)?;
    for (table, limit, message) in [
        (
            "agent_schedule_definitions",
            MAX_SCHEDULES,
            "agent schedule retention requires archival",
        ),
        (
            "agent_schedule_runs",
            MAX_RUNS,
            "agent schedule run retention requires archival",
        ),
        (
            "agent_schedule_event_inbox",
            MAX_EVENTS,
            "agent schedule event retention requires archival",
        ),
        (
            "agent_schedule_receipts",
            MAX_RECEIPTS,
            "agent schedule receipt retention requires archival",
        ),
        (
            "agent_schedule_audit_events",
            MAX_AUDIT_EVENTS,
            "agent schedule audit retention requires archival",
        ),
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let count: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
        ensure!(count <= limit, "{message}");
    }
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/agent-schedules", get(snapshot_handler))
        .route("/v1/agent-schedules/create", post(create_schedule_handler))
        .route("/v1/agent-schedules/pause", post(pause_schedule_handler))
        .route("/v1/agent-schedules/resume", post(resume_schedule_handler))
        .route("/v1/agent-schedules/cancel", post(cancel_schedule_handler))
        .route(
            "/v1/agent-schedules/events/record",
            post(record_event_handler),
        )
        .route("/v1/agent-schedules/run-once", post(run_once_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now() + StdDuration::from_secs(5);
    let mut interval = tokio::time::interval_at(start, StdDuration::from_secs(5));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let cycle_state = state.clone();
                match tokio::task::spawn_blocking(move || process_cycle(&cycle_state)).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => warn!(?error, "authorized schedule cycle failed"),
                    Err(error) => error!(?error, "authorized schedule task failed"),
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

async fn snapshot_handler(State(state): State<Arc<AppState>>) -> ApiResult<ScheduleSnapshot> {
    run_blocking(move || snapshot(&state), "agent_schedule_snapshot_failed").await
}

async fn create_schedule_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateScheduleRequest>,
) -> ApiResult<ScheduleSnapshot> {
    run_blocking(
        move || {
            create_schedule(&state, request)?;
            snapshot(&state)
        },
        "agent_schedule_create_failed",
    )
    .await
}

async fn pause_schedule_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScheduleReferenceRequest>,
) -> ApiResult<ScheduleSnapshot> {
    run_blocking(
        move || {
            pause_schedule(&state, request)?;
            snapshot(&state)
        },
        "agent_schedule_pause_failed",
    )
    .await
}

async fn resume_schedule_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScheduleReferenceRequest>,
) -> ApiResult<ScheduleSnapshot> {
    run_blocking(
        move || {
            resume_schedule(&state, request)?;
            snapshot(&state)
        },
        "agent_schedule_resume_failed",
    )
    .await
}

async fn cancel_schedule_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ScheduleReferenceRequest>,
) -> ApiResult<ScheduleSnapshot> {
    run_blocking(
        move || {
            cancel_schedule(&state, request)?;
            snapshot(&state)
        },
        "agent_schedule_cancel_failed",
    )
    .await
}

async fn record_event_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RecordSafeEventRequest>,
) -> ApiResult<ScheduleSnapshot> {
    run_blocking(
        move || {
            record_safe_event(&state, request)?;
            snapshot(&state)
        },
        "agent_schedule_event_record_failed",
    )
    .await
}

async fn run_once_handler(State(state): State<Arc<AppState>>) -> ApiResult<ScheduleSnapshot> {
    run_blocking(
        move || {
            process_cycle(&state)?;
            snapshot(&state)
        },
        "agent_schedule_cycle_failed",
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
        .map_err(|error| api_error(code, anyhow::anyhow!("agent schedule task failed: {error}")))?
        .map(Json)
        .map_err(|error| api_error(code, error))
}

fn create_schedule(state: &AppState, mut request: CreateScheduleRequest) -> Result<String> {
    ensure!(
        !request.plan_template.steps.is_empty() && request.plan_template.steps.len() <= 32,
        "schedule plan template must contain between one and 32 steps"
    );
    ensure!(
        (1..=525_600).contains(&request.expires_minutes),
        "schedule expiration must be between one minute and one year"
    );
    let actor = bounded_text(&request.created_by_user_id, 1, 160, "created-by user ID")?;
    let title = bounded_text(&request.title, 1, 180, "schedule title")?;
    let description = bounded_text(&request.description, 0, 2000, "schedule description")?;
    let agent_id = validate_uuid(&request.plan_template.agent_id, "agent ID")?;
    let trigger_kind = validate_choice(
        &request.trigger.trigger_kind,
        &["one_time", "interval", "event"],
        "trigger kind",
    )?;
    let misfire_policy = validate_choice(
        &request.misfire_policy,
        &["skip", "fire_once", "fail"],
        "misfire policy",
    )?;
    let overlap_policy = validate_choice(
        &request.overlap_policy,
        &["skip", "queue_one"],
        "overlap policy",
    )?;
    let debounce_seconds = request.debounce_seconds.unwrap_or(0);
    ensure!(
        debounce_seconds <= 86_400,
        "schedule debounce cannot exceed one day"
    );
    let max_runs = request
        .max_runs
        .unwrap_or(if trigger_kind == "one_time" { 1 } else { 1000 });
    ensure!(
        (1..=100_000).contains(&max_runs),
        "schedule max runs is invalid"
    );

    let now = Utc::now();
    let expires_at = now + Duration::minutes(i64::from(request.expires_minutes));
    let (run_at, interval_seconds, event_topic, event_source_id, next_fire) =
        normalize_trigger(&request.trigger, &trigger_kind, now, expires_at)?;

    let connection_id = request
        .plan_template
        .steps
        .first()
        .map(|step| step.job.connection_id.clone())
        .context("schedule plan template has no connection")?;
    validate_uuid(&connection_id, "connection ID")?;
    for (index, step) in request.plan_template.steps.iter().enumerate() {
        ensure!(
            step.job.connection_id == connection_id,
            "all schedule plan steps must use one connection"
        );
        ensure!(
            step.action_type == step.tool_key,
            "schedule action type must exactly match its tool key"
        );
        ensure!(
            step.job.approval_id.is_none() && step.job.plan_hash.is_none(),
            "schedule templates cannot pre-bind approvals or runtime plan hashes"
        );
        ensure!(
            step.job.submitted_by_type == "agent"
                || step.job.submitted_by_type == "system"
                || step.job.submitted_by_type == "local_user",
            "schedule plan submitter type is invalid"
        );
        ensure!(
            !step.job.idempotency_key.trim().is_empty(),
            "schedule plan step idempotency key is required"
        );
        ensure!(
            index < 32,
            "schedule plan template exceeds the maximum step count"
        );
    }

    request.plan_template.agent_id = agent_id.clone();
    request.plan_template.requested_by_user_id = actor.clone();
    let connection = state.connection()?;
    let authority = capture_authority(
        &connection,
        &agent_id,
        &connection_id,
        &request.plan_template.steps,
    )?;
    let authority_document = AuthorityDocument {
        schema: "homeserver.agent-schedule-authority.v1".to_owned(),
        agent_id: agent_id.clone(),
        agent_revision: authority.agent_revision,
        assignment_id: authority.assignment_id.clone(),
        assignment_revision: authority.assignment_revision,
        wrapper_id: authority.wrapper_id.clone(),
        connection_id: connection_id.clone(),
        connection_authority_revision: authority.connection_authority_revision,
        bindings: authority.bindings,
    };
    let authority_json = canonical_json(&authority_document)?;
    let authority_hash = hash_text(&authority_json);
    let template_json = canonical_json(&request.plan_template)?;
    ensure!(
        template_json.len() <= MAX_TEMPLATE_BYTES,
        "schedule private plan template exceeds the size limit"
    );
    let template_hash = hash_text(&template_json);
    let schedule_id = Uuid::new_v4().to_string();
    let now_text = timestamp(now);
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO agent_schedule_definitions (schedule_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,created_by_user_id,title,description,state,trigger_kind,run_at_utc,interval_seconds,event_topic,event_source_id,misfire_policy,overlap_policy,debounce_seconds,max_runs,run_count,template_hash,authority_snapshot_json,authority_hash,next_fire_at_utc,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'active',?12,?13,?14,?15,?16,?17,?18,?19,?20,0,?21,?22,?23,?24,?25,?26,?26)",
        params![
            schedule_id,
            agent_id,
            authority_document.agent_revision,
            authority_document.assignment_id,
            authority_document.assignment_revision,
            authority_document.wrapper_id,
            connection_id,
            authority_document.connection_authority_revision,
            actor,
            title,
            description,
            trigger_kind,
            run_at,
            interval_seconds,
            event_topic,
            event_source_id,
            misfire_policy,
            overlap_policy,
            i64::from(debounce_seconds),
            i64::from(max_runs),
            template_hash,
            authority_json,
            authority_hash,
            next_fire,
            timestamp(expires_at),
            now_text
        ],
    )?;
    transaction.execute(
        "INSERT INTO agent_schedule_private_templates (schedule_id,classification,template_json,template_bytes,created_at_utc) VALUES (?1,'private',?2,?3,?4)",
        params![schedule_id, template_json, template_json.len() as i64, now_text],
    )?;
    if trigger_kind == "event" {
        let current_event_sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(event_sequence),0) FROM agent_schedule_event_inbox",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO agent_schedule_cursors (schedule_id,last_event_sequence,updated_at_utc) VALUES (?1,?2,?3)",
            params![schedule_id, current_event_sequence, now_text],
        )?;
    }
    record_audit_tx(
        &transaction,
        Some(&schedule_id),
        None,
        "agent.schedule_created",
        "success",
        "local_user",
        &actor,
        "authority_captured",
        json!({
            "trigger_kind": trigger_kind,
            "template_hash": template_hash,
            "authority_hash": authority_hash,
            "private_template_exposed": false
        }),
    )?;
    transaction.commit()?;
    Ok(schedule_id)
}

fn normalize_trigger(
    trigger: &TriggerInput,
    trigger_kind: &str,
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<NormalizedTrigger> {
    match trigger_kind {
        "one_time" => {
            let value = trigger
                .run_at_utc
                .as_deref()
                .context("one-time schedule requires run_at_utc")?;
            let run_at = parse_utc(value, "schedule run time")?;
            ensure!(run_at > now, "one-time schedule must be in the future");
            ensure!(
                run_at < expires_at,
                "one-time schedule must run before schedule expiration"
            );
            ensure!(
                trigger.interval_seconds.is_none() && trigger.event_topic.is_none(),
                "one-time schedule contains incompatible trigger fields"
            );
            let value = timestamp(run_at);
            Ok((Some(value.clone()), None, None, None, Some(value)))
        }
        "interval" => {
            let seconds = trigger
                .interval_seconds
                .context("interval schedule requires interval_seconds")?;
            ensure!(
                (60..=2_592_000).contains(&seconds),
                "interval schedule must be between one minute and 30 days"
            );
            ensure!(
                trigger.event_topic.is_none(),
                "interval schedule contains an event topic"
            );
            let first = if let Some(value) = trigger.run_at_utc.as_deref() {
                let parsed = parse_utc(value, "interval start time")?;
                ensure!(parsed > now, "interval start must be in the future");
                parsed
            } else {
                now + Duration::seconds(i64::from(seconds))
            };
            ensure!(
                first < expires_at,
                "interval schedule starts after schedule expiration"
            );
            let value = timestamp(first);
            Ok((
                trigger.run_at_utc.as_ref().map(|_| value.clone()),
                Some(i64::from(seconds)),
                None,
                None,
                Some(value),
            ))
        }
        "event" => {
            ensure!(
                trigger.run_at_utc.is_none() && trigger.interval_seconds.is_none(),
                "event schedule contains time-trigger fields"
            );
            let topic = validate_choice(
                trigger
                    .event_topic
                    .as_deref()
                    .context("event schedule requires an event topic")?,
                EVENT_TOPICS,
                "event topic",
            )?;
            let source = trigger
                .event_source_id
                .as_deref()
                .map(|value| bounded_text(value, 1, 180, "event source ID"))
                .transpose()?;
            Ok((None, None, Some(topic), source, None))
        }
        _ => bail!("unsupported schedule trigger kind"),
    }
}

fn capture_authority(
    connection: &Connection,
    agent_id: &str,
    connection_id: &str,
    steps: &[wrapper_runtime::RuntimePlanStepRequest],
) -> Result<AuthorityCapture> {
    let now = now_utc();
    let row: (i64, String, i64, String, i64) = connection.query_row(
        "SELECT a.revision,x.assignment_id,x.assignment_revision,x.wrapper_id,c.grant_revision FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.agent_id=a.agent_id JOIN wrapper_connections c ON c.connection_id=x.connection_id AND c.wrapper_id=x.wrapper_id WHERE a.agent_id=?1 AND x.connection_id=?2 AND a.state='active' AND a.expires_at_utc>?3 AND x.state='active' AND x.expires_at_utc>?3 AND c.lifecycle_state='active'",
        params![agent_id, connection_id, now],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    ensure_no_emergency_stop(connection, agent_id, &row.3, connection_id, &now)?;
    let mut bindings = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let binding: (String, i64, String, i64, String, i64) = connection.query_row(
            "SELECT b.binding_id,b.grant_revision,b.grant_id,g.grant_revision,p.policy_id,p.policy_revision FROM agent_capability_bindings b JOIN wrapper_capability_grants g ON g.grant_id=b.grant_id AND g.connection_id=?1 AND g.wrapper_id=?2 AND g.capability_key=b.capability_key JOIN agent_execution_policies p ON p.agent_id=?3 AND p.action_type=?4 WHERE b.assignment_id=?5 AND b.capability_key=?6 AND b.state='active' AND b.expires_at_utc>?7 AND g.state='active' AND g.not_before_utc<=?7 AND g.expires_at_utc>?7 AND p.state='active' AND p.not_before_utc<=?7 AND p.expires_at_utc>?7 AND EXISTS (SELECT 1 FROM json_each(b.allowed_operations_json) WHERE value=?8) AND EXISTS (SELECT 1 FROM json_each(g.allowed_operations_json) WHERE value=?8)",
            params![
                connection_id,
                row.3,
                agent_id,
                step.action_type,
                row.1,
                step.job.capability_key,
                now,
                step.job.operation
            ],
            |record| Ok((
                record.get(0)?,
                record.get(1)?,
                record.get(2)?,
                record.get(3)?,
                record.get(4)?,
                record.get(5)?,
            )),
        ).with_context(|| format!("schedule step {} lacks current agent authority", index + 1))?;
        ensure!(
            binding.1 == binding.3,
            "schedule binding grant revision is stale"
        );
        bindings.push(BindingCapture {
            sequence_number: index + 1,
            capability_key: step.job.capability_key.clone(),
            operation: step.job.operation.clone(),
            action_type: step.action_type.clone(),
            binding_id: binding.0,
            binding_grant_revision: binding.1,
            grant_id: binding.2,
            grant_revision: binding.3,
            policy_id: binding.4,
            policy_revision: binding.5,
        });
    }
    Ok(AuthorityCapture {
        agent_revision: row.0,
        assignment_id: row.1,
        assignment_revision: row.2,
        wrapper_id: row.3,
        connection_authority_revision: row.4,
        bindings,
    })
}

fn revalidate_authority(connection: &Connection, schedule: &ScheduleRecord) -> Result<()> {
    let document: AuthorityDocument = serde_json::from_str(&schedule.authority_json)
        .context("schedule authority snapshot is invalid")?;
    ensure!(
        hash_text(&schedule.authority_json) == schedule.authority_hash,
        "schedule authority snapshot hash changed"
    );
    ensure!(
        document.agent_id == schedule.agent_id
            && document.agent_revision == schedule.agent_revision
            && document.assignment_id == schedule.assignment_id
            && document.assignment_revision == schedule.assignment_revision
            && document.wrapper_id == schedule.wrapper_id
            && document.connection_id == schedule.connection_id
            && document.connection_authority_revision == schedule.connection_authority_revision,
        "schedule authority snapshot no longer matches its definition"
    );
    let now = now_utc();
    let authority_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.agent_id=a.agent_id JOIN wrapper_connections c ON c.connection_id=x.connection_id AND c.wrapper_id=x.wrapper_id WHERE a.agent_id=?1 AND a.revision=?2 AND a.state='active' AND a.expires_at_utc>?8 AND x.assignment_id=?3 AND x.assignment_revision=?4 AND x.state='active' AND x.expires_at_utc>?8 AND c.wrapper_id=?5 AND c.connection_id=?6 AND c.grant_revision=?7 AND c.lifecycle_state='active'",
        params![
            schedule.agent_id,
            schedule.agent_revision,
            schedule.assignment_id,
            schedule.assignment_revision,
            schedule.wrapper_id,
            schedule.connection_id,
            schedule.connection_authority_revision,
            now
        ],
        |row| row.get(0),
    )?;
    ensure!(
        authority_count == 1,
        "schedule agent, assignment, or connection authority changed"
    );
    ensure_no_emergency_stop(
        connection,
        &schedule.agent_id,
        &schedule.wrapper_id,
        &schedule.connection_id,
        &now,
    )?;
    for binding in &document.bindings {
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_capability_bindings b JOIN wrapper_capability_grants g ON g.grant_id=b.grant_id JOIN agent_execution_policies p ON p.policy_id=?1 WHERE b.binding_id=?2 AND b.assignment_id=?3 AND b.grant_id=?4 AND b.grant_revision=?5 AND b.capability_key=?6 AND b.state='active' AND b.expires_at_utc>?11 AND g.connection_id=?7 AND g.wrapper_id=?8 AND g.grant_revision=?9 AND g.state='active' AND g.not_before_utc<=?11 AND g.expires_at_utc>?11 AND p.agent_id=?12 AND p.policy_revision=?10 AND p.action_type=?13 AND p.state='active' AND p.not_before_utc<=?11 AND p.expires_at_utc>?11 AND EXISTS (SELECT 1 FROM json_each(b.allowed_operations_json) WHERE value=?14) AND EXISTS (SELECT 1 FROM json_each(g.allowed_operations_json) WHERE value=?14)",
            params![
                binding.policy_id,
                binding.binding_id,
                schedule.assignment_id,
                binding.grant_id,
                binding.binding_grant_revision,
                binding.capability_key,
                schedule.connection_id,
                schedule.wrapper_id,
                binding.grant_revision,
                binding.policy_revision,
                now,
                schedule.agent_id,
                binding.action_type,
                binding.operation
            ],
            |row| row.get(0),
        )?;
        ensure!(
            count == 1,
            "schedule capability binding, grant, or execution policy changed"
        );
    }
    Ok(())
}

fn ensure_no_emergency_stop(
    connection: &Connection,
    agent_id: &str,
    wrapper_id: &str,
    connection_id: &str,
    now: &str,
) -> Result<()> {
    let active_stops: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_emergency_stops WHERE state='active' AND (expires_at_utc IS NULL OR expires_at_utc>?4) AND (scope_type='global' OR (scope_type='agent' AND agent_id=?1) OR (scope_type='wrapper' AND wrapper_id=?2) OR (scope_type='connection' AND connection_id=?3))",
        params![agent_id, wrapper_id, connection_id, now],
        |row| row.get(0),
    )?;
    ensure!(
        active_stops == 0,
        "schedule authority is blocked by an active emergency stop"
    );
    Ok(())
}

fn pause_schedule(state: &AppState, request: ScheduleReferenceRequest) -> Result<()> {
    mutate_schedule_state(
        state,
        request,
        "PAUSE SCHEDULE",
        "active",
        "paused",
        "schedule_paused",
    )
}

fn resume_schedule(state: &AppState, request: ScheduleReferenceRequest) -> Result<()> {
    let schedule_id = validate_uuid(&request.schedule_id, "schedule ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 500, "resume reason")?;
    ensure!(
        request.confirmation == format!("RESUME SCHEDULE {schedule_id}"),
        "schedule resume confirmation is invalid"
    );
    let connection = state.connection()?;
    let schedule = read_schedule(&connection, &schedule_id)?;
    ensure!(schedule.state == "paused", "schedule is not paused");
    revalidate_authority(&connection, &schedule)?;
    let now = Utc::now();
    let next_fire = match schedule.trigger_kind.as_str() {
        "one_time" => {
            let run_at = schedule
                .run_at_utc
                .as_deref()
                .context("one-time schedule is missing run time")?;
            Some(if parse_utc(run_at, "schedule run time")? > now {
                run_at.to_owned()
            } else {
                timestamp(now)
            })
        }
        "interval" => Some(timestamp(
            now + Duration::seconds(schedule.interval_seconds.context("interval missing")?),
        )),
        "event" => None,
        _ => bail!("unsupported schedule trigger kind"),
    };
    let now_text = timestamp(now);
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE agent_schedule_definitions SET state='active',next_fire_at_utc=?1,failure_code=NULL,updated_at_utc=?2 WHERE schedule_id=?3 AND state='paused'",
        params![next_fire, now_text, schedule_id],
    )?;
    record_audit_tx(
        &transaction,
        Some(&schedule_id),
        None,
        "agent.schedule_resumed",
        "success",
        "local_user",
        &actor,
        "authority_revalidated",
        json!({"reason": reason}),
    )?;
    transaction.commit()?;
    Ok(())
}

fn cancel_schedule(state: &AppState, request: ScheduleReferenceRequest) -> Result<()> {
    let schedule_id = validate_uuid(&request.schedule_id, "schedule ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 500, "cancellation reason")?;
    ensure!(
        request.confirmation == format!("CANCEL SCHEDULE {schedule_id}"),
        "schedule cancellation confirmation is invalid"
    );
    let connection = state.connection()?;
    let current: String = connection.query_row(
        "SELECT state FROM agent_schedule_definitions WHERE schedule_id=?1",
        params![schedule_id],
        |row| row.get(0),
    )?;
    ensure!(
        matches!(current.as_str(), "active" | "paused" | "failed"),
        "schedule is not cancellable"
    );
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE agent_schedule_definitions SET state='cancelled',next_fire_at_utc=NULL,failure_code='cancelled_by_authority',completed_at_utc=?1,updated_at_utc=?1 WHERE schedule_id=?2",
        params![now, schedule_id],
    )?;
    transaction.execute(
        "UPDATE agent_schedule_runs SET state='failed',outcome='failed',result_code='schedule_cancelled',failure_code='cancelled_by_authority',completed_at_utc=?1 WHERE schedule_id=?2 AND state='queued'",
        params![now, schedule_id],
    )?;
    finalize_unreceipted_terminal_runs_tx(&transaction, &schedule_id)?;
    record_audit_tx(
        &transaction,
        Some(&schedule_id),
        None,
        "agent.schedule_cancelled",
        "warning",
        "local_user",
        &actor,
        "cancelled_by_authority",
        json!({"reason": reason}),
    )?;
    transaction.commit()?;
    Ok(())
}

fn mutate_schedule_state(
    state: &AppState,
    request: ScheduleReferenceRequest,
    confirmation_prefix: &str,
    required_state: &str,
    next_state: &str,
    event_type: &str,
) -> Result<()> {
    let schedule_id = validate_uuid(&request.schedule_id, "schedule ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&request.reason, 1, 500, "state-change reason")?;
    ensure!(
        request.confirmation == format!("{confirmation_prefix} {schedule_id}"),
        "schedule state-change confirmation is invalid"
    );
    let connection = state.connection()?;
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE agent_schedule_definitions SET state=?1,next_fire_at_utc=CASE WHEN ?1='paused' THEN NULL ELSE next_fire_at_utc END,updated_at_utc=?2 WHERE schedule_id=?3 AND state=?4",
        params![next_state, now, schedule_id, required_state],
    )?;
    ensure!(changed == 1, "schedule state does not allow this operation");
    record_audit_tx(
        &transaction,
        Some(&schedule_id),
        None,
        event_type,
        "success",
        "local_user",
        &actor,
        event_type,
        json!({"reason": reason}),
    )?;
    transaction.commit()?;
    Ok(())
}

fn record_safe_event(state: &AppState, request: RecordSafeEventRequest) -> Result<String> {
    let topic = validate_choice(&request.topic, EVENT_TOPICS, "event topic")?;
    let source_type = validate_choice(
        &request.source_type,
        &["wrapper", "runtime", "orchestration", "cloud", "system"],
        "event source type",
    )?;
    ensure!(
        source_type == expected_source_type(&topic)?,
        "safe event source type does not match its topic"
    );
    let source_id = bounded_text(&request.source_id, 1, 180, "event source ID")?;
    let event_key = bounded_text(&request.event_key, 1, 240, "event key")?;
    ensure_safe_metadata(&topic, &request.safe_metadata)?;
    let metadata_json = canonical_json(&request.safe_metadata)?;
    ensure!(
        metadata_json.len() <= MAX_SAFE_EVENT_BYTES,
        "safe event metadata exceeds the size limit"
    );
    let occurred_at = request
        .occurred_at_utc
        .as_deref()
        .map(|value| parse_utc(value, "event occurrence time"))
        .transpose()?
        .unwrap_or_else(Utc::now);
    ensure!(
        occurred_at <= Utc::now() + Duration::minutes(5),
        "safe event occurrence time is too far in the future"
    );
    let document = json!({
        "schema": "homeserver.agent-schedule-event.v1",
        "topic": topic,
        "source_type": source_type,
        "source_id": source_id,
        "event_key": event_key,
        "safe_metadata": request.safe_metadata,
        "occurred_at_utc": timestamp(occurred_at)
    });
    let payload_hash = hash_json(&document)?;
    let event_id = Uuid::new_v4().to_string();
    let connection = state.connection()?;
    let inserted = connection.execute(
        "INSERT OR IGNORE INTO agent_schedule_event_inbox (event_id,topic,source_type,source_id,event_key,safe_metadata_json,payload_hash,occurred_at_utc,received_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            event_id,
            topic,
            source_type,
            source_id,
            event_key,
            metadata_json,
            payload_hash,
            timestamp(occurred_at),
            now_utc()
        ],
    )?;
    if inserted == 0 {
        let existing: String = connection.query_row(
            "SELECT event_id FROM agent_schedule_event_inbox WHERE event_key=?1 AND payload_hash=?2",
            params![event_key, payload_hash],
            |row| row.get(0),
        ).context("safe event key was reused with different metadata")?;
        return Ok(existing);
    }
    record_audit(
        &connection,
        None,
        None,
        "agent.schedule_event_recorded",
        "success",
        "event_source",
        &source_id,
        "safe_metadata_only",
        json!({
            "event_id": event_id,
            "topic": topic,
            "payload_hash": payload_hash,
            "private_payload_exposed": false
        }),
    )?;
    Ok(event_id)
}

fn process_cycle(state: &AppState) -> Result<usize> {
    {
        let connection = state.connection()?;
        reconcile_schedules(&connection)?;
        maintain_history(&connection)?;
        queue_due_time_runs(&connection)?;
        queue_event_runs(&connection)?;
    }
    let run_ids = {
        let connection = state.connection()?;
        let mut statement = connection.prepare(
            "SELECT run_id FROM agent_schedule_runs WHERE state='queued' ORDER BY created_at_utc,run_id LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![MAX_RUNS_PER_CYCLE as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut completed = 0usize;
    let mut last_error: Option<String> = None;
    for run_id in run_ids {
        match process_queued_run(state, &run_id) {
            Ok(()) => completed += 1,
            Err(error) => {
                last_error = Some(hash_text(&error.to_string()));
                warn!(%run_id, ?error, "scheduled plan creation failed");
            }
        }
    }
    let connection = state.connection()?;
    connection.execute(
        "UPDATE agent_scheduler_state SET state=?1,last_cycle_at_utc=?2,last_error_code=?3,updated_at_utc=?2 WHERE singleton_id=1",
        params![
            if last_error.is_some() { "degraded" } else { "active" },
            now_utc(),
            last_error
        ],
    )?;
    Ok(completed)
}

fn queue_due_time_runs(connection: &Connection) -> Result<()> {
    let due = {
        let mut statement = connection.prepare(
            "SELECT schedule_id FROM agent_schedule_definitions WHERE state='active' AND trigger_kind IN ('one_time','interval') AND next_fire_at_utc IS NOT NULL AND next_fire_at_utc<=?1 ORDER BY next_fire_at_utc,schedule_id LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![now_utc(), MAX_RUNS_PER_CYCLE as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for schedule_id in due {
        queue_time_trigger(connection, &schedule_id)?;
    }
    Ok(())
}

fn queue_time_trigger(connection: &Connection, schedule_id: &str) -> Result<()> {
    let schedule = read_schedule(connection, schedule_id)?;
    if schedule.run_count >= schedule.max_runs {
        complete_schedule(connection, schedule_id, "max_runs_reached")?;
        return Ok(());
    }
    let scheduled_for = schedule
        .next_fire_at_utc
        .clone()
        .context("due schedule is missing next fire time")?;
    let scheduled_time = parse_utc(&scheduled_for, "scheduled fire time")?;
    let now = Utc::now();
    let grace_seconds = schedule.interval_seconds.unwrap_or(60).max(60);
    let misfired = now.signed_duration_since(scheduled_time).num_seconds() > grace_seconds;
    if misfired && schedule.misfire_policy == "fail" {
        create_terminal_run(
            connection,
            &schedule,
            None,
            &scheduled_for,
            "failed",
            "misfire_failed",
            Some("schedule_misfire"),
        )?;
        fail_schedule(connection, schedule_id, "schedule_misfire")?;
        return Ok(());
    }
    if misfired && schedule.misfire_policy == "skip" {
        create_terminal_run(
            connection,
            &schedule,
            None,
            &scheduled_for,
            "skipped",
            "misfire_skipped",
            None,
        )?;
        if schedule.trigger_kind == "one_time" {
            complete_schedule(connection, &schedule.schedule_id, "misfire_skipped")?;
        } else {
            advance_time_schedule(connection, &schedule, now)?;
        }
        return Ok(());
    }
    if handle_overlap(connection, &schedule, None, &scheduled_for)? {
        if schedule.trigger_kind == "one_time" {
            complete_schedule(connection, &schedule.schedule_id, "overlap_skipped")?;
        } else {
            advance_time_schedule(connection, &schedule, now)?;
        }
        return Ok(());
    }
    create_queued_run(connection, &schedule, None, &scheduled_for)?;
    advance_time_schedule(connection, &schedule, now)
}

fn advance_time_schedule(
    connection: &Connection,
    schedule: &ScheduleRecord,
    now: DateTime<Utc>,
) -> Result<()> {
    if schedule.trigger_kind == "one_time" {
        connection.execute(
            "UPDATE agent_schedule_definitions SET next_fire_at_utc=NULL,updated_at_utc=?1 WHERE schedule_id=?2",
            params![timestamp(now), schedule.schedule_id],
        )?;
        return Ok(());
    }
    let seconds = schedule
        .interval_seconds
        .context("interval schedule is missing interval")?;
    let current = parse_utc(
        schedule
            .next_fire_at_utc
            .as_deref()
            .context("interval schedule is missing next fire time")?,
        "interval fire time",
    )?;
    let next = next_interval_after(current, seconds, now);
    connection.execute(
        "UPDATE agent_schedule_definitions SET next_fire_at_utc=?1,updated_at_utc=?2 WHERE schedule_id=?3",
        params![timestamp(next), timestamp(now), schedule.schedule_id],
    )?;
    Ok(())
}

fn queue_event_runs(connection: &Connection) -> Result<()> {
    let schedule_ids = {
        let mut statement = connection.prepare(
            "SELECT schedule_id FROM agent_schedule_definitions WHERE state='active' AND trigger_kind='event' ORDER BY updated_at_utc,schedule_id LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![MAX_RUNS_PER_CYCLE as i64], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for schedule_id in schedule_ids {
        let schedule = read_schedule(connection, &schedule_id)?;
        if schedule.run_count >= schedule.max_runs {
            complete_schedule(connection, &schedule_id, "max_runs_reached")?;
            continue;
        }
        let cursor: i64 = connection.query_row(
            "SELECT last_event_sequence FROM agent_schedule_cursors WHERE schedule_id=?1",
            params![schedule_id],
            |row| row.get(0),
        )?;
        let event = connection
            .query_row(
                "SELECT event_sequence,event_id,occurred_at_utc FROM agent_schedule_event_inbox WHERE event_sequence>?1 AND topic=?2 AND (?3 IS NULL OR source_id=?3) ORDER BY event_sequence LIMIT 1",
                params![cursor, schedule.event_topic, schedule.event_source_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()?;
        let Some((sequence, event_id, occurred_at)) = event else {
            continue;
        };
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE agent_schedule_cursors SET last_event_sequence=?1,updated_at_utc=?2 WHERE schedule_id=?3",
            params![sequence, now_utc(), schedule_id],
        )?;
        let debounced = if let Some(last) = schedule.last_fired_at_utc.as_deref() {
            let elapsed = parse_utc(&occurred_at, "event occurrence time")?
                .signed_duration_since(parse_utc(last, "last schedule fire time")?)
                .num_seconds();
            elapsed >= 0 && elapsed < schedule.debounce_seconds
        } else {
            false
        };
        if debounced {
            create_terminal_run_tx(
                &transaction,
                &schedule,
                Some(&event_id),
                &occurred_at,
                "skipped",
                "event_debounced",
                None,
            )?;
            transaction.commit()?;
            continue;
        }
        if handle_overlap_tx(&transaction, &schedule, Some(&event_id), &occurred_at)? {
            transaction.commit()?;
            continue;
        }
        create_queued_run_tx(&transaction, &schedule, Some(&event_id), &occurred_at)?;
        transaction.commit()?;
    }
    Ok(())
}

fn handle_overlap(
    connection: &Connection,
    schedule: &ScheduleRecord,
    event_id: Option<&str>,
    scheduled_for: &str,
) -> Result<bool> {
    let transaction = connection.unchecked_transaction()?;
    let result = handle_overlap_tx(&transaction, schedule, event_id, scheduled_for)?;
    transaction.commit()?;
    Ok(result)
}

fn handle_overlap_tx(
    transaction: &Transaction<'_>,
    schedule: &ScheduleRecord,
    event_id: Option<&str>,
    scheduled_for: &str,
) -> Result<bool> {
    let creating: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_schedule_runs WHERE schedule_id=?1 AND state='creating_plan'",
        params![schedule.schedule_id],
        |row| row.get(0),
    )?;
    let queued: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_schedule_runs WHERE schedule_id=?1 AND state='queued'",
        params![schedule.schedule_id],
        |row| row.get(0),
    )?;
    if creating == 0 && queued == 0 {
        return Ok(false);
    }
    if schedule.overlap_policy == "queue_one" && creating > 0 && queued == 0 {
        return Ok(false);
    }
    let code = if schedule.overlap_policy == "queue_one" {
        "overlap_coalesced"
    } else {
        "overlap_skipped"
    };
    create_terminal_run_tx(
        transaction,
        schedule,
        event_id,
        scheduled_for,
        "skipped",
        code,
        None,
    )?;
    Ok(true)
}

fn create_queued_run(
    connection: &Connection,
    schedule: &ScheduleRecord,
    event_id: Option<&str>,
    scheduled_for: &str,
) -> Result<String> {
    let transaction = connection.unchecked_transaction()?;
    let run_id = create_queued_run_tx(&transaction, schedule, event_id, scheduled_for)?;
    transaction.commit()?;
    Ok(run_id)
}

fn create_queued_run_tx(
    transaction: &Transaction<'_>,
    schedule: &ScheduleRecord,
    event_id: Option<&str>,
    scheduled_for: &str,
) -> Result<String> {
    let trigger_token = trigger_token(&schedule.schedule_id, scheduled_for, event_id)?;
    if let Some(existing) = transaction
        .query_row(
            "SELECT run_id FROM agent_schedule_runs WHERE trigger_token=?1",
            params![trigger_token],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(existing);
    }
    let run_id = Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO agent_schedule_runs (run_id,schedule_id,trigger_kind,trigger_token,event_id,scheduled_for_utc,state,authority_hash,template_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,'queued',?7,?8,?9)",
        params![
            run_id,
            schedule.schedule_id,
            schedule.trigger_kind,
            trigger_token,
            event_id,
            scheduled_for,
            schedule.authority_hash,
            schedule.template_hash,
            now_utc()
        ],
    )?;
    record_audit_tx(
        transaction,
        Some(&schedule.schedule_id),
        Some(&run_id),
        "agent.schedule_run_queued",
        "success",
        "scheduler",
        SCHEDULER_ACTOR,
        "trigger_deduplicated",
        json!({
            "trigger_kind": schedule.trigger_kind,
            "trigger_token": trigger_token,
            "event_id": event_id,
            "private_template_exposed": false
        }),
    )?;
    Ok(run_id)
}

fn create_terminal_run(
    connection: &Connection,
    schedule: &ScheduleRecord,
    event_id: Option<&str>,
    scheduled_for: &str,
    outcome: &str,
    result_code: &str,
    failure_code: Option<&str>,
) -> Result<String> {
    let transaction = connection.unchecked_transaction()?;
    let run_id = create_terminal_run_tx(
        &transaction,
        schedule,
        event_id,
        scheduled_for,
        outcome,
        result_code,
        failure_code,
    )?;
    transaction.commit()?;
    Ok(run_id)
}

fn create_terminal_run_tx(
    transaction: &Transaction<'_>,
    schedule: &ScheduleRecord,
    event_id: Option<&str>,
    scheduled_for: &str,
    outcome: &str,
    result_code: &str,
    failure_code: Option<&str>,
) -> Result<String> {
    ensure!(
        matches!(outcome, "skipped" | "failed" | "interrupted"),
        "terminal schedule outcome is invalid"
    );
    let trigger = trigger_token(&schedule.schedule_id, scheduled_for, event_id)?;
    if let Some(existing) = transaction
        .query_row(
            "SELECT run_id FROM agent_schedule_runs WHERE trigger_token=?1",
            params![trigger],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(existing);
    }
    let run_id = Uuid::new_v4().to_string();
    let now = now_utc();
    transaction.execute(
        "INSERT INTO agent_schedule_runs (run_id,schedule_id,trigger_kind,trigger_token,event_id,scheduled_for_utc,state,authority_hash,template_hash,outcome,result_code,failure_code,created_at_utc,completed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?7,?10,?11,?12,?12)",
        params![
            run_id,
            schedule.schedule_id,
            schedule.trigger_kind,
            trigger,
            event_id,
            scheduled_for,
            outcome,
            schedule.authority_hash,
            schedule.template_hash,
            result_code,
            failure_code,
            now
        ],
    )?;
    write_receipt_tx(
        transaction,
        schedule,
        &RunRecord {
            run_id: run_id.clone(),
            trigger_kind: schedule.trigger_kind.clone(),
            trigger_token: trigger,
            event_id: event_id.map(str::to_owned),
            authority_hash: schedule.authority_hash.clone(),
            template_hash: schedule.template_hash.clone(),
        },
        outcome,
        result_code,
        None,
        None,
    )?;
    Ok(run_id)
}

fn process_queued_run(state: &AppState, run_id: &str) -> Result<()> {
    let run_id = validate_uuid(run_id, "schedule run ID")?;
    let (schedule, run, template_json) = {
        let connection = state.connection()?;
        let transaction = connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE agent_schedule_runs SET state='creating_plan',started_at_utc=?1 WHERE run_id=?2 AND state='queued'",
            params![now_utc(), run_id],
        )?;
        if changed == 0 {
            transaction.commit()?;
            return Ok(());
        }
        let schedule_id: String = transaction.query_row(
            "SELECT schedule_id FROM agent_schedule_runs WHERE run_id=?1",
            params![run_id],
            |row| row.get(0),
        )?;
        let schedule = read_schedule_tx(&transaction, &schedule_id)?;
        let run = read_run_tx(&transaction, &run_id)?;
        let template_json: String = transaction.query_row(
            "SELECT template_json FROM agent_schedule_private_templates WHERE schedule_id=?1",
            params![schedule_id],
            |row| row.get(0),
        )?;
        transaction.commit()?;
        (schedule, run, template_json)
    };
    let result = (|| -> Result<(String, String)> {
        let connection = state.connection()?;
        ensure!(schedule.state == "active", "schedule is no longer active");
        ensure!(
            parse_utc(&schedule.expires_at_utc, "schedule expiration")? > Utc::now(),
            "schedule expired before plan creation"
        );
        ensure!(
            hash_text(&template_json) == schedule.template_hash,
            "schedule private plan template hash changed"
        );
        revalidate_authority(&connection, &schedule)?;
        let mut plan: wrapper_runtime::CreateRuntimePlanRequest =
            serde_json::from_str(&template_json).context("schedule plan template is invalid")?;
        plan.requested_by_user_id = format!("agent_scheduler:{run_id}");
        plan.expires_minutes = plan
            .expires_minutes
            .min(remaining_minutes(&schedule.expires_at_utc)?.clamp(1, 10_080) as u32);
        for (index, step) in plan.steps.iter_mut().enumerate() {
            step.job.idempotency_key = format!(
                "schedule-{}",
                hash_text(&format!(
                    "{}:{}:{}",
                    schedule.schedule_id,
                    run.trigger_token,
                    index + 1
                ))
            );
            step.job.submitted_by_type = "agent".to_owned();
            step.job.submitted_by_id = schedule.agent_id.clone();
            step.job.correlation_id = None;
            step.job.causation_id = None;
            step.job.available_at_utc = None;
            step.job.approval_id = None;
            step.job.plan_hash = None;
        }
        let plan_id = wrapper_runtime::create_plan(state, plan)?;
        let post_creation_authority = (|| -> Result<()> {
            let connection = state.connection()?;
            let current_schedule = read_schedule(&connection, &schedule.schedule_id)?;
            ensure!(
                current_schedule.state == "active",
                "schedule changed during runtime plan creation"
            );
            revalidate_authority(&connection, &current_schedule)
        })();
        if let Err(error) = post_creation_authority {
            wrapper_runtime::cancel_plan_as_system(
                state,
                wrapper_runtime::RuntimePlanReferenceRequest {
                    plan_id: plan_id.clone(),
                    actor_user_id: SCHEDULER_ACTOR.to_owned(),
                    confirmation: format!("CANCEL PLAN {plan_id}"),
                    reason: "schedule authority changed during plan creation".to_owned(),
                },
            )?;
            return Err(error).context("schedule authority changed during plan creation");
        }
        let connection = state.connection()?;
        let plan_hash: String = connection.query_row(
            "SELECT plan_hash FROM agent_runtime_plans WHERE plan_id=?1",
            params![plan_id],
            |row| row.get(0),
        )?;
        Ok((plan_id, plan_hash))
    })();
    match result {
        Ok((plan_id, plan_hash)) => {
            finalize_run_success(state, &schedule, &run, &plan_id, &plan_hash)
        }
        Err(error) => {
            finalize_run_failure(
                state,
                &schedule,
                &run,
                "schedule_authority_or_plan_failed",
                &error,
            )?;
            Err(error)
        }
    }
}

fn finalize_run_success(
    state: &AppState,
    schedule: &ScheduleRecord,
    run: &RunRecord,
    plan_id: &str,
    plan_hash: &str,
) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let now = now_utc();
    transaction.execute(
        "UPDATE agent_schedule_runs SET state='completed',plan_id=?1,plan_hash=?2,outcome='completed',result_code='runtime_plan_created',completed_at_utc=?3 WHERE run_id=?4 AND state='creating_plan'",
        params![plan_id, plan_hash, now, run.run_id],
    )?;
    write_receipt_tx(
        &transaction,
        schedule,
        run,
        "completed",
        "runtime_plan_created",
        Some(plan_id),
        Some(plan_hash),
    )?;
    transaction.execute(
        "UPDATE agent_schedule_definitions SET run_count=run_count+1,last_fired_at_utc=?1,state=CASE WHEN trigger_kind='one_time' OR run_count+1>=max_runs THEN 'completed' ELSE state END,completed_at_utc=CASE WHEN trigger_kind='one_time' OR run_count+1>=max_runs THEN ?1 ELSE completed_at_utc END,updated_at_utc=?1 WHERE schedule_id=?2",
        params![now, schedule.schedule_id],
    )?;
    record_audit_tx(
        &transaction,
        Some(&schedule.schedule_id),
        Some(&run.run_id),
        "agent.schedule_plan_created",
        "success",
        "scheduler",
        SCHEDULER_ACTOR,
        "phase17_plan_created",
        json!({
            "plan_id": plan_id,
            "plan_hash": plan_hash,
            "phase17_runtime_required": true,
            "phase18_supervision_required": true
        }),
    )?;
    transaction.commit()?;
    Ok(())
}

fn finalize_run_failure(
    state: &AppState,
    schedule: &ScheduleRecord,
    run: &RunRecord,
    failure_code: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    let now = now_utc();
    transaction.execute(
        "UPDATE agent_schedule_runs SET state='failed',outcome='failed',result_code='schedule_plan_not_created',failure_code=?1,completed_at_utc=?2 WHERE run_id=?3 AND state='creating_plan'",
        params![failure_code, now, run.run_id],
    )?;
    write_receipt_tx(
        &transaction,
        schedule,
        run,
        "failed",
        "schedule_plan_not_created",
        None,
        None,
    )?;
    transaction.execute(
        "UPDATE agent_schedule_definitions SET state='failed',next_fire_at_utc=NULL,failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE schedule_id=?3 AND state='active'",
        params![failure_code, now, schedule.schedule_id],
    )?;
    record_audit_tx(
        &transaction,
        Some(&schedule.schedule_id),
        Some(&run.run_id),
        "agent.schedule_run_failed",
        "error",
        "scheduler",
        SCHEDULER_ACTOR,
        failure_code,
        json!({"error_hash": hash_text(&error.to_string())}),
    )?;
    transaction.commit()?;
    Ok(())
}

fn write_receipt_tx(
    transaction: &Transaction<'_>,
    schedule: &ScheduleRecord,
    run: &RunRecord,
    outcome: &str,
    result_code: &str,
    plan_id: Option<&str>,
    plan_hash: Option<&str>,
) -> Result<String> {
    if let Some(existing) = transaction
        .query_row(
            "SELECT receipt_id FROM agent_schedule_receipts WHERE run_id=?1",
            params![run.run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(existing);
    }
    let receipt_id = Uuid::new_v4().to_string();
    let completed_at = now_utc();
    let document = json!({
        "schema": "homeserver.agent-schedule-receipt.v1",
        "receipt_id": receipt_id,
        "schedule_id": schedule.schedule_id,
        "run_id": run.run_id,
        "agent_id": schedule.agent_id,
        "assignment_id": schedule.assignment_id,
        "wrapper_id": schedule.wrapper_id,
        "connection_id": schedule.connection_id,
        "trigger_kind": run.trigger_kind,
        "trigger_token": run.trigger_token,
        "event_id": run.event_id,
        "outcome": outcome,
        "result_code": result_code,
        "authority_hash": run.authority_hash,
        "template_hash": run.template_hash,
        "plan_id": plan_id,
        "plan_hash": plan_hash,
        "completed_at_utc": completed_at
    });
    let receipt_hash = hash_json(&document)?;
    transaction.execute(
        "INSERT INTO agent_schedule_receipts (receipt_id,schedule_id,run_id,agent_id,assignment_id,wrapper_id,connection_id,trigger_kind,trigger_token,event_id,outcome,result_code,authority_hash,template_hash,plan_id,plan_hash,receipt_hash,completed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
        params![
            receipt_id,
            schedule.schedule_id,
            run.run_id,
            schedule.agent_id,
            schedule.assignment_id,
            schedule.wrapper_id,
            schedule.connection_id,
            run.trigger_kind,
            run.trigger_token,
            run.event_id,
            outcome,
            result_code,
            run.authority_hash,
            run.template_hash,
            plan_id,
            plan_hash,
            receipt_hash,
            completed_at
        ],
    )?;
    Ok(receipt_id)
}

fn reconcile_interrupted_runs(connection: &Connection) -> Result<()> {
    let interrupted = {
        let mut statement = connection.prepare(
            "SELECT run_id,schedule_id FROM agent_schedule_runs WHERE state='creating_plan'",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for (run_id, schedule_id) in interrupted {
        let actor = format!("agent_scheduler:{run_id}");
        let plan = connection
            .query_row(
                "SELECT plan_id,plan_hash FROM agent_runtime_plans WHERE requested_by_user_id=?1 ORDER BY created_at_utc DESC LIMIT 1",
                params![actor],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let schedule = read_schedule(connection, &schedule_id)?;
        let run = read_run(connection, &run_id)?;
        let transaction = connection.unchecked_transaction()?;
        if let Some((plan_id, plan_hash)) = plan {
            transaction.execute(
                "UPDATE agent_schedule_runs SET state='completed',plan_id=?1,plan_hash=?2,outcome='completed',result_code='runtime_plan_recovered',completed_at_utc=?3 WHERE run_id=?4",
                params![plan_id, plan_hash, now_utc(), run_id],
            )?;
            write_receipt_tx(
                &transaction,
                &schedule,
                &run,
                "completed",
                "runtime_plan_recovered",
                Some(&plan_id),
                Some(&plan_hash),
            )?;
            transaction.execute(
                "UPDATE agent_schedule_definitions SET run_count=MIN(max_runs,run_count+1),last_fired_at_utc=?1,updated_at_utc=?1 WHERE schedule_id=?2",
                params![now_utc(), schedule_id],
            )?;
        } else {
            transaction.execute(
                "UPDATE agent_schedule_runs SET state='interrupted',outcome='interrupted',result_code='plan_creation_interrupted',failure_code='restart_interrupted',completed_at_utc=?1 WHERE run_id=?2",
                params![now_utc(), run_id],
            )?;
            write_receipt_tx(
                &transaction,
                &schedule,
                &run,
                "interrupted",
                "plan_creation_interrupted",
                None,
                None,
            )?;
            transaction.execute(
                "UPDATE agent_schedule_definitions SET state='failed',next_fire_at_utc=NULL,failure_code='restart_interrupted',completed_at_utc=?1,updated_at_utc=?1 WHERE schedule_id=?2",
                params![now_utc(), schedule_id],
            )?;
        }
        transaction.commit()?;
    }
    Ok(())
}

fn reconcile_schedules(connection: &Connection) -> Result<()> {
    let now = now_utc();
    connection.execute(
        "UPDATE agent_schedule_definitions SET state='expired',next_fire_at_utc=NULL,failure_code='schedule_expired',completed_at_utc=?1,updated_at_utc=?1 WHERE state IN ('active','paused') AND expires_at_utc<=?1",
        params![now],
    )?;
    connection.execute(
        "UPDATE agent_schedule_definitions SET state='completed',next_fire_at_utc=NULL,completed_at_utc=COALESCE(completed_at_utc,?1),updated_at_utc=?1 WHERE state='active' AND run_count>=max_runs",
        params![now],
    )?;
    Ok(())
}

fn complete_schedule(connection: &Connection, schedule_id: &str, code: &str) -> Result<()> {
    connection.execute(
        "UPDATE agent_schedule_definitions SET state='completed',next_fire_at_utc=NULL,failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE schedule_id=?3 AND state='active'",
        params![code, now_utc(), schedule_id],
    )?;
    Ok(())
}

fn fail_schedule(connection: &Connection, schedule_id: &str, code: &str) -> Result<()> {
    connection.execute(
        "UPDATE agent_schedule_definitions SET state='failed',next_fire_at_utc=NULL,failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE schedule_id=?3 AND state='active'",
        params![code, now_utc(), schedule_id],
    )?;
    Ok(())
}

fn finalize_unreceipted_terminal_runs_tx(
    transaction: &Transaction<'_>,
    schedule_id: &str,
) -> Result<()> {
    let schedule = read_schedule_tx(transaction, schedule_id)?;
    let runs = {
        let mut statement = transaction.prepare(
            "SELECT run_id FROM agent_schedule_runs r WHERE schedule_id=?1 AND state IN ('completed','skipped','failed','interrupted') AND NOT EXISTS (SELECT 1 FROM agent_schedule_receipts x WHERE x.run_id=r.run_id)",
        )?;
        let rows = statement
            .query_map(params![schedule_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for run_id in runs {
        let run = read_run_tx(transaction, &run_id)?;
        let (outcome, result_code, plan_id, plan_hash): (
            String,
            String,
            Option<String>,
            Option<String>,
        ) = transaction.query_row(
            "SELECT COALESCE(outcome,state),COALESCE(result_code,'terminal_run'),plan_id,plan_hash FROM agent_schedule_runs WHERE run_id=?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        write_receipt_tx(
            transaction,
            &schedule,
            &run,
            &outcome,
            &result_code,
            plan_id.as_deref(),
            plan_hash.as_deref(),
        )?;
    }
    Ok(())
}

fn snapshot(state: &AppState) -> Result<ScheduleSnapshot> {
    let connection = state.connection()?;
    let (scheduler_state, scheduler_revision): (String, i64) = connection.query_row(
        "SELECT state,scheduler_revision FROM agent_scheduler_state WHERE singleton_id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ScheduleSnapshot {
        schema: "homeserver.agent-schedules.v1".to_owned(),
        scheduler_state,
        scheduler_revision: scheduler_revision.max(1) as u64,
        schedules: read_schedules(&connection)?,
        runs: read_runs(&connection)?,
        events: read_events(&connection)?,
        receipts: read_receipts(&connection)?,
        allowed_event_topics: EVENT_TOPICS
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        private_templates_exposed: false,
        private_event_payloads_exposed: false,
        direct_execution_allowed: false,
        phase17_runtime_required: true,
        phase18_supervision_required: true,
    })
}

fn read_schedules(connection: &Connection) -> Result<Vec<ScheduleSummary>> {
    let mut statement = connection.prepare(
        "SELECT schedule_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,created_by_user_id,title,description,state,trigger_kind,run_at_utc,interval_seconds,event_topic,event_source_id,misfire_policy,overlap_policy,debounce_seconds,max_runs,run_count,template_hash,authority_hash,next_fire_at_utc,last_fired_at_utc,expires_at_utc,failure_code,created_at_utc,updated_at_utc,completed_at_utc FROM agent_schedule_definitions ORDER BY updated_at_utc DESC,schedule_id DESC LIMIT 250",
    )?;
    let rows = statement
        .query_map([], schedule_summary_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn schedule_summary_from_row(row: &Row<'_>) -> rusqlite::Result<ScheduleSummary> {
    Ok(ScheduleSummary {
        schedule_id: row.get(0)?,
        agent_id: row.get(1)?,
        agent_revision: positive_u64(row.get::<_, i64>(2)?),
        assignment_id: row.get(3)?,
        assignment_revision: positive_u64(row.get::<_, i64>(4)?),
        wrapper_id: row.get(5)?,
        connection_id: row.get(6)?,
        connection_authority_revision: nonnegative_u64(row.get::<_, i64>(7)?),
        created_by_user_id: row.get(8)?,
        title: row.get(9)?,
        description: row.get(10)?,
        state: row.get(11)?,
        trigger_kind: row.get(12)?,
        run_at_utc: row.get(13)?,
        interval_seconds: row
            .get::<_, Option<i64>>(14)?
            .map(|value| value.max(0) as u32),
        event_topic: row.get(15)?,
        event_source_id: row.get(16)?,
        misfire_policy: row.get(17)?,
        overlap_policy: row.get(18)?,
        debounce_seconds: row.get::<_, i64>(19)?.max(0) as u32,
        max_runs: row.get::<_, i64>(20)?.max(0) as u32,
        run_count: row.get::<_, i64>(21)?.max(0) as u32,
        template_hash: row.get(22)?,
        authority_hash: row.get(23)?,
        next_fire_at_utc: row.get(24)?,
        last_fired_at_utc: row.get(25)?,
        expires_at_utc: row.get(26)?,
        failure_code: row.get(27)?,
        created_at_utc: row.get(28)?,
        updated_at_utc: row.get(29)?,
        completed_at_utc: row.get(30)?,
    })
}

fn read_runs(connection: &Connection) -> Result<Vec<ScheduleRunSummary>> {
    let mut statement = connection.prepare(
        "SELECT run_id,schedule_id,trigger_kind,trigger_token,event_id,scheduled_for_utc,state,authority_hash,template_hash,plan_id,plan_hash,outcome,result_code,failure_code,created_at_utc,started_at_utc,completed_at_utc FROM agent_schedule_runs ORDER BY created_at_utc DESC,run_id DESC LIMIT 500",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(ScheduleRunSummary {
                run_id: row.get(0)?,
                schedule_id: row.get(1)?,
                trigger_kind: row.get(2)?,
                trigger_token: row.get(3)?,
                event_id: row.get(4)?,
                scheduled_for_utc: row.get(5)?,
                state: row.get(6)?,
                authority_hash: row.get(7)?,
                template_hash: row.get(8)?,
                plan_id: row.get(9)?,
                plan_hash: row.get(10)?,
                outcome: row.get(11)?,
                result_code: row.get(12)?,
                failure_code: row.get(13)?,
                created_at_utc: row.get(14)?,
                started_at_utc: row.get(15)?,
                completed_at_utc: row.get(16)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_events(connection: &Connection) -> Result<Vec<SafeEventSummary>> {
    let mut statement = connection.prepare(
        "SELECT event_sequence,event_id,topic,source_type,source_id,event_key,safe_metadata_json,payload_hash,occurred_at_utc,received_at_utc FROM agent_schedule_event_inbox ORDER BY event_sequence DESC LIMIT 250",
    )?;
    let rows = statement
        .query_map([], |row| {
            let metadata: String = row.get(6)?;
            Ok(SafeEventSummary {
                event_sequence: nonnegative_u64(row.get::<_, i64>(0)?),
                event_id: row.get(1)?,
                topic: row.get(2)?,
                source_type: row.get(3)?,
                source_id: row.get(4)?,
                event_key: row.get(5)?,
                safe_metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),
                payload_hash: row.get(7)?,
                occurred_at_utc: row.get(8)?,
                received_at_utc: row.get(9)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_receipts(connection: &Connection) -> Result<Vec<ScheduleReceiptSummary>> {
    let mut statement = connection.prepare(
        "SELECT receipt_id,schedule_id,run_id,agent_id,assignment_id,wrapper_id,connection_id,trigger_kind,trigger_token,event_id,outcome,result_code,authority_hash,template_hash,plan_id,plan_hash,receipt_hash,completed_at_utc FROM agent_schedule_receipts ORDER BY completed_at_utc DESC,receipt_id DESC LIMIT 500",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(ScheduleReceiptSummary {
                receipt_id: row.get(0)?,
                schedule_id: row.get(1)?,
                run_id: row.get(2)?,
                agent_id: row.get(3)?,
                assignment_id: row.get(4)?,
                wrapper_id: row.get(5)?,
                connection_id: row.get(6)?,
                trigger_kind: row.get(7)?,
                trigger_token: row.get(8)?,
                event_id: row.get(9)?,
                outcome: row.get(10)?,
                result_code: row.get(11)?,
                authority_hash: row.get(12)?,
                template_hash: row.get(13)?,
                plan_id: row.get(14)?,
                plan_hash: row.get(15)?,
                receipt_hash: row.get(16)?,
                completed_at_utc: row.get(17)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn read_schedule(connection: &Connection, schedule_id: &str) -> Result<ScheduleRecord> {
    connection
        .query_row(
            "SELECT schedule_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,state,trigger_kind,run_at_utc,interval_seconds,event_topic,event_source_id,misfire_policy,overlap_policy,debounce_seconds,max_runs,run_count,template_hash,authority_snapshot_json,authority_hash,next_fire_at_utc,last_fired_at_utc,expires_at_utc FROM agent_schedule_definitions WHERE schedule_id=?1",
            params![schedule_id],
            schedule_record_from_row,
        )
        .map_err(Into::into)
}

fn read_schedule_tx(transaction: &Transaction<'_>, schedule_id: &str) -> Result<ScheduleRecord> {
    transaction
        .query_row(
            "SELECT schedule_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,state,trigger_kind,run_at_utc,interval_seconds,event_topic,event_source_id,misfire_policy,overlap_policy,debounce_seconds,max_runs,run_count,template_hash,authority_snapshot_json,authority_hash,next_fire_at_utc,last_fired_at_utc,expires_at_utc FROM agent_schedule_definitions WHERE schedule_id=?1",
            params![schedule_id],
            schedule_record_from_row,
        )
        .map_err(Into::into)
}

fn schedule_record_from_row(row: &Row<'_>) -> rusqlite::Result<ScheduleRecord> {
    Ok(ScheduleRecord {
        schedule_id: row.get(0)?,
        agent_id: row.get(1)?,
        agent_revision: row.get(2)?,
        assignment_id: row.get(3)?,
        assignment_revision: row.get(4)?,
        wrapper_id: row.get(5)?,
        connection_id: row.get(6)?,
        connection_authority_revision: row.get(7)?,
        state: row.get(8)?,
        trigger_kind: row.get(9)?,
        run_at_utc: row.get(10)?,
        interval_seconds: row.get(11)?,
        event_topic: row.get(12)?,
        event_source_id: row.get(13)?,
        misfire_policy: row.get(14)?,
        overlap_policy: row.get(15)?,
        debounce_seconds: row.get(16)?,
        max_runs: row.get(17)?,
        run_count: row.get(18)?,
        template_hash: row.get(19)?,
        authority_json: row.get(20)?,
        authority_hash: row.get(21)?,
        next_fire_at_utc: row.get(22)?,
        last_fired_at_utc: row.get(23)?,
        expires_at_utc: row.get(24)?,
    })
}

fn read_run(connection: &Connection, run_id: &str) -> Result<RunRecord> {
    connection
        .query_row(
            "SELECT run_id,trigger_kind,trigger_token,event_id,authority_hash,template_hash FROM agent_schedule_runs WHERE run_id=?1",
            params![run_id],
            run_record_from_row,
        )
        .map_err(Into::into)
}

fn read_run_tx(transaction: &Transaction<'_>, run_id: &str) -> Result<RunRecord> {
    transaction
        .query_row(
            "SELECT run_id,trigger_kind,trigger_token,event_id,authority_hash,template_hash FROM agent_schedule_runs WHERE run_id=?1",
            params![run_id],
            run_record_from_row,
        )
        .map_err(Into::into)
}

fn run_record_from_row(row: &Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        run_id: row.get(0)?,
        trigger_kind: row.get(1)?,
        trigger_token: row.get(2)?,
        event_id: row.get(3)?,
        authority_hash: row.get(4)?,
        template_hash: row.get(5)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_audit(
    connection: &Connection,
    schedule_id: Option<&str>,
    run_id: Option<&str>,
    event_type: &str,
    outcome: &str,
    actor_type: &str,
    actor_id: &str,
    detail_code: &str,
    metadata: Value,
) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    record_audit_tx(
        &transaction,
        schedule_id,
        run_id,
        event_type,
        outcome,
        actor_type,
        actor_id,
        detail_code,
        metadata,
    )?;
    transaction.commit()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_audit_tx(
    transaction: &Transaction<'_>,
    schedule_id: Option<&str>,
    run_id: Option<&str>,
    event_type: &str,
    outcome: &str,
    actor_type: &str,
    actor_id: &str,
    detail_code: &str,
    metadata: Value,
) -> Result<()> {
    let audit_event_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let document = json!({
        "schema": "homeserver.agent-schedule-audit.v1",
        "audit_event_id": audit_event_id,
        "schedule_id": schedule_id,
        "run_id": run_id,
        "event_type": event_type,
        "outcome": outcome,
        "actor_type": actor_type,
        "actor_id": actor_id,
        "detail_code": detail_code,
        "metadata": metadata.clone(),
        "created_at_utc": now
    });
    let event_hash = hash_json(&document)?;
    transaction.execute(
        "INSERT INTO agent_schedule_audit_events (audit_event_id,schedule_id,run_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            audit_event_id,
            schedule_id,
            run_id,
            event_type,
            outcome,
            actor_type,
            actor_id,
            detail_code,
            canonical_json(&metadata)?,
            event_hash,
            now
        ],
    )?;
    Ok(())
}

fn trigger_token(schedule_id: &str, scheduled_for: &str, event_id: Option<&str>) -> Result<String> {
    hash_json(&json!({
        "schema": "homeserver.agent-schedule-trigger.v1",
        "schedule_id": schedule_id,
        "scheduled_for_utc": scheduled_for,
        "event_id": event_id
    }))
}

fn next_interval_after(
    current: DateTime<Utc>,
    interval_seconds: i64,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    if current > now {
        return current;
    }
    let elapsed = now.signed_duration_since(current).num_seconds().max(0);
    let intervals = elapsed / interval_seconds + 1;
    current + Duration::seconds(interval_seconds.saturating_mul(intervals))
}

fn remaining_minutes(expires_at: &str) -> Result<i64> {
    Ok(parse_utc(expires_at, "schedule expiration")?
        .signed_duration_since(Utc::now())
        .num_minutes()
        .max(1))
}

fn expected_source_type(topic: &str) -> Result<&'static str> {
    match topic {
        "wrapper.job.completed" => Ok("wrapper"),
        "runtime.plan.completed" => Ok("runtime"),
        "supervised.action.completed" => Ok("orchestration"),
        "cloud.sync.completed" => Ok("cloud"),
        _ => bail!("safe event topic has no source contract"),
    }
}

fn allowed_safe_event_fields(topic: &str) -> Result<&'static [&'static str]> {
    match topic {
        "wrapper.job.completed" => Ok(&[
            "job_id",
            "connection_id",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        "runtime.plan.completed" => Ok(&[
            "plan_id",
            "agent_id",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        "supervised.action.completed" => Ok(&[
            "checkpoint_id",
            "proposal_id",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        "cloud.sync.completed" => Ok(&[
            "connection_id",
            "operation_type",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        _ => bail!("safe event topic has no metadata contract"),
    }
}

fn ensure_safe_metadata(topic: &str, value: &Value) -> Result<()> {
    let allowed = allowed_safe_event_fields(topic)?;
    let object = value
        .as_object()
        .context("safe event metadata must be an object")?;
    ensure!(
        object.len() <= allowed.len(),
        "safe event metadata contains too many fields"
    );
    for (key, child) in object {
        let normalized = key.to_ascii_lowercase();
        ensure!(
            allowed.iter().any(|candidate| *candidate == normalized),
            "safe event metadata field is not allowed for this topic"
        );
        ensure!(
            !FORBIDDEN_EVENT_KEYS
                .iter()
                .any(|forbidden| normalized.contains(forbidden)),
            "safe event metadata contains a forbidden private field"
        );
        match child {
            Value::String(text) => ensure!(
                text.chars().count() <= 500,
                "safe event metadata string is too long"
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
            Value::Object(_) | Value::Array(_) => {
                bail!("safe event metadata values must be primitive")
            }
        }
    }
    Ok(())
}

fn validate_choice(value: &str, allowed: &[&str], label: &str) -> Result<String> {
    let value = bounded_text(value, 1, 160, label)?;
    ensure!(allowed.contains(&value.as_str()), "{label} is not allowed");
    Ok(value)
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    let parsed = Uuid::parse_str(value).with_context(|| format!("{label} is invalid"))?;
    Ok(parsed.to_string())
}

fn bounded_text(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    let count = value.chars().count();
    ensure!(
        count >= minimum && count <= maximum,
        "{label} length is invalid"
    );
    Ok(value.to_owned())
}

fn parse_utc(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{label} is invalid"))
        .map(|value| value.with_timezone(&Utc))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn now_utc() -> String {
    timestamp(Utc::now())
}

fn canonical_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).context("unable to serialize canonical schedule evidence")
}

fn hash_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(hash_text(&canonical_json(value)?))
}

fn hash_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn empty_object() -> Value {
    json!({})
}

fn positive_u64(value: i64) -> u64 {
    value.max(1) as u64
}

fn nonnegative_u64(value: i64) -> u64 {
    value.max(0) as u64
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
