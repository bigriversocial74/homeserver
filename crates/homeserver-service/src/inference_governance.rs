use crate::{model_center, openrouter_provider, AppState};
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
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0028_authorized_model_routing.sql");
const MIGRATION_KEY: &str = "0028_authorized_model_routing";
const MAX_CONTROL_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_PRIVATE_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_POLICIES: i64 = 10_000;
const MAX_REQUESTS: i64 = 100_000;
const MAX_ATTEMPTS: i64 = 400_000;
const MAX_RECEIPTS: i64 = 100_000;
const MAX_EVENTS: i64 = 200_000;
const PROVIDERS: &[&str] = &["ollama", "openrouter"];
const ACTOR_TYPES: &[&str] = &["local_user", "agent", "system", "mcp_client"];
const PUBLIC_REMOTE_CLASSES: &[&str] = &["public", "safe_receipt", "security_metadata"];
const NEVER_MODEL_CLASSES: &[&str] = &["secret"];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingPolicySummary {
    pub policy_id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub agent_id: Option<String>,
    pub agent_revision: Option<u64>,
    pub assignment_id: Option<String>,
    pub assignment_revision: Option<u64>,
    pub wrapper_id: Option<String>,
    pub connection_id: Option<String>,
    pub connection_authority_revision: Option<u64>,
    pub purpose: String,
    pub allowed_data_classes: Vec<String>,
    pub provider_order: Vec<String>,
    pub allowed_models: Vec<String>,
    pub allow_fallback: bool,
    pub remote_context_mode: String,
    pub require_zdr: bool,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
    pub window_seconds: u32,
    pub max_requests: u64,
    pub max_total_tokens: u64,
    pub max_spend_microusd: u64,
    pub policy_revision: u64,
    pub policy_hash: String,
    pub state: String,
    pub created_by_user_id: String,
    pub reason: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceReceiptSummary {
    pub receipt_id: String,
    pub request_id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub agent_id: Option<String>,
    pub assignment_id: Option<String>,
    pub wrapper_id: Option<String>,
    pub connection_id: Option<String>,
    pub policy_id: String,
    pub policy_revision: u64,
    pub purpose_hash: String,
    pub data_classification: String,
    pub provider_key: Option<String>,
    pub model_id: Option<String>,
    pub outcome: String,
    pub result_code: String,
    pub request_hash: String,
    pub authority_hash: String,
    pub prompt_hash: String,
    pub context_hash: String,
    pub result_hash: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub reported_cost_microusd: u64,
    pub receipt_hash: String,
    pub completed_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferenceEventSummary {
    pub event_id: String,
    pub request_id: Option<String>,
    pub policy_id: Option<String>,
    pub event_type: String,
    pub outcome: String,
    pub actor_type: String,
    pub actor_id: String,
    pub detail_code: String,
    pub metadata: Value,
    pub event_hash: String,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferenceGovernanceSnapshot {
    pub schema: String,
    pub policies: Vec<RoutingPolicySummary>,
    pub receipts: Vec<InferenceReceiptSummary>,
    pub events: Vec<InferenceEventSummary>,
    pub providers: Vec<String>,
    pub private_prompts_exposed: bool,
    pub private_results_exposed: bool,
    pub silent_remote_fallback_allowed: bool,
    pub provider_can_grant_authority: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateRoutingPolicyRequest {
    pub subject_type: String,
    pub agent_id: Option<String>,
    pub assignment_id: Option<String>,
    pub purpose: String,
    #[serde(default)]
    pub allowed_data_classes: Vec<String>,
    pub provider_order: Vec<String>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    pub allow_fallback: bool,
    pub remote_context_mode: String,
    pub require_zdr: bool,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
    pub window_seconds: u32,
    pub max_requests: u64,
    pub max_total_tokens: u64,
    pub max_spend_microusd: u64,
    pub created_by_user_id: String,
    pub reason: String,
    pub expires_minutes: u32,
    pub confirmation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyReferenceRequest {
    pub policy_id: String,
    pub actor_user_id: String,
    pub reason: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CancelInferenceRequest {
    pub request_id: String,
    pub actor_user_id: String,
    pub reason: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GovernedInferenceRequest {
    pub actor_type: String,
    pub actor_id: String,
    pub agent_id: Option<String>,
    pub assignment_id: Option<String>,
    pub policy_id: Option<String>,
    pub purpose: String,
    pub data_classification: String,
    pub provider_preference: Option<String>,
    pub model: Option<String>,
    pub privacy_selector_id: Option<String>,
    pub idempotency_key: String,
    pub prompt: String,
    pub context_hash: String,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GovernedInferenceResult {
    pub request_id: String,
    pub receipt_id: String,
    pub provider_key: String,
    pub model_id: String,
    pub output: String,
    pub output_hash: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub reported_cost_microusd: u64,
    pub policy_id: String,
    pub policy_revision: u64,
    pub authority_hash: String,
}

#[derive(Debug, Clone)]
struct PolicyRecord {
    policy_id: String,
    subject_type: String,
    subject_id: String,
    agent_id: Option<String>,
    agent_revision: Option<u64>,
    assignment_id: Option<String>,
    assignment_revision: Option<u64>,
    wrapper_id: Option<String>,
    connection_id: Option<String>,
    connection_authority_revision: Option<u64>,
    purpose: String,
    purpose_hash: String,
    allowed_data_classes: Vec<String>,
    provider_order: Vec<String>,
    allowed_models: Vec<String>,
    allow_fallback: bool,
    remote_context_mode: String,
    require_zdr: bool,
    max_input_chars: u32,
    max_output_tokens: u32,
    window_seconds: u32,
    max_requests: u64,
    max_total_tokens: u64,
    max_spend_microusd: u64,
    policy_revision: u64,
    policy_hash: String,
    state: String,
    created_by_user_id: String,
    reason: String,
    not_before_utc: String,
    expires_at_utc: String,
    created_at_utc: String,
    updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize)]
struct AuthorityDocument {
    schema: &'static str,
    subject_type: String,
    subject_id: String,
    agent_id: Option<String>,
    agent_revision: Option<u64>,
    assignment_id: Option<String>,
    assignment_revision: Option<u64>,
    wrapper_id: Option<String>,
    connection_id: Option<String>,
    connection_authority_revision: Option<u64>,
    policy_id: String,
    policy_revision: u64,
    policy_hash: String,
    purpose_hash: String,
    data_classification: String,
    privacy_selector_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ProviderOutcome {
    provider_key: String,
    model_id: String,
    output: String,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    reported_cost_microusd: u64,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    reconcile_interrupted(connection)?;
    health_check(connection)?;
    maintain_history(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "model routing migration is not registered exactly once"
    );
    for table in [
        "model_routing_policies",
        "model_inference_requests",
        "model_inference_attempts",
        "model_inference_private_results",
        "model_inference_receipts",
        "model_inference_events",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let missing_receipts: i64 = connection.query_row(
        "SELECT COUNT(*) FROM model_inference_requests r LEFT JOIN model_inference_receipts x ON x.request_id=r.request_id WHERE r.state IN ('completed','failed','cancelled','interrupted') AND x.request_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        missing_receipts == 0,
        "terminal model inference requests are missing immutable receipts"
    );
    let missing_private_results: i64 = connection.query_row(
        "SELECT COUNT(*) FROM model_inference_requests r LEFT JOIN model_inference_private_results x ON x.request_id=r.request_id WHERE r.state='completed' AND x.request_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        missing_private_results == 0,
        "completed model inference requests are missing private results"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    reconcile_interrupted(connection)?;
    for (table, limit, message) in [
        (
            "model_routing_policies",
            MAX_POLICIES,
            "model routing policy retention requires archival",
        ),
        (
            "model_inference_requests",
            MAX_REQUESTS,
            "model inference request retention requires archival",
        ),
        (
            "model_inference_attempts",
            MAX_ATTEMPTS,
            "model inference attempt retention requires archival",
        ),
        (
            "model_inference_receipts",
            MAX_RECEIPTS,
            "model inference receipt retention requires archival",
        ),
        (
            "model_inference_events",
            MAX_EVENTS,
            "model inference event retention requires archival",
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
        .route("/v1/models/governance", get(snapshot_handler))
        .route("/v1/models/governance/policies", post(create_policy_handler))
        .route(
            "/v1/models/governance/policies/revoke",
            post(revoke_policy_handler),
        )
        .route("/v1/models/inference", post(inference_handler))
        .route("/v1/models/inference/cancel", post(cancel_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn snapshot_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<InferenceGovernanceSnapshot> {
    run_blocking(move || snapshot(&state), "model_governance_snapshot_failed").await
}

async fn create_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateRoutingPolicyRequest>,
) -> ApiResult<InferenceGovernanceSnapshot> {
    run_blocking(
        move || {
            create_policy(&state, request)?;
            snapshot(&state)
        },
        "model_governance_policy_rejected",
    )
    .await
}

async fn revoke_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PolicyReferenceRequest>,
) -> ApiResult<InferenceGovernanceSnapshot> {
    run_blocking(
        move || {
            revoke_policy(&state, request)?;
            snapshot(&state)
        },
        "model_governance_policy_revoke_rejected",
    )
    .await
}

async fn inference_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<GovernedInferenceRequest>,
) -> ApiResult<GovernedInferenceResult> {
    infer(state, request)
        .await
        .map(Json)
        .map_err(|error| api_error("model_inference_rejected", error))
}

async fn cancel_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CancelInferenceRequest>,
) -> ApiResult<InferenceGovernanceSnapshot> {
    run_blocking(
        move || {
            cancel_request(&state, request)?;
            snapshot(&state)
        },
        "model_inference_cancel_rejected",
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
        .map_err(|error| api_error(code, anyhow::anyhow!("model governance task failed: {error}")))?
        .map(Json)
        .map_err(|error| api_error(code, error))
}

pub fn snapshot(state: &AppState) -> Result<InferenceGovernanceSnapshot> {
    let connection = state.connection()?;
    let policies = read_policies(&connection)?;
    let receipts = read_receipts(&connection)?;
    let events = read_events(&connection)?;
    Ok(InferenceGovernanceSnapshot {
        schema: "homeserver.model-inference-governance.v1".to_owned(),
        policies,
        receipts,
        events,
        providers: PROVIDERS.iter().map(|value| (*value).to_owned()).collect(),
        private_prompts_exposed: false,
        private_results_exposed: false,
        silent_remote_fallback_allowed: false,
        provider_can_grant_authority: false,
    })
}

pub async fn infer(
    state: Arc<AppState>,
    mut request: GovernedInferenceRequest,
) -> Result<GovernedInferenceResult> {
    normalize_inference_request(&mut request)?;
    let prompt_hash = hash_text(&request.prompt);
    let input_chars = request.prompt.chars().count() as u32;
    let (policy, authority, authority_hash, request_id, request_hash, existing) = {
        let connection = state.connection()?;
        let policy = select_policy(&connection, &request)?;
        ensure!(
            input_chars <= policy.max_input_chars,
            "inference prompt exceeds the policy input limit"
        );
        ensure!(
            policy
                .allowed_data_classes
                .contains(&request.data_classification),
            "data classification is not allowed by the inference policy"
        );
        ensure!(
            !NEVER_MODEL_CLASSES.contains(&request.data_classification.as_str()),
            "this data classification may never enter a model"
        );
        let authority = capture_authority(&connection, &request, &policy)?;
        let authority_hash = hash_json(&authority)?;
        let max_output_tokens = request
            .max_output_tokens
            .unwrap_or(policy.max_output_tokens)
            .min(policy.max_output_tokens);
        request.max_output_tokens = Some(max_output_tokens);
        let request_document = json!({
            "schema": "homeserver.model-inference-request.v1",
            "actor_type": request.actor_type,
            "actor_id": request.actor_id,
            "agent_id": request.agent_id,
            "assignment_id": request.assignment_id,
            "policy_id": policy.policy_id,
            "policy_revision": policy.policy_revision,
            "purpose": request.purpose,
            "data_classification": request.data_classification,
            "provider_preference": request.provider_preference,
            "model": request.model,
            "privacy_selector_id": request.privacy_selector_id,
            "idempotency_key": request.idempotency_key,
            "prompt_hash": prompt_hash,
            "context_hash": request.context_hash,
            "max_output_tokens": max_output_tokens,
            "authority_hash": authority_hash
        });
        let request_hash = hash_json(&request_document)?;
        if let Some(existing) = existing_request(&connection, &request.idempotency_key)? {
            ensure!(
                existing.1 == request_hash,
                "inference idempotency key was reused with a different request"
            );
            let result = if existing.2 == "completed" {
                Some(load_completed_result(&connection, &existing.0)?)
            } else {
                None
            };
            (policy, authority, authority_hash, existing.0, request_hash, result)
        } else {
            reserve_request(
                &connection,
                &request,
                &policy,
                &authority,
                &authority_hash,
                &request_hash,
                &prompt_hash,
                input_chars,
            )?
        }
    };
    if let Some(result) = existing {
        return Ok(result);
    }

    let provider_order = effective_provider_order(&policy, &request)?;
    let mut last_error: Option<anyhow::Error> = None;
    for (index, provider) in provider_order.iter().enumerate() {
        let sequence = (index + 1) as u32;
        let model = match resolve_model(state.clone(), provider, request.model.as_deref(), &policy).await {
            Ok(model) => model,
            Err(error) => {
                last_error = Some(error);
                if !policy.allow_fallback {
                    break;
                }
                continue;
            }
        };
        let result = attempt_provider(
            state.clone(),
            &request,
            &policy,
            &authority,
            &authority_hash,
            &request_id,
            &request_hash,
            provider,
            &model,
            sequence,
        )
        .await;
        match result {
            Ok(result) => return Ok(result),
            Err(error) => {
                last_error = Some(error);
                if !policy.allow_fallback {
                    break;
                }
            }
        }
    }

    let error = last_error.unwrap_or_else(|| anyhow::anyhow!("no authorized model provider was available"));
    let failure_code = public_failure_code(&error);
    {
        let connection = state.connection()?;
        finish_failed_request(
            &connection,
            &request_id,
            &request,
            &policy,
            &authority_hash,
            &request_hash,
            &prompt_hash,
            &failure_code,
        )?;
    }
    Err(error)
}

#[allow(clippy::too_many_arguments)]
async fn attempt_provider(
    state: Arc<AppState>,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    authority: &AuthorityDocument,
    authority_hash: &str,
    request_id: &str,
    request_hash: &str,
    provider: &str,
    model: &str,
    sequence: u32,
) -> Result<GovernedInferenceResult> {
    {
        let connection = state.connection()?;
        revalidate_authority(&connection, request, policy, authority)?;
        validate_remote_context(&connection, request, policy, provider)?;
        ensure_request_running(&connection, request_id)?;
    }
    let decision_document = json!({
        "schema": "homeserver.model-routing-decision.v1",
        "request_id": request_id,
        "request_hash": request_hash,
        "policy_id": policy.policy_id,
        "policy_revision": policy.policy_revision,
        "authority_hash": authority_hash,
        "provider": provider,
        "model": model,
        "attempt_sequence": sequence,
        "fallback_authorized": policy.allow_fallback
    });
    let decision_hash = hash_json(&decision_document)?;
    let attempt_id = Uuid::new_v4().to_string();
    {
        let connection = state.connection()?;
        connection.execute(
            "INSERT INTO model_inference_attempts (attempt_id,request_id,attempt_sequence,provider_key,model_id,authority_hash,decision_hash,state,started_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'running',?8)",
            params![attempt_id,request_id,sequence,provider,model,authority_hash,decision_hash,now_utc()],
        )?;
        connection.execute(
            "UPDATE model_inference_requests SET state='running',attempt_count=?1,selected_provider=?2,selected_model=?3,started_at_utc=COALESCE(started_at_utc,?4) WHERE request_id=?5 AND state IN ('reserved','running')",
            params![sequence,provider,model,now_utc(),request_id],
        )?;
        record_event(
            &connection,
            Some(request_id),
            Some(&policy.policy_id),
            "model.inference_attempt_started",
            "success",
            &request.actor_type,
            &request.actor_id,
            "authority_revalidated",
            json!({
                "attempt_sequence": sequence,
                "provider": provider,
                "model": model,
                "decision_hash": decision_hash,
                "prompt_exposed": false
            }),
        )?;
    }

    let provider_result = match provider {
        "ollama" => model_center::generate_text(
            state.clone(),
            model.to_owned(),
            request.prompt.clone(),
            request.max_output_tokens.unwrap_or(policy.max_output_tokens),
        )
        .await
        .map(|output| ProviderOutcome {
            provider_key: "ollama".to_owned(),
            model_id: model.to_owned(),
            output,
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            reported_cost_microusd: 0,
        }),
        "openrouter" => openrouter_provider::generate_governed_response(
            state.clone(),
            model,
            &request.prompt,
            request.max_output_tokens.unwrap_or(policy.max_output_tokens),
            request_id,
        )
        .await
        .map(|result| ProviderOutcome {
            provider_key: "openrouter".to_owned(),
            model_id: result.resolved_model,
            output: result.output,
            prompt_tokens: result.prompt_tokens,
            completion_tokens: result.completion_tokens,
            total_tokens: result.total_tokens,
            reported_cost_microusd: result.reported_cost_microusd,
        }),
        _ => bail!("unsupported governed model provider"),
    };

    match provider_result {
        Ok(result) => {
            let connection = state.connection()?;
            revalidate_authority(&connection, request, policy, authority)?;
            validate_remote_context(&connection, request, policy, provider)?;
            ensure_request_running(&connection, request_id)?;
            ensure!(
                result.output.as_bytes().len() <= MAX_PRIVATE_OUTPUT_BYTES,
                "model output exceeds the private result limit"
            );
            let window_usage = policy_usage(&connection, policy)?;
            ensure!(
                window_usage.1.saturating_add(result.total_tokens) <= policy.max_total_tokens,
                "model inference token budget would be exceeded"
            );
            ensure!(
                window_usage.2.saturating_add(result.reported_cost_microusd)
                    <= policy.max_spend_microusd,
                "model inference spending budget would be exceeded"
            );
            complete_success(
                &connection,
                request_id,
                request,
                policy,
                authority_hash,
                request_hash,
                &hash_text(&request.prompt),
                &attempt_id,
                &result,
            )
        }
        Err(error) => {
            let failure_code = public_failure_code(&error);
            let connection = state.connection()?;
            connection.execute(
                "UPDATE model_inference_attempts SET state='failed',failure_code=?1,completed_at_utc=?2 WHERE attempt_id=?3 AND state='running'",
                params![failure_code,now_utc(),attempt_id],
            )?;
            record_event(
                &connection,
                Some(request_id),
                Some(&policy.policy_id),
                "model.inference_attempt_failed",
                "error",
                &request.actor_type,
                &request.actor_id,
                &failure_code,
                json!({
                    "attempt_sequence": sequence,
                    "provider": provider,
                    "model": model,
                    "error_hash": hash_text(&error.to_string())
                }),
            )?;
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn complete_success(
    connection: &Connection,
    request_id: &str,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    authority_hash: &str,
    request_hash: &str,
    prompt_hash: &str,
    attempt_id: &str,
    result: &ProviderOutcome,
) -> Result<GovernedInferenceResult> {
    let output = result.output.trim();
    ensure!(!output.is_empty(), "model returned an empty result");
    let output_hash = hash_text(output);
    let completed_at = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE model_inference_attempts SET state='succeeded',prompt_tokens=?1,completion_tokens=?2,total_tokens=?3,reported_cost_microusd=?4,output_hash=?5,completed_at_utc=?6 WHERE attempt_id=?7 AND state='running'",
        params![
            result.prompt_tokens as i64,
            result.completion_tokens as i64,
            result.total_tokens as i64,
            result.reported_cost_microusd as i64,
            output_hash,
            completed_at,
            attempt_id
        ],
    )?;
    transaction.execute(
        "INSERT INTO model_inference_private_results (request_id,classification,output_text,output_bytes,output_hash,created_at_utc) VALUES (?1,'private',?2,?3,?4,?5)",
        params![request_id,output,output.as_bytes().len() as i64,output_hash,completed_at],
    )?;
    transaction.execute(
        "UPDATE model_inference_requests SET state='completed',selected_provider=?1,selected_model=?2,result_hash=?3,completed_at_utc=?4 WHERE request_id=?5 AND state='running'",
        params![result.provider_key,result.model_id,output_hash,completed_at,request_id],
    )?;
    let receipt_id = write_receipt_tx(
        &transaction,
        request_id,
        request,
        policy,
        authority_hash,
        request_hash,
        prompt_hash,
        Some(&result.provider_key),
        Some(&result.model_id),
        "completed",
        "model_inference_completed",
        Some(&output_hash),
        result.prompt_tokens,
        result.completion_tokens,
        result.total_tokens,
        result.reported_cost_microusd,
        &completed_at,
    )?;
    record_event_tx(
        &transaction,
        Some(request_id),
        Some(&policy.policy_id),
        "model.inference_completed",
        "success",
        &request.actor_type,
        &request.actor_id,
        "private_result_retained",
        json!({
            "provider": result.provider_key,
            "model": result.model_id,
            "result_hash": output_hash,
            "receipt_id": receipt_id,
            "private_result_exposed": false
        }),
    )?;
    transaction.commit()?;
    Ok(GovernedInferenceResult {
        request_id: request_id.to_owned(),
        receipt_id,
        provider_key: result.provider_key.clone(),
        model_id: result.model_id.clone(),
        output: output.to_owned(),
        output_hash,
        prompt_tokens: result.prompt_tokens,
        completion_tokens: result.completion_tokens,
        total_tokens: result.total_tokens,
        reported_cost_microusd: result.reported_cost_microusd,
        policy_id: policy.policy_id.clone(),
        policy_revision: policy.policy_revision,
        authority_hash: authority_hash.to_owned(),
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_failed_request(
    connection: &Connection,
    request_id: &str,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    authority_hash: &str,
    request_hash: &str,
    prompt_hash: &str,
    failure_code: &str,
) -> Result<()> {
    let completed_at = now_utc();
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE model_inference_requests SET state='failed',failure_code=?1,completed_at_utc=?2 WHERE request_id=?3 AND state IN ('reserved','running')",
        params![failure_code,completed_at,request_id],
    )?;
    if changed == 0 {
        transaction.commit()?;
        return Ok(());
    }
    write_receipt_tx(
        &transaction,
        request_id,
        request,
        policy,
        authority_hash,
        request_hash,
        prompt_hash,
        None,
        None,
        "failed",
        failure_code,
        None,
        0,
        0,
        0,
        0,
        &completed_at,
    )?;
    record_event_tx(
        &transaction,
        Some(request_id),
        Some(&policy.policy_id),
        "model.inference_failed",
        "error",
        &request.actor_type,
        &request.actor_id,
        failure_code,
        json!({"private_prompt_exposed": false}),
    )?;
    transaction.commit()?;
    Ok(())
}

fn reserve_request(
    connection: &Connection,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    authority: &AuthorityDocument,
    authority_hash: &str,
    request_hash: &str,
    prompt_hash: &str,
    input_chars: u32,
) -> Result<(
    PolicyRecord,
    AuthorityDocument,
    String,
    String,
    String,
    Option<GovernedInferenceResult>,
)> {
    let usage = policy_usage(connection, policy)?;
    ensure!(usage.0 < policy.max_requests, "model inference request budget has been reached");
    ensure!(usage.1 < policy.max_total_tokens, "model inference token budget has been reached");
    ensure!(usage.2 < policy.max_spend_microusd || policy.max_spend_microusd == 0, "model inference spending budget has been reached");
    let request_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO model_inference_requests (request_id,idempotency_key,request_hash,subject_type,subject_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,policy_id,policy_revision,policy_hash,purpose,purpose_hash,data_classification,provider_order_json,requested_model,privacy_selector_id,prompt_hash,context_hash,authority_hash,input_chars,max_output_tokens,state,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,'reserved',?27)",
        params![
            request_id,
            request.idempotency_key,
            request_hash,
            policy.subject_type,
            policy.subject_id,
            authority.agent_id,
            authority.agent_revision.map(|value| value as i64),
            authority.assignment_id,
            authority.assignment_revision.map(|value| value as i64),
            authority.wrapper_id,
            authority.connection_id,
            authority.connection_authority_revision.map(|value| value as i64),
            policy.policy_id,
            policy.policy_revision as i64,
            policy.policy_hash,
            request.purpose,
            policy.purpose_hash,
            request.data_classification,
            serde_json::to_string(&effective_provider_order(policy, request)?)?,
            request.model,
            request.privacy_selector_id,
            prompt_hash,
            request.context_hash,
            authority_hash,
            input_chars as i64,
            request.max_output_tokens.unwrap_or(policy.max_output_tokens) as i64,
            now
        ],
    )?;
    record_event_tx(
        &transaction,
        Some(&request_id),
        Some(&policy.policy_id),
        "model.inference_reserved",
        "success",
        &request.actor_type,
        &request.actor_id,
        "budget_reserved",
        json!({
            "authority_hash": authority_hash,
            "request_hash": request_hash,
            "data_classification": request.data_classification,
            "private_prompt_exposed": false
        }),
    )?;
    transaction.commit()?;
    Ok((
        policy.clone(),
        authority.clone(),
        authority_hash.to_owned(),
        request_id,
        request_hash.to_owned(),
        None,
    ))
}

fn create_policy(state: &AppState, request: CreateRoutingPolicyRequest) -> Result<String> {
    ensure!(
        request.confirmation == "CREATE MODEL POLICY",
        "type CREATE MODEL POLICY to create inference authority"
    );
    let subject_type = choice(
        &request.subject_type,
        &["local_control_center", "agent_assignment"],
        "policy subject type",
    )?;
    let purpose = bounded_text(&request.purpose, 1, 500, "policy purpose")?;
    let actor = bounded_text(&request.created_by_user_id, 1, 160, "policy creator")?;
    let reason = bounded_text(&request.reason, 1, 500, "policy reason")?;
    let classes = normalize_classes(&request.allowed_data_classes)?;
    let providers = normalize_providers(&request.provider_order)?;
    let models = normalize_models(&request.allowed_models, &providers)?;
    ensure!(
        !request.allow_fallback || providers.len() > 1,
        "fallback requires more than one ordered provider"
    );
    let remote_context_mode = choice(
        &request.remote_context_mode,
        &["deny", "public_only", "approved_selector"],
        "remote context mode",
    )?;
    if providers.iter().any(|provider| provider == "openrouter") {
        ensure!(
            remote_context_mode != "deny",
            "OpenRouter requires an explicit remote context mode"
        );
    } else {
        ensure!(
            remote_context_mode == "deny",
            "local-only policies must deny remote context"
        );
    }
    ensure!((1..=30_000).contains(&request.max_input_chars), "policy input limit is invalid");
    ensure!((16..=4_096).contains(&request.max_output_tokens), "policy output limit is invalid");
    ensure!((60..=2_592_000).contains(&request.window_seconds), "policy window is invalid");
    ensure!((1..=1_000_000).contains(&request.max_requests), "policy request budget is invalid");
    ensure!((16..=1_000_000_000).contains(&request.max_total_tokens), "policy token budget is invalid");
    ensure!(request.max_spend_microusd <= 1_000_000_000_000, "policy spending budget is invalid");
    ensure!((1..=525_600).contains(&request.expires_minutes), "policy expiration is invalid");

    let connection = state.connection()?;
    let now = Utc::now();
    let expires = now + Duration::minutes(i64::from(request.expires_minutes));
    let (subject_id, agent_id, agent_revision, assignment_id, assignment_revision, wrapper_id, connection_id, connection_authority_revision) =
        if subject_type == "local_control_center" {
            ensure!(request.agent_id.is_none() && request.assignment_id.is_none(), "local policy cannot bind an agent assignment");
            (
                "local_control_center".to_owned(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        } else {
            let agent_id = validate_uuid(request.agent_id.as_deref().context("agent policy requires agent_id")?, "agent ID")?;
            let assignment_id = validate_uuid(request.assignment_id.as_deref().context("agent policy requires assignment_id")?, "assignment ID")?;
            let row = connection.query_row(
                "SELECT a.revision,x.assignment_revision,x.wrapper_id,x.connection_id,c.grant_revision FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.agent_id=a.agent_id JOIN wrapper_connections c ON c.connection_id=x.connection_id AND c.wrapper_id=x.wrapper_id WHERE a.agent_id=?1 AND a.state='active' AND a.expires_at_utc>?3 AND x.assignment_id=?2 AND x.state='active' AND x.expires_at_utc>?3 AND c.lifecycle_state='active'",
                params![agent_id,assignment_id,now_utc()],
                |row| Ok((row.get::<_,i64>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?)),
            ).context("active agent assignment was not found")?;
            ensure_no_emergency_stop(&connection, &agent_id, &row.2, &row.3)?;
            (
                assignment_id.clone(),
                Some(agent_id),
                Some(positive_u64(row.0)),
                Some(assignment_id),
                Some(positive_u64(row.1)),
                Some(row.2),
                Some(row.3),
                Some(nonnegative_u64(row.4)),
            )
        };
    if remote_context_mode == "approved_selector" {
        ensure!(subject_type == "agent_assignment", "approved-selector remote context requires an agent assignment");
    }
    let purpose_hash = hash_text(&purpose);
    let previous_revision: i64 = connection.query_row(
        "SELECT COALESCE(MAX(policy_revision),0) FROM model_routing_policies WHERE subject_type=?1 AND subject_id=?2 AND purpose_hash=?3",
        params![subject_type,subject_id,purpose_hash],
        |row| row.get(0),
    )?;
    let revision = positive_u64(previous_revision.saturating_add(1));
    let policy_document = json!({
        "schema": "homeserver.model-routing-policy.v1",
        "subject_type": subject_type,
        "subject_id": subject_id,
        "agent_id": agent_id,
        "agent_revision": agent_revision,
        "assignment_id": assignment_id,
        "assignment_revision": assignment_revision,
        "wrapper_id": wrapper_id,
        "connection_id": connection_id,
        "connection_authority_revision": connection_authority_revision,
        "purpose": purpose,
        "allowed_data_classes": classes,
        "provider_order": providers,
        "allowed_models": models,
        "allow_fallback": request.allow_fallback,
        "remote_context_mode": remote_context_mode,
        "require_zdr": request.require_zdr,
        "max_input_chars": request.max_input_chars,
        "max_output_tokens": request.max_output_tokens,
        "window_seconds": request.window_seconds,
        "max_requests": request.max_requests,
        "max_total_tokens": request.max_total_tokens,
        "max_spend_microusd": request.max_spend_microusd,
        "policy_revision": revision,
        "not_before_utc": timestamp(now),
        "expires_at_utc": timestamp(expires)
    });
    let policy_hash = hash_json(&policy_document)?;
    let policy_id = Uuid::new_v4().to_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE model_routing_policies SET state='superseded',updated_at_utc=?1 WHERE subject_type=?2 AND subject_id=?3 AND purpose_hash=?4 AND state='active'",
        params![now_utc(),subject_type,subject_id,purpose_hash],
    )?;
    transaction.execute(
        "INSERT INTO model_routing_policies (policy_id,subject_type,subject_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,purpose,purpose_hash,allowed_data_classes_json,provider_order_json,allowed_models_json,allow_fallback,remote_context_mode,require_zdr,max_input_chars,max_output_tokens,window_seconds,max_requests,max_total_tokens,max_spend_microusd,policy_revision,policy_hash,state,created_by_user_id,reason,not_before_utc,expires_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,'active',?27,?28,?29,?30,?29,?29)",
        params![
            policy_id,subject_type,subject_id,agent_id,agent_revision.map(|value| value as i64),assignment_id,
            assignment_revision.map(|value| value as i64),wrapper_id,connection_id,
            connection_authority_revision.map(|value| value as i64),purpose,purpose_hash,
            serde_json::to_string(&classes)?,serde_json::to_string(&providers)?,serde_json::to_string(&models)?,
            i64::from(request.allow_fallback),remote_context_mode,i64::from(request.require_zdr),
            request.max_input_chars as i64,request.max_output_tokens as i64,request.window_seconds as i64,
            request.max_requests as i64,request.max_total_tokens as i64,request.max_spend_microusd as i64,
            revision as i64,policy_hash,actor,reason,timestamp(now),timestamp(expires)
        ],
    )?;
    record_event_tx(
        &transaction,
        None,
        Some(&policy_id),
        "model.routing_policy_created",
        "success",
        "local_user",
        &actor,
        "authority_captured",
        json!({"policy_hash": policy_hash,"policy_revision": revision}),
    )?;
    transaction.commit()?;
    Ok(policy_id)
}

fn revoke_policy(state: &AppState, request: PolicyReferenceRequest) -> Result<()> {
    let policy_id = validate_uuid(&request.policy_id, "policy ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "policy actor")?;
    let reason = bounded_text(&request.reason, 1, 500, "policy revoke reason")?;
    ensure!(
        request.confirmation == format!("REVOKE MODEL POLICY {policy_id}"),
        "policy revoke confirmation is invalid"
    );
    let connection = state.connection()?;
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE model_routing_policies SET state='revoked',revoked_at_utc=?1,updated_at_utc=?1 WHERE policy_id=?2 AND state='active'",
        params![now,policy_id],
    )?;
    ensure!(changed == 1, "active model routing policy was not found");
    transaction.execute(
        "UPDATE model_inference_requests SET state='cancelled',failure_code='policy_revoked',completed_at_utc=?1 WHERE policy_id=?2 AND state IN ('reserved','running')",
        params![now,policy_id],
    )?;
    finalize_unreceipted_terminal_requests_tx(&transaction)?;
    record_event_tx(
        &transaction,
        None,
        Some(&policy_id),
        "model.routing_policy_revoked",
        "warning",
        "local_user",
        &actor,
        "policy_revoked",
        json!({"reason": reason}),
    )?;
    transaction.commit()?;
    Ok(())
}

fn cancel_request(state: &AppState, request: CancelInferenceRequest) -> Result<()> {
    let request_id = validate_uuid(&request.request_id, "inference request ID")?;
    let actor = bounded_text(&request.actor_user_id, 1, 160, "inference actor")?;
    let reason = bounded_text(&request.reason, 1, 500, "inference cancellation reason")?;
    ensure!(
        request.confirmation == format!("CANCEL INFERENCE {request_id}"),
        "inference cancellation confirmation is invalid"
    );
    let connection = state.connection()?;
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE model_inference_requests SET state='cancelled',failure_code='cancelled_by_authority',completed_at_utc=?1 WHERE request_id=?2 AND state IN ('reserved','running')",
        params![now,request_id],
    )?;
    ensure!(changed == 1, "active inference request was not found");
    transaction.execute(
        "UPDATE model_inference_attempts SET state='cancelled',failure_code='cancelled_by_authority',completed_at_utc=?1 WHERE request_id=?2 AND state='running'",
        params![now,request_id],
    )?;
    finalize_unreceipted_terminal_requests_tx(&transaction)?;
    record_event_tx(
        &transaction,
        Some(&request_id),
        None,
        "model.inference_cancelled",
        "warning",
        "local_user",
        &actor,
        "cancelled_by_authority",
        json!({"reason": reason}),
    )?;
    transaction.commit()?;
    Ok(())
}

fn select_policy(connection: &Connection, request: &GovernedInferenceRequest) -> Result<PolicyRecord> {
    let (subject_type, subject_id) = match (&request.agent_id, &request.assignment_id) {
        (Some(_), Some(assignment_id)) => ("agent_assignment", assignment_id.as_str()),
        (None, None) => ("local_control_center", "local_control_center"),
        _ => bail!("agent_id and assignment_id must be supplied together"),
    };
    let purpose_hash = hash_text(&request.purpose);
    let now = now_utc();
    let policy = if let Some(policy_id) = request.policy_id.as_deref() {
        let policy_id = validate_uuid(policy_id, "policy ID")?;
        connection.query_row(
            "SELECT policy_id,subject_type,subject_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,purpose,purpose_hash,allowed_data_classes_json,provider_order_json,allowed_models_json,allow_fallback,remote_context_mode,require_zdr,max_input_chars,max_output_tokens,window_seconds,max_requests,max_total_tokens,max_spend_microusd,policy_revision,policy_hash,state,created_by_user_id,reason,not_before_utc,expires_at_utc,created_at_utc,updated_at_utc FROM model_routing_policies WHERE policy_id=?1 AND subject_type=?2 AND subject_id=?3 AND purpose_hash=?4 AND state='active' AND not_before_utc<=?5 AND expires_at_utc>?5",
            params![policy_id,subject_type,subject_id,purpose_hash,now],
            map_policy,
        )
    } else {
        connection.query_row(
            "SELECT policy_id,subject_type,subject_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,purpose,purpose_hash,allowed_data_classes_json,provider_order_json,allowed_models_json,allow_fallback,remote_context_mode,require_zdr,max_input_chars,max_output_tokens,window_seconds,max_requests,max_total_tokens,max_spend_microusd,policy_revision,policy_hash,state,created_by_user_id,reason,not_before_utc,expires_at_utc,created_at_utc,updated_at_utc FROM model_routing_policies WHERE subject_type=?1 AND subject_id=?2 AND purpose_hash=?3 AND state='active' AND not_before_utc<=?4 AND expires_at_utc>?4 ORDER BY policy_revision DESC LIMIT 1",
            params![subject_type,subject_id,purpose_hash,now],
            map_policy,
        )
    };
    policy.context("active inference policy was not found for this subject and purpose")
}

fn capture_authority(
    connection: &Connection,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
) -> Result<AuthorityDocument> {
    let authority = AuthorityDocument {
        schema: "homeserver.model-inference-authority.v1",
        subject_type: policy.subject_type.clone(),
        subject_id: policy.subject_id.clone(),
        agent_id: policy.agent_id.clone(),
        agent_revision: policy.agent_revision,
        assignment_id: policy.assignment_id.clone(),
        assignment_revision: policy.assignment_revision,
        wrapper_id: policy.wrapper_id.clone(),
        connection_id: policy.connection_id.clone(),
        connection_authority_revision: policy.connection_authority_revision,
        policy_id: policy.policy_id.clone(),
        policy_revision: policy.policy_revision,
        policy_hash: policy.policy_hash.clone(),
        purpose_hash: policy.purpose_hash.clone(),
        data_classification: request.data_classification.clone(),
        privacy_selector_id: request.privacy_selector_id.clone(),
    };
    revalidate_authority(connection, request, policy, &authority)?;
    Ok(authority)
}

fn revalidate_authority(
    connection: &Connection,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    authority: &AuthorityDocument,
) -> Result<()> {
    let now = now_utc();
    let active_policy: i64 = connection.query_row(
        "SELECT COUNT(*) FROM model_routing_policies WHERE policy_id=?1 AND policy_revision=?2 AND policy_hash=?3 AND state='active' AND not_before_utc<=?4 AND expires_at_utc>?4",
        params![policy.policy_id,policy.policy_revision as i64,policy.policy_hash,now],
        |row| row.get(0),
    )?;
    ensure!(active_policy == 1, "model routing policy changed or expired");
    ensure!(
        authority.policy_id == policy.policy_id
            && authority.policy_revision == policy.policy_revision
            && authority.policy_hash == policy.policy_hash
            && authority.purpose_hash == policy.purpose_hash
            && authority.data_classification == request.data_classification,
        "captured inference authority no longer matches the request"
    );
    if policy.subject_type == "local_control_center" {
        ensure!(request.actor_type == "local_user" || request.actor_type == "system", "local inference policy requires a trusted local actor");
        ensure!(request.agent_id.is_none() && request.assignment_id.is_none(), "local inference cannot claim agent authority");
        return Ok(());
    }
    let agent_id = policy.agent_id.as_deref().context("agent policy is missing agent ID")?;
    let assignment_id = policy.assignment_id.as_deref().context("agent policy is missing assignment ID")?;
    let wrapper_id = policy.wrapper_id.as_deref().context("agent policy is missing wrapper ID")?;
    let connection_id = policy.connection_id.as_deref().context("agent policy is missing connection ID")?;
    ensure!(request.agent_id.as_deref() == Some(agent_id) && request.assignment_id.as_deref() == Some(assignment_id), "request agent assignment does not match policy authority");
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.agent_id=a.agent_id JOIN wrapper_connections c ON c.connection_id=x.connection_id AND c.wrapper_id=x.wrapper_id WHERE a.agent_id=?1 AND a.revision=?2 AND a.state='active' AND a.expires_at_utc>?8 AND x.assignment_id=?3 AND x.assignment_revision=?4 AND x.state='active' AND x.expires_at_utc>?8 AND c.wrapper_id=?5 AND c.connection_id=?6 AND c.grant_revision=?7 AND c.lifecycle_state='active'",
        params![
            agent_id,
            policy.agent_revision.context("agent revision missing")? as i64,
            assignment_id,
            policy.assignment_revision.context("assignment revision missing")? as i64,
            wrapper_id,
            connection_id,
            policy.connection_authority_revision.context("connection authority revision missing")? as i64,
            now
        ],
        |row| row.get(0),
    )?;
    ensure!(count == 1, "agent, assignment, wrapper, or connection authority changed");
    ensure_no_emergency_stop(connection, agent_id, wrapper_id, connection_id)?;
    enforce_agent_model_restrictions(connection, agent_id, policy)?;
    Ok(())
}

fn validate_remote_context(
    connection: &Connection,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    provider: &str,
) -> Result<()> {
    if provider == "ollama" {
        return Ok(());
    }
    ensure!(provider == "openrouter", "remote provider is unsupported");
    ensure!(policy.remote_context_mode != "deny", "remote model context is denied by policy");
    let provider_snapshot = openrouter_provider::snapshot_from_connection_for_governance(connection)?;
    ensure!(provider_snapshot.enabled && provider_snapshot.allow_remote_context && provider_snapshot.api_key_configured, "OpenRouter is not ready for governed inference");
    if policy.require_zdr {
        ensure!(provider_snapshot.zdr_only, "policy requires zero-data-retention routing");
    }
    if PUBLIC_REMOTE_CLASSES.contains(&request.data_classification.as_str()) {
        ensure!(matches!(policy.remote_context_mode.as_str(), "public_only" | "approved_selector"), "public remote context is not allowed by policy");
        return Ok(());
    }
    ensure!(policy.remote_context_mode == "approved_selector", "private remote context requires an approved Phase 16E selector");
    ensure!(request.data_classification != "private_source", "raw private source content may not be sent to a remote model");
    let selector_id = validate_uuid(
        request
            .privacy_selector_id
            .as_deref()
            .context("private remote context requires privacy_selector_id")?,
        "privacy selector ID",
    )?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM private_resource_selectors WHERE selector_id=?1 AND wrapper_id=?2 AND connection_id=?3 AND agent_id=?4 AND agent_revision=?5 AND purpose_hash=?6 AND state='active' AND expires_at_utc>?7 AND remote_model_mode='approved_provider' AND approved_remote_provider='openrouter'",
        params![
            selector_id,
            policy.wrapper_id.as_deref().context("wrapper authority missing")?,
            policy.connection_id.as_deref().context("connection authority missing")?,
            policy.agent_id.as_deref().context("agent authority missing")?,
            policy.agent_revision.context("agent revision missing")? as i64,
            policy.purpose_hash,
            now_utc()
        ],
        |row| row.get(0),
    )?;
    ensure!(count == 1, "Phase 16E selector does not authorize this remote model context");
    Ok(())
}

fn enforce_agent_model_restrictions(
    connection: &Connection,
    agent_id: &str,
    policy: &PolicyRecord,
) -> Result<()> {
    let restrictions_json: String = connection.query_row(
        "SELECT model_restrictions_json FROM homeserver_agents WHERE agent_id=?1",
        params![agent_id],
        |row| row.get(0),
    )?;
    let restrictions: Value = serde_json::from_str(&restrictions_json).unwrap_or_else(|_| json!({}));
    if let Some(providers) = restrictions.get("providers").and_then(Value::as_array) {
        let allowed = providers.iter().filter_map(Value::as_str).collect::<BTreeSet<_>>();
        for provider in &policy.provider_order {
            ensure!(allowed.contains(provider.as_str()), "agent model restrictions deny a policy provider");
        }
    }
    if restrictions.get("remote_context").and_then(Value::as_bool) == Some(false) {
        ensure!(!policy.provider_order.iter().any(|provider| provider == "openrouter"), "agent model restrictions deny remote context");
    }
    if let Some(models) = restrictions.get("models").and_then(Value::as_array) {
        let allowed = models.iter().filter_map(Value::as_str).collect::<BTreeSet<_>>();
        for model in &policy.allowed_models {
            ensure!(allowed.contains(model.as_str()), "agent model restrictions deny a policy model");
        }
    }
    Ok(())
}

fn ensure_no_emergency_stop(
    connection: &Connection,
    agent_id: &str,
    wrapper_id: &str,
    connection_id: &str,
) -> Result<()> {
    let active: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_emergency_stops WHERE state='active' AND (expires_at_utc IS NULL OR expires_at_utc>?4) AND (scope_type='global' OR (scope_type='agent' AND agent_id=?1) OR (scope_type='wrapper' AND wrapper_id=?2) OR (scope_type='connection' AND connection_id=?3))",
        params![agent_id,wrapper_id,connection_id,now_utc()],
        |row| row.get(0),
    )?;
    ensure!(active == 0, "model inference is blocked by an active emergency stop");
    Ok(())
}

async fn resolve_model(
    state: Arc<AppState>,
    provider: &str,
    requested_model: Option<&str>,
    policy: &PolicyRecord,
) -> Result<String> {
    match provider {
        "ollama" => {
            let snapshot = model_center::snapshot(state).await?;
            ensure!(snapshot.runtime.state == "running", "Ollama runtime is not running");
            let default = snapshot
                .settings
                .default_chat_model
                .clone()
                .or_else(|| snapshot.installed_models.first().map(|model| model.name.clone()))
                .context("no local chat model is installed")?;
            let requested = requested_model
                .and_then(|value| value.strip_prefix("ollama:").or_else(|| if value.starts_with("openrouter:") { None } else { Some(value) }))
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let model = requested.unwrap_or(&default).to_owned();
            ensure!(snapshot.installed_models.iter().any(|installed| installed.name == model), "requested local model is not installed");
            enforce_allowed_model(policy, provider, &model, &default, requested.is_some())?;
            Ok(model)
        }
        "openrouter" => {
            let provider_state = state.clone();
            let snapshot = tokio::task::spawn_blocking(move || openrouter_provider::snapshot(&provider_state))
                .await
                .context("OpenRouter snapshot task failed")??;
            ensure!(snapshot.enabled && snapshot.allow_remote_context && snapshot.api_key_configured, "OpenRouter is not ready");
            let default = snapshot.default_model.context("OpenRouter default model is not configured")?;
            let requested = requested_model
                .and_then(|value| value.strip_prefix("openrouter:"))
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let model = requested.unwrap_or(&default).to_owned();
            enforce_allowed_model(policy, provider, &model, &default, requested.is_some())?;
            Ok(model)
        }
        _ => bail!("unsupported model provider"),
    }
}

fn enforce_allowed_model(
    policy: &PolicyRecord,
    provider: &str,
    model: &str,
    provider_default: &str,
    was_explicit: bool,
) -> Result<()> {
    let qualified = format!("{provider}:{model}");
    if policy.allowed_models.is_empty() {
        ensure!(model == provider_default, "policy allows only the configured provider default model");
        ensure!(!was_explicit || model == provider_default, "explicit model is not allowed by policy");
    } else {
        ensure!(policy.allowed_models.contains(&qualified), "model is not allowed by policy");
    }
    Ok(())
}

fn effective_provider_order(
    policy: &PolicyRecord,
    request: &GovernedInferenceRequest,
) -> Result<Vec<String>> {
    let mut providers = policy.provider_order.clone();
    if let Some(preference) = request.provider_preference.as_deref() {
        let preference = choice(preference, PROVIDERS, "provider preference")?;
        ensure!(providers.contains(&preference), "preferred provider is not allowed by policy");
        providers.retain(|provider| provider == &preference);
    } else if let Some(model) = request.model.as_deref() {
        if model.starts_with("openrouter:") {
            ensure!(providers.iter().any(|provider| provider == "openrouter"), "OpenRouter model is not allowed by policy");
            providers.retain(|provider| provider == "openrouter");
        } else if model.starts_with("ollama:") {
            ensure!(providers.iter().any(|provider| provider == "ollama"), "Ollama model is not allowed by policy");
            providers.retain(|provider| provider == "ollama");
        }
    }
    if !policy.allow_fallback {
        providers.truncate(1);
    }
    ensure!(!providers.is_empty(), "no authorized model provider remains");
    Ok(providers)
}

fn policy_usage(connection: &Connection, policy: &PolicyRecord) -> Result<(u64, u64, u64)> {
    let start = Utc::now() - Duration::seconds(i64::from(policy.window_seconds));
    let (requests, tokens, spend): (i64, i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(total_tokens),0),COALESCE(SUM(reported_cost_microusd),0) FROM model_inference_receipts WHERE policy_id=?1 AND completed_at_utc>=?2",
        params![policy.policy_id,timestamp(start)],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    let active: i64 = connection.query_row(
        "SELECT COUNT(*) FROM model_inference_requests WHERE policy_id=?1 AND state IN ('reserved','running') AND created_at_utc>=?2",
        params![policy.policy_id,timestamp(start)],
        |row| row.get(0),
    )?;
    Ok(((requests + active).max(0) as u64,tokens.max(0) as u64,spend.max(0) as u64))
}

fn normalize_inference_request(request: &mut GovernedInferenceRequest) -> Result<()> {
    request.actor_type = choice(&request.actor_type, ACTOR_TYPES, "inference actor type")?;
    request.actor_id = bounded_text(&request.actor_id, 1, 180, "inference actor ID")?;
    request.purpose = bounded_text(&request.purpose, 1, 500, "inference purpose")?;
    request.data_classification = bounded_text(&request.data_classification, 1, 80, "data classification")?;
    request.idempotency_key = bounded_text(&request.idempotency_key, 16, 240, "inference idempotency key")?;
    request.context_hash = bounded_hash(&request.context_hash, "inference context hash")?;
    request.prompt = bounded_text(&request.prompt, 1, 30_000, "inference prompt")?;
    request.agent_id = request.agent_id.as_deref().map(|value| validate_uuid(value, "agent ID")).transpose()?;
    request.assignment_id = request.assignment_id.as_deref().map(|value| validate_uuid(value, "assignment ID")).transpose()?;
    request.policy_id = request.policy_id.as_deref().map(|value| validate_uuid(value, "policy ID")).transpose()?;
    request.privacy_selector_id = request.privacy_selector_id.as_deref().map(|value| validate_uuid(value, "privacy selector ID")).transpose()?;
    if let Some(provider) = request.provider_preference.as_deref() {
        request.provider_preference = Some(choice(provider, PROVIDERS, "provider preference")?);
    }
    if let Some(model) = request.model.as_deref() {
        request.model = Some(bounded_text(model, 1, 240, "model ID")?);
    }
    if let Some(tokens) = request.max_output_tokens {
        ensure!((16..=4_096).contains(&tokens), "requested output-token limit is invalid");
    }
    Ok(())
}

fn normalize_classes(values: &[String]) -> Result<Vec<String>> {
    ensure!(!values.is_empty() && values.len() <= 16, "policy data classes are invalid");
    let mut set = BTreeSet::new();
    for value in values {
        let value = bounded_text(value, 1, 80, "data classification")?;
        ensure!(!NEVER_MODEL_CLASSES.contains(&value.as_str()), "secret data may not be authorized for model inference");
        set.insert(value);
    }
    Ok(set.into_iter().collect())
}

fn normalize_providers(values: &[String]) -> Result<Vec<String>> {
    ensure!(!values.is_empty() && values.len() <= PROVIDERS.len(), "policy provider order is invalid");
    let mut result = Vec::new();
    for value in values {
        let provider = choice(value, PROVIDERS, "model provider")?;
        ensure!(!result.contains(&provider), "policy provider order contains duplicates");
        result.push(provider);
    }
    Ok(result)
}

fn normalize_models(values: &[String], providers: &[String]) -> Result<Vec<String>> {
    ensure!(values.len() <= 64, "policy contains too many models");
    let mut set = BTreeSet::new();
    for value in values {
        let model = bounded_text(value, 3, 240, "allowed model")?;
        let (provider, model_id) = model.split_once(':').context("allowed models must be provider-qualified")?;
        ensure!(providers.iter().any(|candidate| candidate == provider), "allowed model provider is not in policy order");
        ensure!(!model_id.trim().is_empty(), "allowed model ID is empty");
        set.insert(model);
    }
    Ok(set.into_iter().collect())
}

fn existing_request(connection: &Connection, idempotency_key: &str) -> Result<Option<(String, String, String)>> {
    connection
        .query_row(
            "SELECT request_id,request_hash,state FROM model_inference_requests WHERE idempotency_key=?1",
            params![idempotency_key],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        )
        .optional()
        .map_err(Into::into)
}

fn ensure_request_running(connection: &Connection, request_id: &str) -> Result<()> {
    let state: String = connection.query_row(
        "SELECT state FROM model_inference_requests WHERE request_id=?1",
        params![request_id],
        |row| row.get(0),
    )?;
    ensure!(matches!(state.as_str(), "reserved" | "running"), "inference request is no longer executable");
    Ok(())
}

fn load_completed_result(connection: &Connection, request_id: &str) -> Result<GovernedInferenceResult> {
    connection.query_row(
        "SELECT r.receipt_id,r.provider_key,r.model_id,p.output_text,p.output_hash,r.prompt_tokens,r.completion_tokens,r.total_tokens,r.reported_cost_microusd,r.policy_id,r.policy_revision,r.authority_hash FROM model_inference_receipts r JOIN model_inference_private_results p ON p.request_id=r.request_id WHERE r.request_id=?1 AND r.outcome='completed'",
        params![request_id],
        |row| {
            Ok(GovernedInferenceResult {
                request_id: request_id.to_owned(),
                receipt_id: row.get(0)?,
                provider_key: row.get::<_,Option<String>>(1)?.unwrap_or_default(),
                model_id: row.get::<_,Option<String>>(2)?.unwrap_or_default(),
                output: row.get(3)?,
                output_hash: row.get(4)?,
                prompt_tokens: nonnegative_u64(row.get(5)?),
                completion_tokens: nonnegative_u64(row.get(6)?),
                total_tokens: nonnegative_u64(row.get(7)?),
                reported_cost_microusd: nonnegative_u64(row.get(8)?),
                policy_id: row.get(9)?,
                policy_revision: positive_u64(row.get(10)?),
                authority_hash: row.get(11)?,
            })
        },
    ).map_err(Into::into)
}

fn reconcile_interrupted(connection: &Connection) -> Result<()> {
    let request_ids = {
        let mut statement = connection.prepare(
            "SELECT request_id FROM model_inference_requests WHERE state IN ('reserved','running') ORDER BY created_at_utc,request_id",
        )?;
        statement
            .query_map([], |row| row.get::<_,String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for request_id in request_ids {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE model_inference_attempts SET state='interrupted',failure_code='service_restarted',completed_at_utc=?1 WHERE request_id=?2 AND state='running'",
            params![now_utc(),request_id],
        )?;
        transaction.execute(
            "UPDATE model_inference_requests SET state='interrupted',failure_code='service_restarted',completed_at_utc=?1 WHERE request_id=?2 AND state IN ('reserved','running')",
            params![now_utc(),request_id],
        )?;
        finalize_one_terminal_request_tx(&transaction, &request_id)?;
        transaction.commit()?;
    }
    Ok(())
}

fn finalize_unreceipted_terminal_requests_tx(transaction: &Transaction<'_>) -> Result<()> {
    let request_ids = {
        let mut statement = transaction.prepare(
            "SELECT r.request_id FROM model_inference_requests r LEFT JOIN model_inference_receipts x ON x.request_id=r.request_id WHERE r.state IN ('failed','cancelled','interrupted') AND x.request_id IS NULL ORDER BY r.created_at_utc,r.request_id",
        )?;
        statement
            .query_map([], |row| row.get::<_,String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for request_id in request_ids {
        finalize_one_terminal_request_tx(transaction, &request_id)?;
    }
    Ok(())
}

fn finalize_one_terminal_request_tx(transaction: &Transaction<'_>, request_id: &str) -> Result<()> {
    let row = transaction.query_row(
        "SELECT subject_type,subject_id,agent_id,assignment_id,wrapper_id,connection_id,policy_id,policy_revision,purpose_hash,data_classification,state,COALESCE(failure_code,'service_restarted'),request_hash,authority_hash,prompt_hash,context_hash,completed_at_utc FROM model_inference_requests WHERE request_id=?1",
        params![request_id],
        |row| Ok((
            row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?,
            row.get::<_,Option<String>>(4)?,row.get::<_,Option<String>>(5)?,row.get::<_,String>(6)?,row.get::<_,i64>(7)?,
            row.get::<_,String>(8)?,row.get::<_,String>(9)?,row.get::<_,String>(10)?,row.get::<_,String>(11)?,
            row.get::<_,String>(12)?,row.get::<_,String>(13)?,row.get::<_,String>(14)?,row.get::<_,String>(15)?,row.get::<_,Option<String>>(16)?
        )),
    )?;
    let outcome = match row.10.as_str() {
        "cancelled" => "cancelled",
        "interrupted" => "interrupted",
        _ => "failed",
    };
    let completed_at = row.16.unwrap_or_else(now_utc);
    let receipt_id = Uuid::new_v4().to_string();
    let document = json!({
        "schema":"homeserver.model-inference-receipt.v1","receipt_id":receipt_id,"request_id":request_id,
        "subject_type":row.0,"subject_id":row.1,"agent_id":row.2,"assignment_id":row.3,"wrapper_id":row.4,
        "connection_id":row.5,"policy_id":row.6,"policy_revision":row.7,"purpose_hash":row.8,
        "data_classification":row.9,"outcome":outcome,"result_code":row.11,"request_hash":row.12,
        "authority_hash":row.13,"prompt_hash":row.14,"context_hash":row.15,"completed_at_utc":completed_at
    });
    let receipt_hash = hash_json(&document)?;
    transaction.execute(
        "INSERT INTO model_inference_receipts (receipt_id,request_id,subject_type,subject_id,agent_id,assignment_id,wrapper_id,connection_id,policy_id,policy_revision,purpose_hash,data_classification,outcome,result_code,request_hash,authority_hash,prompt_hash,context_hash,prompt_tokens,completion_tokens,total_tokens,reported_cost_microusd,receipt_hash,completed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,0,0,0,0,?19,?20)",
        params![receipt_id,request_id,row.0,row.1,row.2,row.3,row.4,row.5,row.6,row.7,row.8,row.9,outcome,row.11,row.12,row.13,row.14,row.15,receipt_hash,completed_at],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_receipt_tx(
    transaction: &Transaction<'_>,
    request_id: &str,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    authority_hash: &str,
    request_hash: &str,
    prompt_hash: &str,
    provider: Option<&str>,
    model: Option<&str>,
    outcome: &str,
    result_code: &str,
    result_hash: Option<&str>,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    reported_cost_microusd: u64,
    completed_at: &str,
) -> Result<String> {
    if let Some(existing) = transaction
        .query_row(
            "SELECT receipt_id FROM model_inference_receipts WHERE request_id=?1",
            params![request_id],
            |row| row.get::<_,String>(0),
        )
        .optional()?
    {
        return Ok(existing);
    }
    let receipt_id = Uuid::new_v4().to_string();
    let document = json!({
        "schema":"homeserver.model-inference-receipt.v1","receipt_id":receipt_id,"request_id":request_id,
        "subject_type":policy.subject_type,"subject_id":policy.subject_id,"agent_id":policy.agent_id,
        "assignment_id":policy.assignment_id,"wrapper_id":policy.wrapper_id,"connection_id":policy.connection_id,
        "policy_id":policy.policy_id,"policy_revision":policy.policy_revision,"purpose_hash":policy.purpose_hash,
        "data_classification":request.data_classification,"provider":provider,"model":model,"outcome":outcome,
        "result_code":result_code,"request_hash":request_hash,"authority_hash":authority_hash,
        "prompt_hash":prompt_hash,"context_hash":request.context_hash,"result_hash":result_hash,
        "prompt_tokens":prompt_tokens,"completion_tokens":completion_tokens,"total_tokens":total_tokens,
        "reported_cost_microusd":reported_cost_microusd,"completed_at_utc":completed_at
    });
    let receipt_hash = hash_json(&document)?;
    transaction.execute(
        "INSERT INTO model_inference_receipts (receipt_id,request_id,subject_type,subject_id,agent_id,assignment_id,wrapper_id,connection_id,policy_id,policy_revision,purpose_hash,data_classification,provider_key,model_id,outcome,result_code,request_hash,authority_hash,prompt_hash,context_hash,result_hash,prompt_tokens,completion_tokens,total_tokens,reported_cost_microusd,receipt_hash,completed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)",
        params![
            receipt_id,request_id,policy.subject_type,policy.subject_id,policy.agent_id,policy.assignment_id,
            policy.wrapper_id,policy.connection_id,policy.policy_id,policy.policy_revision as i64,policy.purpose_hash,
            request.data_classification,provider,model,outcome,result_code,request_hash,authority_hash,prompt_hash,
            request.context_hash,result_hash,prompt_tokens as i64,completion_tokens as i64,total_tokens as i64,
            reported_cost_microusd as i64,receipt_hash,completed_at
        ],
    )?;
    Ok(receipt_id)
}

fn read_policies(connection: &Connection) -> Result<Vec<RoutingPolicySummary>> {
    let mut statement = connection.prepare(
        "SELECT policy_id,subject_type,subject_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,purpose,purpose_hash,allowed_data_classes_json,provider_order_json,allowed_models_json,allow_fallback,remote_context_mode,require_zdr,max_input_chars,max_output_tokens,window_seconds,max_requests,max_total_tokens,max_spend_microusd,policy_revision,policy_hash,state,created_by_user_id,reason,not_before_utc,expires_at_utc,created_at_utc,updated_at_utc FROM model_routing_policies ORDER BY updated_at_utc DESC,policy_revision DESC,policy_id DESC LIMIT 500",
    )?;
    statement
        .query_map([], map_policy)?
        .map(|row| row.map(policy_summary))
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_receipts(connection: &Connection) -> Result<Vec<InferenceReceiptSummary>> {
    let mut statement = connection.prepare(
        "SELECT receipt_id,request_id,subject_type,subject_id,agent_id,assignment_id,wrapper_id,connection_id,policy_id,policy_revision,purpose_hash,data_classification,provider_key,model_id,outcome,result_code,request_hash,authority_hash,prompt_hash,context_hash,result_hash,prompt_tokens,completion_tokens,total_tokens,reported_cost_microusd,receipt_hash,completed_at_utc FROM model_inference_receipts ORDER BY completed_at_utc DESC,receipt_id DESC LIMIT 500",
    )?;
    statement
        .query_map([], |row| {
            Ok(InferenceReceiptSummary {
                receipt_id: row.get(0)?,request_id: row.get(1)?,subject_type: row.get(2)?,subject_id: row.get(3)?,
                agent_id: row.get(4)?,assignment_id: row.get(5)?,wrapper_id: row.get(6)?,connection_id: row.get(7)?,
                policy_id: row.get(8)?,policy_revision: positive_u64(row.get(9)?),purpose_hash: row.get(10)?,
                data_classification: row.get(11)?,provider_key: row.get(12)?,model_id: row.get(13)?,outcome: row.get(14)?,
                result_code: row.get(15)?,request_hash: row.get(16)?,authority_hash: row.get(17)?,prompt_hash: row.get(18)?,
                context_hash: row.get(19)?,result_hash: row.get(20)?,prompt_tokens: nonnegative_u64(row.get(21)?),
                completion_tokens: nonnegative_u64(row.get(22)?),total_tokens: nonnegative_u64(row.get(23)?),
                reported_cost_microusd: nonnegative_u64(row.get(24)?),receipt_hash: row.get(25)?,completed_at_utc: row.get(26)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn read_events(connection: &Connection) -> Result<Vec<InferenceEventSummary>> {
    let mut statement = connection.prepare(
        "SELECT event_id,request_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc FROM model_inference_events ORDER BY created_at_utc DESC,event_id DESC LIMIT 500",
    )?;
    statement
        .query_map([], |row| {
            let metadata: String = row.get(8)?;
            Ok(InferenceEventSummary {
                event_id: row.get(0)?,request_id: row.get(1)?,policy_id: row.get(2)?,event_type: row.get(3)?,
                outcome: row.get(4)?,actor_type: row.get(5)?,actor_id: row.get(6)?,detail_code: row.get(7)?,
                metadata: serde_json::from_str(&metadata).unwrap_or_else(|_| json!({})),event_hash: row.get(9)?,created_at_utc: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn map_policy(row: &Row<'_>) -> rusqlite::Result<PolicyRecord> {
    let classes: String = row.get(12)?;
    let providers: String = row.get(13)?;
    let models: String = row.get(14)?;
    Ok(PolicyRecord {
        policy_id: row.get(0)?,subject_type: row.get(1)?,subject_id: row.get(2)?,agent_id: row.get(3)?,
        agent_revision: row.get::<_,Option<i64>>(4)?.map(positive_u64),assignment_id: row.get(5)?,
        assignment_revision: row.get::<_,Option<i64>>(6)?.map(positive_u64),wrapper_id: row.get(7)?,connection_id: row.get(8)?,
        connection_authority_revision: row.get::<_,Option<i64>>(9)?.map(nonnegative_u64),purpose: row.get(10)?,purpose_hash: row.get(11)?,
        allowed_data_classes: serde_json::from_str(&classes).unwrap_or_default(),provider_order: serde_json::from_str(&providers).unwrap_or_default(),
        allowed_models: serde_json::from_str(&models).unwrap_or_default(),allow_fallback: row.get::<_,i64>(15)? == 1,
        remote_context_mode: row.get(16)?,require_zdr: row.get::<_,i64>(17)? == 1,max_input_chars: row.get::<_,i64>(18)?.max(1) as u32,
        max_output_tokens: row.get::<_,i64>(19)?.clamp(16,4096) as u32,window_seconds: row.get::<_,i64>(20)?.max(60) as u32,
        max_requests: nonnegative_u64(row.get(21)?),max_total_tokens: nonnegative_u64(row.get(22)?),max_spend_microusd: nonnegative_u64(row.get(23)?),
        policy_revision: positive_u64(row.get(24)?),policy_hash: row.get(25)?,state: row.get(26)?,created_by_user_id: row.get(27)?,
        reason: row.get(28)?,not_before_utc: row.get(29)?,expires_at_utc: row.get(30)?,created_at_utc: row.get(31)?,updated_at_utc: row.get(32)?,
    })
}

fn policy_summary(policy: PolicyRecord) -> RoutingPolicySummary {
    RoutingPolicySummary {
        policy_id: policy.policy_id,subject_type: policy.subject_type,subject_id: policy.subject_id,agent_id: policy.agent_id,
        agent_revision: policy.agent_revision,assignment_id: policy.assignment_id,assignment_revision: policy.assignment_revision,
        wrapper_id: policy.wrapper_id,connection_id: policy.connection_id,connection_authority_revision: policy.connection_authority_revision,
        purpose: policy.purpose,allowed_data_classes: policy.allowed_data_classes,provider_order: policy.provider_order,
        allowed_models: policy.allowed_models,allow_fallback: policy.allow_fallback,remote_context_mode: policy.remote_context_mode,
        require_zdr: policy.require_zdr,max_input_chars: policy.max_input_chars,max_output_tokens: policy.max_output_tokens,
        window_seconds: policy.window_seconds,max_requests: policy.max_requests,max_total_tokens: policy.max_total_tokens,
        max_spend_microusd: policy.max_spend_microusd,policy_revision: policy.policy_revision,policy_hash: policy.policy_hash,
        state: policy.state,created_by_user_id: policy.created_by_user_id,reason: policy.reason,not_before_utc: policy.not_before_utc,
        expires_at_utc: policy.expires_at_utc,created_at_utc: policy.created_at_utc,updated_at_utc: policy.updated_at_utc,
    }
}

#[allow(clippy::too_many_arguments)]
fn record_event(
    connection: &Connection,
    request_id: Option<&str>,
    policy_id: Option<&str>,
    event_type: &str,
    outcome: &str,
    actor_type: &str,
    actor_id: &str,
    detail_code: &str,
    metadata: Value,
) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    record_event_tx(&transaction,request_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,metadata)?;
    transaction.commit()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_event_tx(
    transaction: &Transaction<'_>,
    request_id: Option<&str>,
    policy_id: Option<&str>,
    event_type: &str,
    outcome: &str,
    actor_type: &str,
    actor_id: &str,
    detail_code: &str,
    metadata: Value,
) -> Result<()> {
    let event_id = Uuid::new_v4().to_string();
    let created_at = now_utc();
    let document = json!({
        "schema":"homeserver.model-inference-event.v1","event_id":event_id,"request_id":request_id,"policy_id":policy_id,
        "event_type":event_type,"outcome":outcome,"actor_type":actor_type,"actor_id":actor_id,
        "detail_code":detail_code,"metadata":metadata,"created_at_utc":created_at
    });
    let event_hash = hash_json(&document)?;
    transaction.execute(
        "INSERT INTO model_inference_events (event_id,request_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![event_id,request_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,serde_json::to_string(&metadata)?,event_hash,created_at],
    )?;
    Ok(())
}

fn bounded_text(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    let count = value.chars().count();
    ensure!(count >= minimum && count <= maximum, "{label} length is invalid");
    ensure!(!value.contains('\0'), "{label} contains an invalid character");
    Ok(value.to_owned())
}

fn bounded_hash(value: &str, label: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()), "{label} is invalid");
    Ok(value)
}

fn choice(value: &str, allowed: &[&str], label: &str) -> Result<String> {
    let value = bounded_text(value, 1, 160, label)?.to_ascii_lowercase();
    ensure!(allowed.contains(&value.as_str()), "{label} is not allowed");
    Ok(value)
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    Uuid::parse_str(value)
        .with_context(|| format!("{label} is invalid"))
        .map(|value| value.to_string())
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

fn hash_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(hash_text(&serde_json::to_string(value)?))
}

fn hash_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn positive_u64(value: i64) -> u64 {
    value.max(1) as u64
}

fn nonnegative_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn public_failure_code(error: &anyhow::Error) -> String {
    let text = error.to_string().to_ascii_lowercase();
    if text.contains("emergency stop") {
        "inference_emergency_stop"
    } else if text.contains("policy") || text.contains("authority") || text.contains("assignment") {
        "inference_authority_changed"
    } else if text.contains("selector") || text.contains("remote context") || text.contains("zdr") {
        "inference_remote_context_denied"
    } else if text.contains("budget") || text.contains("spending") || text.contains("token") {
        "inference_budget_reached"
    } else if text.contains("cancel") {
        "inference_cancelled"
    } else if text.contains("timeout") {
        "inference_timeout"
    } else if text.contains("model") || text.contains("ollama") || text.contains("openrouter") {
        "inference_provider_failed"
    } else {
        "inference_failed"
    }
    .to_owned()
}

fn api_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError { ok: false, error: code, message: error.to_string() }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_order_is_explicit_and_deduplicated() {
        assert_eq!(normalize_providers(&["ollama".to_owned()]).unwrap(), vec!["ollama"]);
        assert!(normalize_providers(&["ollama".to_owned(), "ollama".to_owned()]).is_err());
    }

    #[test]
    fn secret_classification_is_never_model_authority() {
        assert!(normalize_classes(&["secret".to_owned()]).is_err());
    }

    #[test]
    fn hashes_are_lowercase_sha256() {
        assert_eq!(hash_text("phase20").len(), 64);
        assert!(hash_text("phase20").chars().all(|character| character.is_ascii_hexdigit()));
    }
}
