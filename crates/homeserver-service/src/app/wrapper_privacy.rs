use super::wrapper_jobs;
use crate::{semantic_vault, AppState};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0024_private_knowledge_boundary.sql");
const MIGRATION_KEY: &str = "0024_private_knowledge_boundary";
const MAX_CONTROL_BODY_BYTES: usize = 1024 * 1024;
const MAX_SELECTOR_RESOURCES: usize = 500;
const MAX_EVENTS: i64 = 50_000;
const SCAN_VERSION: &str = "homeserver-private-egress-v1";
const FILTER_VERSION: &str = "wrapper-private-egress-v1";
const KNOWLEDGE_CAPABILITIES: &[&str] = &["knowledge.search", "knowledge.result.read"];
const DATA_CLASSES: &[&str] = &[
    "secret",
    "private_source",
    "private_derived",
    "private_selector",
    "shared_approved",
    "wrapper_owned",
    "public",
    "safe_receipt",
    "security_metadata",
];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}
type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DataClassSummary {
    pub class_key: String,
    pub description: String,
    pub sensitivity_tier: String,
    pub wrapper_egress_mode: String,
    pub default_retention_days: u32,
    pub state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivateResourceSummary {
    pub resource_id: String,
    pub resource_namespace: String,
    pub resource_type: String,
    pub local_source_id: String,
    pub local_display_name: String,
    pub source_hash: Option<String>,
    pub state: String,
    pub resource_revision: u64,
    pub class_key: String,
    pub classification_revision: u64,
    pub updated_at_utc: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelectorSummary {
    pub selector_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub grant_id: String,
    pub grant_revision: u64,
    pub selector_revision: u64,
    pub agent_id: Option<String>,
    pub agent_revision: Option<u64>,
    pub resource_namespace: String,
    pub resource_type: String,
    pub allowed_operations: Vec<String>,
    pub maximum_items: u32,
    pub maximum_source_bytes: u64,
    pub purpose: String,
    pub purpose_hash: String,
    pub output_schema: String,
    pub allow_citations: bool,
    pub remote_model_mode: String,
    pub approved_remote_provider: Option<String>,
    pub egress_approval_mode: String,
    pub state: String,
    pub expires_at_utc: String,
    pub resource_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EgressDecisionSummary {
    pub decision_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub job_id: String,
    pub selector_id: Option<String>,
    pub grant_id: String,
    pub grant_revision: u64,
    pub output_schema: String,
    pub input_classes: Vec<String>,
    pub output_classes: Vec<String>,
    pub policy: String,
    pub state: String,
    pub detail_code: String,
    pub approval_required: bool,
    pub output_hash: Option<String>,
    pub private_evidence_hash: String,
    pub scan_version: String,
    pub created_at_utc: String,
    pub decided_at_utc: Option<String>,
    pub delivered_at_utc: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyIncidentSummary {
    pub incident_id: String,
    pub wrapper_id: Option<String>,
    pub connection_id: Option<String>,
    pub job_id: Option<String>,
    pub selector_id: Option<String>,
    pub severity: String,
    pub category: String,
    pub detail_code: String,
    pub evidence_hash: String,
    pub state: String,
    pub detected_at_utc: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacySnapshot {
    pub schema: String,
    pub data_classes: Vec<DataClassSummary>,
    pub resources: Vec<PrivateResourceSummary>,
    pub selectors: Vec<SelectorSummary>,
    pub decisions: Vec<EgressDecisionSummary>,
    pub incidents: Vec<PrivacyIncidentSummary>,
    pub private_sources_exposed: bool,
    pub local_paths_exposed: bool,
    pub destination_specific_aliases: bool,
    pub fail_closed: bool,
    pub pairing_implies_private_authority: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionSnapshotRequest {
    pub connection_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct ClassifyResourceRequest {
    pub resource_id: String,
    pub class_key: String,
    pub actor_user_id: String,
    pub reason: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CreateSelectorRequest {
    pub connection_id: String,
    pub grant_id: String,
    pub agent_id: Option<String>,
    pub resource_namespace: String,
    pub resource_type: String,
    pub allowed_operations: Vec<String>,
    pub resource_ids: Vec<String>,
    pub maximum_items: Option<u32>,
    pub maximum_source_bytes: Option<u64>,
    pub purpose: String,
    pub output_schema: String,
    pub allow_citations: Option<bool>,
    pub remote_model_mode: Option<String>,
    pub approved_remote_provider: Option<String>,
    pub egress_approval_mode: Option<String>,
    pub created_by_user_id: String,
    pub reason: String,
    pub expires_minutes: u32,
}
#[derive(Debug, Clone, Deserialize)]
pub struct RevokeSelectorRequest {
    pub selector_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewEgressRequest {
    pub decision_id: String,
    pub output_hash: String,
    pub actor_user_id: String,
    pub decision: String,
    pub confirmation: String,
    pub reason: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct PurgeCacheRequest {
    pub connection_id: Option<String>,
    pub selector_id: Option<String>,
    pub confirmation: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct PrivateSearchRequest {
    pub worker_id: String,
    pub job_id: String,
    pub lease_token: String,
    pub query: String,
    pub mode: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrivateSearchHit {
    pub resource_id: String,
    pub title: String,
    pub snippet: String,
    pub page_number: Option<u32>,
    pub combined_score: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrivateSearchResult {
    pub job_id: String,
    pub selector_id: String,
    pub query_hash: String,
    pub hits: Vec<PrivateSearchHit>,
    pub source_count: u32,
    pub private_only: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivacySubmissionBinding {
    selector_id: String,
    selector_revision: u64,
    purpose_hash: String,
    output_schema: String,
    remote_model_provider: Option<String>,
    classification_set_hash: String,
}
#[derive(Debug, Clone)]
pub(crate) struct EgressContext<'a> {
    pub job_id: &'a str,
    pub wrapper_id: &'a str,
    pub connection_id: &'a str,
    pub grant_id: &'a str,
    pub grant_revision: u64,
    pub connection_authority_revision: u64,
    pub capability_key: &'a str,
}
#[derive(Debug, Clone)]
pub(crate) struct EgressEvaluation {
    pub decision_id: String,
    pub state: String,
    pub detail_code: String,
    pub safe_result: Option<Value>,
    pub output_hash: Option<String>,
    pub filter_version: &'static str,
    pub approval_required: bool,
}
#[derive(Debug, Clone)]
struct SelectorAuthority {
    selector_id: String,
    wrapper_id: String,
    connection_id: String,
    grant_id: String,
    grant_revision: u64,
    selector_revision: u64,
    agent_id: Option<String>,
    agent_revision: Option<u64>,
    allowed_operations: Vec<String>,
    maximum_items: u32,
    maximum_source_bytes: u64,
    purpose_hash: String,
    output_schema: String,
    allow_citations: bool,
    remote_model_mode: String,
    approved_remote_provider: Option<String>,
    egress_approval_mode: String,
    expires_at_utc: String,
}
#[derive(Debug)]
struct ScanOutcome {
    value: Value,
    redactions: Vec<Redaction>,
    denied_category: Option<String>,
}
#[derive(Debug)]
struct Redaction {
    category: String,
    json_path_hash: String,
    match_hash: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    expire_and_reconcile(connection)?;
    maintain_history(connection)?;
    health_check(connection)?;
    wrapper_jobs::reconcile_authority(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        count == 1,
        "private knowledge migration is not registered exactly once"
    );
    for table in [
        "data_classification_catalog",
        "private_resource_catalog",
        "private_resource_classifications",
        "private_resource_selectors",
        "private_selector_resources",
        "private_resource_aliases",
        "wrapper_job_privacy_bindings",
        "private_knowledge_access_receipts",
        "egress_decisions",
        "wrapper_resource_projections",
        "egress_redactions",
        "egress_approvals",
        "private_evidence_records",
        "privacy_boundary_incidents",
        "projection_cache_entries",
        "deletion_propagation_jobs",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    let classes: i64 = connection.query_row(
        "SELECT COUNT(*) FROM data_classification_catalog WHERE state='active'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        classes == DATA_CLASSES.len() as i64,
        "private data-class catalog is incomplete"
    );
    let unclassified: i64 = connection.query_row("SELECT COUNT(*) FROM private_resource_catalog r LEFT JOIN private_resource_classifications c ON c.resource_id=r.resource_id AND c.state='active' WHERE r.state<>'deleted' AND c.resource_id IS NULL", [], |row| row.get(0))?;
    ensure!(
        unclassified == 0,
        "private resources are missing classifications"
    );
    let cross_wrapper: i64 = connection.query_row("SELECT COUNT(*) FROM private_resource_selectors s JOIN wrapper_connections c ON c.connection_id=s.connection_id JOIN wrapper_capability_grants g ON g.grant_id=s.grant_id WHERE c.wrapper_id<>s.wrapper_id OR g.wrapper_id<>s.wrapper_id OR g.connection_id<>s.connection_id", [], |row| row.get(0))?;
    ensure!(
        cross_wrapper == 0,
        "private selectors contain cross-wrapper authority"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    expire_and_reconcile(connection)?;
    process_deletion_queue(connection)?;
    connection.execute("DELETE FROM private_knowledge_access_receipts WHERE created_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')", [])?;
    connection.execute("DELETE FROM privacy_boundary_incidents WHERE state IN ('resolved','dismissed') AND detected_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')", [])?;
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM egress_decisions", [], |row| {
        row.get(0)
    })?;
    ensure!(
        count <= MAX_EVENTS,
        "privacy decision retention requires archival"
    );
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/privacy", get(snapshot_handler))
        .route(
            "/v1/privacy/connection-snapshot",
            post(connection_snapshot_handler),
        )
        .route("/v1/privacy/data-classes", get(data_classes_handler))
        .route("/v1/privacy/resources", get(resources_handler))
        .route("/v1/privacy/resources/classify", post(classify_handler))
        .route(
            "/v1/privacy/selectors",
            get(selectors_handler).post(create_selector_handler),
        )
        .route(
            "/v1/privacy/selectors/revoke",
            post(revoke_selector_handler),
        )
        .route("/v1/privacy/egress-decisions", get(decisions_handler))
        .route(
            "/v1/privacy/egress-decisions/review",
            post(review_egress_handler),
        )
        .route("/v1/privacy/cache/purge", post(purge_cache_handler))
        .route("/v1/privacy/incidents", get(incidents_handler))
        .route("/v1/internal/privacy/search", post(private_search_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn snapshot_handler(State(state): State<Arc<AppState>>) -> ApiResult<PrivacySnapshot> {
    run_blocking(move || snapshot(&state, None), "privacy_snapshot_failed").await
}
async fn connection_snapshot_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionSnapshotRequest>,
) -> ApiResult<PrivacySnapshot> {
    run_blocking(
        move || snapshot(&state, request.connection_id.as_deref()),
        "privacy_connection_snapshot_failed",
    )
    .await
}
async fn data_classes_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Vec<DataClassSummary>> {
    run_blocking(
        move || {
            let c = state.connection()?;
            read_data_classes(&c)
        },
        "privacy_data_classes_failed",
    )
    .await
}
async fn resources_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Vec<PrivateResourceSummary>> {
    run_blocking(
        move || {
            let c = state.connection()?;
            read_resources(&c)
        },
        "privacy_resources_failed",
    )
    .await
}
async fn selectors_handler(State(state): State<Arc<AppState>>) -> ApiResult<Vec<SelectorSummary>> {
    run_blocking(
        move || {
            let c = state.connection()?;
            read_selectors(&c, None)
        },
        "privacy_selectors_failed",
    )
    .await
}
async fn decisions_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Vec<EgressDecisionSummary>> {
    run_blocking(
        move || {
            let c = state.connection()?;
            read_decisions(&c, None)
        },
        "privacy_decisions_failed",
    )
    .await
}
async fn incidents_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Vec<PrivacyIncidentSummary>> {
    run_blocking(
        move || {
            let c = state.connection()?;
            read_incidents(&c, None)
        },
        "privacy_incidents_failed",
    )
    .await
}
async fn classify_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ClassifyResourceRequest>,
) -> ApiResult<PrivateResourceSummary> {
    run_blocking(
        move || {
            let c = state.connection()?;
            classify_resource(&c, request)
        },
        "privacy_classification_failed",
    )
    .await
}
async fn create_selector_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateSelectorRequest>,
) -> ApiResult<SelectorSummary> {
    run_blocking(
        move || {
            let c = state.connection()?;
            create_selector(&c, request)
        },
        "privacy_selector_create_failed",
    )
    .await
}
async fn revoke_selector_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RevokeSelectorRequest>,
) -> ApiResult<SelectorSummary> {
    run_blocking(
        move || {
            let c = state.connection()?;
            revoke_selector(&c, request)
        },
        "privacy_selector_revoke_failed",
    )
    .await
}
async fn review_egress_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ReviewEgressRequest>,
) -> ApiResult<EgressDecisionSummary> {
    run_blocking(
        move || {
            let c = state.connection()?;
            review_egress(&c, request)
        },
        "privacy_egress_review_failed",
    )
    .await
}
async fn purge_cache_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PurgeCacheRequest>,
) -> ApiResult<Value> {
    run_blocking(
        move || {
            let c = state.connection()?;
            purge_cache(&c, request)
        },
        "privacy_cache_purge_failed",
    )
    .await
}

async fn private_search_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PrivateSearchRequest>,
) -> ApiResult<PrivateSearchResult> {
    let auth_state = state.clone();
    let auth_request = request.clone();
    let authority = tokio::task::spawn_blocking(move || {
        let c = auth_state.connection()?;
        validate_private_search_authority(&c, &auth_request)
    })
    .await
    .map_err(|e| api_error("privacy_search_task_failed", e.into()))?
    .map_err(|e| api_error("privacy_search_denied", e))?;
    let search = semantic_vault::semantic_search(
        state.clone(),
        semantic_vault::SemanticSearchRequest {
            query: request.query.clone(),
            limit: Some(
                request
                    .limit
                    .unwrap_or(authority.maximum_items)
                    .min(authority.maximum_items),
            ),
            mode: request.mode.clone(),
        },
    )
    .await
    .map_err(|e| api_error("privacy_search_failed", e))?;
    let finish_state = state.clone();
    tokio::task::spawn_blocking(move || {
        let c = finish_state.connection()?;
        finish_private_search(&c, &request, &authority, search)
    })
    .await
    .map_err(|e| api_error("privacy_search_task_failed", e.into()))?
    .map(Json)
    .map_err(|e| api_error("privacy_search_failed", e))
}

async fn run_blocking<T, F>(task: F, code: &'static str) -> ApiResult<T>
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|e| api_error(code, anyhow::anyhow!("privacy task failed: {e}")))?
        .map(Json)
        .map_err(|e| api_error(code, e))
}

pub(crate) fn validate_job_privacy_submission(
    connection: &Connection,
    connection_id: &str,
    grant_id: &str,
    grant_revision: u64,
    capability_key: &str,
    operation: &str,
    submitted_by_type: &str,
    submitted_by_id: &str,
    selector_id: Option<&str>,
    purpose: Option<&str>,
    output_schema: Option<&str>,
    remote_model_provider: Option<&str>,
) -> Result<Option<PrivacySubmissionBinding>> {
    let knowledge = KNOWLEDGE_CAPABILITIES.contains(&capability_key);
    if !knowledge && selector_id.is_none() {
        return Ok(None);
    }
    let selector_id = validate_uuid(
        selector_id.context("private knowledge jobs require an explicit selector")?,
        "selector ID",
    )?;
    let selector = selector_authority(connection, &selector_id)?;
    ensure!(
        selector.connection_id == connection_id,
        "selector belongs to a different connection"
    );
    ensure!(
        selector.grant_id == grant_id && selector.grant_revision == grant_revision,
        "selector grant authority is stale"
    );
    ensure!(
        selector.allowed_operations.iter().any(|v| v == operation),
        "selector operation is denied"
    );
    ensure!(
        parse_utc(&selector.expires_at_utc, "selector expiration")? > Utc::now(),
        "selector expired"
    );
    let purpose = bounded_text(
        purpose.context("private knowledge jobs require a purpose")?,
        1,
        1000,
        "purpose",
    )?;
    ensure!(
        hash_text(&purpose) == selector.purpose_hash,
        "job purpose does not match selector authority"
    );
    let output_schema = validate_symbol(
        output_schema.context("private knowledge jobs require an output schema")?,
        160,
        "output schema",
    )?;
    ensure!(
        output_schema == selector.output_schema,
        "job output schema does not match selector authority"
    );
    validate_remote_model(&selector, remote_model_provider)?;
    if let Some(agent_id) = selector.agent_id.as_deref() {
        ensure!(
            submitted_by_type == "agent" && submitted_by_id == agent_id,
            "selector belongs to a different agent"
        );
        let revision:i64=connection.query_row("SELECT revision FROM homeserver_agents WHERE agent_id=?1 AND state='active' AND expires_at_utc>strftime('%Y-%m-%dT%H:%M:%fZ','now')",params![agent_id],|row|row.get(0))?;
        ensure!(
            Some(revision.max(0) as u64) == selector.agent_revision,
            "selector agent revision is stale"
        );
    }
    Ok(Some(PrivacySubmissionBinding {
        selector_id,
        selector_revision: selector.selector_revision,
        purpose_hash: selector.purpose_hash,
        output_schema,
        remote_model_provider: remote_model_provider.map(ToOwned::to_owned),
        classification_set_hash: classification_set_hash(connection, &selector.selector_id)?,
    }))
}

pub(crate) fn bind_job_privacy_tx(
    transaction: &Transaction<'_>,
    job_id: &str,
    binding: &PrivacySubmissionBinding,
) -> Result<()> {
    transaction.execute("INSERT INTO wrapper_job_privacy_bindings(job_id,selector_id,selector_revision,purpose_hash,output_schema,remote_model_provider,classification_set_hash,created_at_utc) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![job_id,binding.selector_id,binding.selector_revision as i64,binding.purpose_hash,binding.output_schema,binding.remote_model_provider,binding.classification_set_hash,now_utc()])?;
    Ok(())
}

pub(crate) fn job_privacy_authority_is_current_tx(
    transaction: &Transaction<'_>,
    job_id: &str,
    capability_key: &str,
) -> Result<bool> {
    if !table_exists_tx(transaction, "wrapper_job_privacy_bindings")? {
        return Ok(true);
    }
    let binding:Option<(String,i64,String,String,Option<String>,String)>=transaction.query_row("SELECT selector_id,selector_revision,purpose_hash,output_schema,remote_model_provider,classification_set_hash FROM wrapper_job_privacy_bindings WHERE job_id=?1",params![job_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).optional()?;
    if binding.is_none() {
        return Ok(!KNOWLEDGE_CAPABILITIES.contains(&capability_key));
    }
    let (selector_id, revision, purpose_hash, schema, remote_provider, class_hash) =
        binding.context("privacy binding missing")?;
    let selector:Option<(String,i64,String,String,String,Option<String>,String)>=transaction.query_row("SELECT state,selector_revision,purpose_hash,output_schema,remote_model_mode,approved_remote_provider,expires_at_utc FROM private_resource_selectors WHERE selector_id=?1",params![selector_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?))).optional()?;
    let Some((
        state,
        current_revision,
        current_purpose,
        current_schema,
        remote_mode,
        approved,
        expires,
    )) = selector
    else {
        return Ok(false);
    };
    if state != "active"
        || current_revision.max(0) as u64 != revision.max(0) as u64
        || current_purpose != purpose_hash
        || current_schema != schema
        || parse_utc(&expires, "selector expiration")? <= Utc::now()
    {
        return Ok(false);
    }
    if let Some(provider) = remote_provider {
        if remote_mode != "approved_provider" || approved.as_deref() != Some(&provider) {
            return Ok(false);
        }
    }
    Ok(classification_set_hash_tx(transaction, &selector_id)? == class_hash)
}

pub(crate) fn evaluate_egress_tx(
    transaction: &Transaction<'_>,
    context: EgressContext<'_>,
    private_result: &Value,
    initial_safe_result: &Value,
    private_result_hash: &str,
    source_count: u32,
) -> Result<EgressEvaluation> {
    if !table_exists_tx(transaction, "egress_decisions")? {
        return Ok(EgressEvaluation {
            decision_id: String::new(),
            state: "allowed".to_owned(),
            detail_code: "legacy_safe_projection".to_owned(),
            safe_result: Some(initial_safe_result.clone()),
            output_hash: Some(hash_json(initial_safe_result)?),
            filter_version: FILTER_VERSION,
            approval_required: false,
        });
    }
    let binding:Option<(String,String,String)>=transaction.query_row("SELECT selector_id,output_schema,classification_set_hash FROM wrapper_job_privacy_bindings WHERE job_id=?1",params![context.job_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    if binding.is_none() && !KNOWLEDGE_CAPABILITIES.contains(&context.capability_key) {
        return Ok(EgressEvaluation {
            decision_id: String::new(),
            state: "allowed".to_owned(),
            detail_code: "non_knowledge_safe_projection".to_owned(),
            safe_result: Some(initial_safe_result.clone()),
            output_hash: Some(hash_json(initial_safe_result)?),
            filter_version: FILTER_VERSION,
            approval_required: false,
        });
    }
    let (selector_id, output_schema, class_hash) =
        binding.context("knowledge result has no privacy selector binding")?;
    ensure!(
        job_privacy_authority_is_current_tx(transaction, context.job_id, context.capability_key)?,
        "privacy authority changed"
    );
    let selector = selector_authority_tx(transaction, &selector_id)?;
    let scan = scan_for_egress(transaction, &selector, initial_safe_result, "$", 0)?;
    let private_evidence_hash = hash_json(
        &json!({"job_id":context.job_id,"private_result_hash":private_result_hash,"classification_set_hash":class_hash,"scan_version":SCAN_VERSION,"private_result_shape_hash":hash_json(&shape_only(private_result,0)?)?}),
    )?;
    let decision_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let (policy, state, detail_code, safe_result, output_hash, approval_required) =
        if let Some(category) = scan.denied_category.as_deref() {
            record_incident_tx(
                transaction,
                Some(context.wrapper_id),
                Some(context.connection_id),
                Some(context.job_id),
                Some(&selector_id),
                "critical",
                category,
                "egress_content_denied",
                &private_evidence_hash,
            )?;
            (
                "deny",
                "denied",
                format!("{category}_detected"),
                None,
                None,
                false,
            )
        } else {
            let output_hash = hash_json(&scan.value)?;
            if selector.egress_approval_mode == "per_result" {
                (
                    "pending_review",
                    "pending_review",
                    "fresh_egress_approval_required".to_owned(),
                    Some(scan.value),
                    Some(output_hash),
                    true,
                )
            } else if scan.redactions.is_empty() {
                (
                    "allow",
                    "allowed",
                    "egress_allowed".to_owned(),
                    Some(scan.value),
                    Some(output_hash),
                    false,
                )
            } else {
                (
                    "allow_with_redaction",
                    "allowed",
                    "egress_allowed_with_redaction".to_owned(),
                    Some(scan.value),
                    Some(output_hash),
                    false,
                )
            }
        };
    transaction.execute("INSERT INTO egress_decisions(decision_id,wrapper_id,connection_id,job_id,selector_id,grant_id,grant_revision,connection_authority_revision,output_schema,input_classes_json,output_classes_json,policy,state,detail_code,approval_required,output_hash,private_evidence_hash,scan_version,created_at_utc,decided_at_utc) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,CASE WHEN ?13='allowed' THEN ?19 ELSE NULL END)",params![decision_id,context.wrapper_id,context.connection_id,context.job_id,selector_id,context.grant_id,context.grant_revision as i64,context.connection_authority_revision as i64,output_schema,serde_json::to_string(&vec!["private_source","private_derived"])?,serde_json::to_string(&vec!["shared_approved","safe_receipt","security_metadata"])?,policy,state,detail_code,i64::from(approval_required),output_hash,private_evidence_hash,SCAN_VERSION,now])?;
    for redaction in &scan.redactions {
        transaction.execute("INSERT INTO egress_redactions(redaction_id,decision_id,category,json_path_hash,match_hash,created_at_utc) VALUES(?1,?2,?3,?4,?5,?6)",params![Uuid::new_v4().to_string(),decision_id,redaction.category,redaction.json_path_hash,redaction.match_hash,now])?;
    }
    transaction.execute("INSERT INTO private_evidence_records(evidence_id,decision_id,job_id,evidence_hash,source_set_hash,private_result_hash,retention_until_utc,state,created_at_utc) VALUES(?1,?2,?3,?4,?5,?6,?7,'active',?8)",params![Uuid::new_v4().to_string(),decision_id,context.job_id,private_evidence_hash,class_hash,private_result_hash,timestamp(Utc::now()+Duration::days(90)),now])?;
    if let (Some(value), Some(hash)) = (safe_result.as_ref(), output_hash.as_ref()) {
        let projection_id = Uuid::new_v4().to_string();
        let projection_state = if state == "allowed" {
            "active"
        } else {
            "pending_review"
        };
        transaction.execute("INSERT INTO wrapper_resource_projections(projection_id,decision_id,wrapper_id,connection_id,job_id,selector_id,output_schema,safe_result_json,output_hash,source_count,state,expires_at_utc,created_at_utc) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![projection_id,decision_id,context.wrapper_id,context.connection_id,context.job_id,selector_id,output_schema,serde_json::to_string(value)?,hash,i64::from(source_count),projection_state,selector.expires_at_utc,now])?;
        transaction.execute("INSERT INTO projection_cache_entries(cache_id,projection_id,connection_id,selector_id,source_revision_hash,output_hash,state,expires_at_utc,created_at_utc) VALUES(?1,?2,?3,?4,?5,?6,'active',?7,?8)",params![Uuid::new_v4().to_string(),projection_id,context.connection_id,selector_id,class_hash,hash,selector.expires_at_utc,now])?;
        if approval_required {
            transaction.execute("INSERT INTO egress_approvals(approval_id,decision_id,output_hash,state,requested_by_user_id,reason,expires_at_utc,created_at_utc) VALUES(?1,?2,?3,'pending','homeserver-system','Fresh approval required by selector policy',?4,?5)",params![Uuid::new_v4().to_string(),decision_id,hash,timestamp(Utc::now()+Duration::minutes(15)),now])?;
        }
    }
    Ok(EgressEvaluation {
        decision_id,
        state: state.to_owned(),
        detail_code,
        safe_result: if state == "allowed" {
            safe_result
        } else {
            None
        },
        output_hash,
        filter_version: FILTER_VERSION,
        approval_required,
    })
}

pub(crate) fn delivery_egress_is_current_tx(
    transaction: &Transaction<'_>,
    job_id: &str,
) -> Result<bool> {
    if !table_exists_tx(transaction, "egress_decisions")? {
        return Ok(true);
    }
    let capability: Option<String> = transaction
        .query_row(
            "SELECT capability_key FROM wrapper_jobs WHERE job_id=?1",
            params![job_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(capability) = capability else {
        return Ok(false);
    };
    if !KNOWLEDGE_CAPABILITIES.contains(&capability.as_str()) {
        return Ok(true);
    }
    let decision: Option<(String, Option<String>)> = transaction
        .query_row(
            "SELECT state,selector_id FROM egress_decisions WHERE job_id=?1",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((state, selector_id)) = decision else {
        return Ok(false);
    };
    if state == "denied" {
        return Ok(true);
    }
    if state != "allowed" {
        return Ok(false);
    }
    let Some(selector_id) = selector_id else {
        return Ok(false);
    };
    let selector: Option<(String, String)> = transaction
        .query_row(
            "SELECT state,expires_at_utc FROM private_resource_selectors WHERE selector_id=?1",
            params![selector_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(selector.is_some_and(|(state, expires)| {
        state == "active"
            && parse_utc(&expires, "selector expiration").is_ok_and(|v| v > Utc::now())
    }))
}

pub(crate) fn safe_result_is_visible(connection: &Connection, job_id: &str) -> Result<bool> {
    if !table_exists(connection, "egress_decisions")? {
        return Ok(true);
    }
    let capability: String = connection.query_row(
        "SELECT capability_key FROM wrapper_jobs WHERE job_id=?1",
        params![job_id],
        |row| row.get(0),
    )?;
    if !KNOWLEDGE_CAPABILITIES.contains(&capability.as_str()) {
        return Ok(true);
    }
    let state: Option<String> = connection
        .query_row(
            "SELECT state FROM egress_decisions WHERE job_id=?1",
            params![job_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(state.as_deref() == Some("allowed"))
}

fn snapshot(state: &AppState, connection_id: Option<&str>) -> Result<PrivacySnapshot> {
    let c = state.connection()?;
    Ok(PrivacySnapshot {
        schema: "homeserver.private-knowledge-boundary.v1".to_owned(),
        data_classes: read_data_classes(&c)?,
        resources: read_resources(&c)?,
        selectors: read_selectors(&c, connection_id)?,
        decisions: read_decisions(&c, connection_id)?,
        incidents: read_incidents(&c, connection_id)?,
        private_sources_exposed: false,
        local_paths_exposed: false,
        destination_specific_aliases: true,
        fail_closed: true,
        pairing_implies_private_authority: false,
    })
}
fn read_data_classes(c: &Connection) -> Result<Vec<DataClassSummary>> {
    let mut s=c.prepare("SELECT class_key,description,sensitivity_tier,wrapper_egress_mode,default_retention_days,state FROM data_classification_catalog ORDER BY class_key")?;
    s.query_map([], |r| {
        let d: i64 = r.get(4)?;
        Ok(DataClassSummary {
            class_key: r.get(0)?,
            description: r.get(1)?,
            sensitivity_tier: r.get(2)?,
            wrapper_egress_mode: r.get(3)?,
            default_retention_days: d.max(0) as u32,
            state: r.get(5)?,
        })
    })?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(Into::into)
}
fn read_resources(c: &Connection) -> Result<Vec<PrivateResourceSummary>> {
    let mut s=c.prepare("SELECT r.resource_id,r.resource_namespace,r.resource_type,r.local_source_id,r.local_display_name,r.source_hash,r.state,r.resource_revision,c.class_key,c.classification_revision,r.updated_at_utc FROM private_resource_catalog r JOIN private_resource_classifications c ON c.resource_id=r.resource_id AND c.state='active' ORDER BY r.updated_at_utc DESC,r.resource_id LIMIT 500")?;
    s.query_map([], |r| {
        let rr: i64 = r.get(7)?;
        let cr: i64 = r.get(9)?;
        Ok(PrivateResourceSummary {
            resource_id: r.get(0)?,
            resource_namespace: r.get(1)?,
            resource_type: r.get(2)?,
            local_source_id: r.get(3)?,
            local_display_name: r.get(4)?,
            source_hash: r.get(5)?,
            state: r.get(6)?,
            resource_revision: rr.max(0) as u64,
            class_key: r.get(8)?,
            classification_revision: cr.max(0) as u64,
            updated_at_utc: r.get(10)?,
        })
    })?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(Into::into)
}
fn read_selectors(c: &Connection, connection_id: Option<&str>) -> Result<Vec<SelectorSummary>> {
    let sql = if connection_id.is_some() {
        "SELECT selector_id,wrapper_id,connection_id,grant_id,grant_revision,selector_revision,agent_id,agent_revision,resource_namespace,resource_type,allowed_operations_json,maximum_items,maximum_source_bytes,purpose,purpose_hash,output_schema,allow_citations,remote_model_mode,approved_remote_provider,egress_approval_mode,state,expires_at_utc FROM private_resource_selectors WHERE connection_id=?1 ORDER BY updated_at_utc DESC LIMIT 500"
    } else {
        "SELECT selector_id,wrapper_id,connection_id,grant_id,grant_revision,selector_revision,agent_id,agent_revision,resource_namespace,resource_type,allowed_operations_json,maximum_items,maximum_source_bytes,purpose,purpose_hash,output_schema,allow_citations,remote_model_mode,approved_remote_provider,egress_approval_mode,state,expires_at_utc FROM private_resource_selectors ORDER BY updated_at_utc DESC LIMIT 500"
    };
    let mut s = c.prepare(sql)?;
    let mut items = if let Some(v) = connection_id {
        s.query_map(params![v], selector_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        s.query_map([], selector_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for item in &mut items {
        item.resource_ids = selector_resource_ids(c, &item.selector_id)?;
    }
    Ok(items)
}
fn selector_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SelectorSummary> {
    let gr: i64 = r.get(4)?;
    let sr: i64 = r.get(5)?;
    let ar: Option<i64> = r.get(7)?;
    let mi: i64 = r.get(11)?;
    let mb: i64 = r.get(12)?;
    let ac: i64 = r.get(16)?;
    let ops: String = r.get(10)?;
    Ok(SelectorSummary {
        selector_id: r.get(0)?,
        wrapper_id: r.get(1)?,
        connection_id: r.get(2)?,
        grant_id: r.get(3)?,
        grant_revision: gr.max(0) as u64,
        selector_revision: sr.max(0) as u64,
        agent_id: r.get(6)?,
        agent_revision: ar.map(|v| v.max(0) as u64),
        resource_namespace: r.get(8)?,
        resource_type: r.get(9)?,
        allowed_operations: serde_json::from_str(&ops).unwrap_or_default(),
        maximum_items: mi.max(0) as u32,
        maximum_source_bytes: mb.max(0) as u64,
        purpose: r.get(13)?,
        purpose_hash: r.get(14)?,
        output_schema: r.get(15)?,
        allow_citations: ac != 0,
        remote_model_mode: r.get(17)?,
        approved_remote_provider: r.get(18)?,
        egress_approval_mode: r.get(19)?,
        state: r.get(20)?,
        expires_at_utc: r.get(21)?,
        resource_ids: Vec::new(),
    })
}
fn read_decisions(
    c: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<EgressDecisionSummary>> {
    let sql = if connection_id.is_some() {
        "SELECT decision_id,wrapper_id,connection_id,job_id,selector_id,grant_id,grant_revision,output_schema,input_classes_json,output_classes_json,policy,state,detail_code,approval_required,output_hash,private_evidence_hash,scan_version,created_at_utc,decided_at_utc,delivered_at_utc FROM egress_decisions WHERE connection_id=?1 ORDER BY created_at_utc DESC LIMIT 500"
    } else {
        "SELECT decision_id,wrapper_id,connection_id,job_id,selector_id,grant_id,grant_revision,output_schema,input_classes_json,output_classes_json,policy,state,detail_code,approval_required,output_hash,private_evidence_hash,scan_version,created_at_utc,decided_at_utc,delivered_at_utc FROM egress_decisions ORDER BY created_at_utc DESC LIMIT 500"
    };
    let mut s = c.prepare(sql)?;
    let f = |r: &rusqlite::Row<'_>| {
        let rev: i64 = r.get(6)?;
        let a: i64 = r.get(13)?;
        let i: String = r.get(8)?;
        let o: String = r.get(9)?;
        Ok(EgressDecisionSummary {
            decision_id: r.get(0)?,
            wrapper_id: r.get(1)?,
            connection_id: r.get(2)?,
            job_id: r.get(3)?,
            selector_id: r.get(4)?,
            grant_id: r.get(5)?,
            grant_revision: rev.max(0) as u64,
            output_schema: r.get(7)?,
            input_classes: serde_json::from_str(&i).unwrap_or_default(),
            output_classes: serde_json::from_str(&o).unwrap_or_default(),
            policy: r.get(10)?,
            state: r.get(11)?,
            detail_code: r.get(12)?,
            approval_required: a != 0,
            output_hash: r.get(14)?,
            private_evidence_hash: r.get(15)?,
            scan_version: r.get(16)?,
            created_at_utc: r.get(17)?,
            decided_at_utc: r.get(18)?,
            delivered_at_utc: r.get(19)?,
        })
    };
    if let Some(v) = connection_id {
        s.query_map(params![v], f)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    } else {
        s.query_map([], f)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}
fn read_incidents(
    c: &Connection,
    connection_id: Option<&str>,
) -> Result<Vec<PrivacyIncidentSummary>> {
    let sql = if connection_id.is_some() {
        "SELECT incident_id,wrapper_id,connection_id,job_id,selector_id,severity,category,detail_code,evidence_hash,state,detected_at_utc FROM privacy_boundary_incidents WHERE connection_id=?1 ORDER BY detected_at_utc DESC LIMIT 500"
    } else {
        "SELECT incident_id,wrapper_id,connection_id,job_id,selector_id,severity,category,detail_code,evidence_hash,state,detected_at_utc FROM privacy_boundary_incidents ORDER BY detected_at_utc DESC LIMIT 500"
    };
    let mut s = c.prepare(sql)?;
    let f = |r: &rusqlite::Row<'_>| {
        Ok(PrivacyIncidentSummary {
            incident_id: r.get(0)?,
            wrapper_id: r.get(1)?,
            connection_id: r.get(2)?,
            job_id: r.get(3)?,
            selector_id: r.get(4)?,
            severity: r.get(5)?,
            category: r.get(6)?,
            detail_code: r.get(7)?,
            evidence_hash: r.get(8)?,
            state: r.get(9)?,
            detected_at_utc: r.get(10)?,
        })
    };
    if let Some(v) = connection_id {
        s.query_map(params![v], f)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    } else {
        s.query_map([], f)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }
}

fn classify_resource(c: &Connection, r: ClassifyResourceRequest) -> Result<PrivateResourceSummary> {
    let id = bounded_text(&r.resource_id, 1, 240, "resource ID")?;
    let class = validate_enum(&r.class_key, DATA_CLASSES, "data class")?;
    let actor = bounded_text(&r.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&r.reason, 1, 500, "reason")?;
    let tx = c.unchecked_transaction()?;
    let state: String = tx.query_row(
        "SELECT state FROM private_resource_catalog WHERE resource_id=?1",
        params![id],
        |row| row.get(0),
    )?;
    ensure!(
        state != "deleted",
        "deleted resource cannot be reclassified"
    );
    let next:i64=tx.query_row("SELECT COALESCE(MAX(classification_revision),0)+1 FROM private_resource_classifications WHERE resource_id=?1",params![id],|row|row.get(0))?;
    tx.execute("UPDATE private_resource_classifications SET state='superseded',revoked_at_utc=?1 WHERE resource_id=?2 AND state='active'",params![now_utc(),id])?;
    tx.execute("INSERT INTO private_resource_classifications(classification_id,resource_id,class_key,classification_revision,state,classified_by_user_id,reason,created_at_utc) VALUES(?1,?2,?3,?4,'active',?5,?6,?7)",params![Uuid::new_v4().to_string(),id,class,next,actor,reason,now_utc()])?;
    invalidate_resource_tx(&tx, &id, "classification_changed")?;
    tx.commit()?;
    read_resources(c)?
        .into_iter()
        .find(|x| x.resource_id == id)
        .context("classified resource was not found")
}

fn create_selector(c: &Connection, r: CreateSelectorRequest) -> Result<SelectorSummary> {
    ensure!(
        (1..=525_600).contains(&r.expires_minutes),
        "selector expiration must be between one minute and one year"
    );
    ensure!(
        !r.resource_ids.is_empty() && r.resource_ids.len() <= MAX_SELECTOR_RESOURCES,
        "selector requires 1 to 500 resources"
    );
    let connection_id = validate_uuid(&r.connection_id, "connection ID")?;
    let grant_id = validate_uuid(&r.grant_id, "grant ID")?;
    let ns = validate_symbol(&r.resource_namespace, 80, "resource namespace")?;
    let typ = validate_symbol(&r.resource_type, 80, "resource type")?;
    let ops = validate_symbol_list(r.allowed_operations, 16, 80, "operation")?;
    ensure!(!ops.is_empty(), "selector requires an operation");
    let purpose = bounded_text(&r.purpose, 1, 1000, "purpose")?;
    let purpose_hash = hash_text(&purpose);
    let schema = validate_symbol(&r.output_schema, 160, "output schema")?;
    let actor = bounded_text(&r.created_by_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&r.reason, 1, 500, "reason")?;
    let max_items = r
        .maximum_items
        .unwrap_or(r.resource_ids.len() as u32)
        .clamp(1, 500);
    let max_bytes = r
        .maximum_source_bytes
        .unwrap_or(10 * 1024 * 1024)
        .clamp(1024, 100 * 1024 * 1024);
    let remote = validate_enum(
        r.remote_model_mode.as_deref().unwrap_or("disabled"),
        &["disabled", "local_only", "approved_provider"],
        "remote model mode",
    )?;
    let provider = r
        .approved_remote_provider
        .as_deref()
        .map(|v| validate_symbol(v, 120, "remote provider"))
        .transpose()?;
    ensure!(
        (remote == "approved_provider") == provider.is_some(),
        "approved provider is required only for approved-provider mode"
    );
    let approval = validate_enum(
        r.egress_approval_mode.as_deref().unwrap_or("preauthorized"),
        &["preauthorized", "per_result"],
        "egress approval mode",
    )?;
    let(wrapper_id,grant_rev,cap,state,grant_exp):(String,i64,String,String,String)=c.query_row("SELECT g.wrapper_id,g.grant_revision,g.capability_key,g.state,g.expires_at_utc FROM wrapper_capability_grants g JOIN wrapper_connections c ON c.connection_id=g.connection_id AND c.wrapper_id=g.wrapper_id WHERE g.grant_id=?1 AND g.connection_id=?2",params![grant_id,connection_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
    ensure!(state == "active", "selector grant is not active");
    ensure!(
        KNOWLEDGE_CAPABILITIES.contains(&cap.as_str()) || cap == "model.inference.request",
        "selector grant is not knowledge-scoped"
    );
    let now = Utc::now();
    let expiration = (now + Duration::minutes(i64::from(r.expires_minutes)))
        .min(parse_utc(&grant_exp, "grant expiration")?);
    ensure!(expiration > now, "selector expiration is invalid");
    let agent = if let Some(id) = r.agent_id.as_deref() {
        let id = validate_uuid(id, "agent ID")?;
        let rev:i64=c.query_row("SELECT a.revision FROM homeserver_agents a JOIN wrapper_agent_assignments x ON x.agent_id=a.agent_id WHERE a.agent_id=?1 AND a.state='active' AND x.connection_id=?2 AND x.state='active' AND x.expires_at_utc>strftime('%Y-%m-%dT%H:%M:%fZ','now')",params![id,connection_id],|row|row.get(0))?;
        Some((id, rev.max(0) as u64))
    } else {
        None
    };
    let mut resources = Vec::new();
    let mut seen = BTreeSet::new();
    for id in r.resource_ids {
        let id = bounded_text(&id, 1, 240, "resource ID")?;
        ensure!(seen.insert(id.clone()), "duplicate selector resource");
        let row:(String,String,String,i64,i64)=c.query_row("SELECT r.resource_namespace,r.resource_type,r.state,r.resource_revision,c.classification_revision FROM private_resource_catalog r JOIN private_resource_classifications c ON c.resource_id=r.resource_id AND c.state='active' WHERE r.resource_id=?1",params![id],|x|Ok((x.get(0)?,x.get(1)?,x.get(2)?,x.get(3)?,x.get(4)?)))?;
        ensure!(
            row.0 == ns && row.1 == typ && row.2 == "active",
            "selector resource authority is invalid"
        );
        resources.push((id, row.3.max(1), row.4.max(1)));
    }
    let selector_id = Uuid::new_v4().to_string();
    let now_text = timestamp(now);
    let tx = c.unchecked_transaction()?;
    tx.execute("INSERT INTO private_resource_selectors(selector_id,wrapper_id,connection_id,grant_id,grant_revision,selector_revision,agent_id,agent_revision,resource_namespace,resource_type,allowed_operations_json,maximum_items,maximum_source_bytes,purpose,purpose_hash,output_schema,allow_citations,remote_model_mode,approved_remote_provider,egress_approval_mode,state,created_by_user_id,reason,expires_at_utc,created_at_utc,updated_at_utc) VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,'active',?20,?21,?22,?23,?23)",params![selector_id,wrapper_id,connection_id,grant_id,grant_rev,agent.as_ref().map(|v|v.0.clone()),agent.as_ref().map(|v|v.1 as i64),ns,typ,serde_json::to_string(&ops)?,i64::from(max_items),max_bytes as i64,purpose,purpose_hash,schema,i64::from(r.allow_citations.unwrap_or(false)),remote,provider,approval,actor,reason,timestamp(expiration),now_text])?;
    for (id, rr, cr) in resources {
        tx.execute("INSERT INTO private_selector_resources(selector_id,resource_id,captured_resource_revision,captured_classification_revision,created_at_utc) VALUES(?1,?2,?3,?4,?5)",params![selector_id,id,rr,cr,now_text])?;
    }
    tx.commit()?;
    read_selectors(c, Some(&connection_id))?
        .into_iter()
        .find(|x| x.selector_id == selector_id)
        .context("created selector was not found")
}

fn revoke_selector(c: &Connection, r: RevokeSelectorRequest) -> Result<SelectorSummary> {
    let id = validate_uuid(&r.selector_id, "selector ID")?;
    ensure!(
        r.confirmation == format!("REVOKE SELECTOR {id}"),
        "selector revocation confirmation is invalid"
    );
    let _ = bounded_text(&r.actor_user_id, 1, 160, "actor user ID")?;
    let _ = bounded_text(&r.reason, 1, 500, "reason")?;
    let tx = c.unchecked_transaction()?;
    let connection_id:String=tx.query_row("SELECT connection_id FROM private_resource_selectors WHERE selector_id=?1 AND state IN ('active','suspended')",params![id],|row|row.get(0))?;
    let now = now_utc();
    tx.execute("UPDATE private_resource_selectors SET state='revoked',selector_revision=selector_revision+1,revoked_at_utc=?1,updated_at_utc=?1 WHERE selector_id=?2",params![now,id])?;
    tx.execute("UPDATE wrapper_resource_projections SET state='revoked',revoked_at_utc=?1 WHERE selector_id=?2 AND state IN ('active','pending_review')",params![now,id])?;
    tx.execute("UPDATE projection_cache_entries SET state='invalidated',invalidated_at_utc=?1 WHERE selector_id=?2 AND state='active'",params![now,id])?;
    tx.execute("UPDATE egress_decisions SET state='revoked',revoked_at_utc=?1 WHERE selector_id=?2 AND state IN ('allowed','pending_review')",params![now,id])?;
    tx.execute("UPDATE wrapper_job_deliveries SET state='expired',updated_at_utc=?1 WHERE job_id IN(SELECT job_id FROM wrapper_job_privacy_bindings WHERE selector_id=?2) AND state IN('pending','in_flight')",params![now,id])?;
    tx.commit()?;
    wrapper_jobs::reconcile_authority(c)?;
    read_selectors(c, Some(&connection_id))?
        .into_iter()
        .find(|x| x.selector_id == id)
        .context("revoked selector was not found")
}

fn review_egress(c: &Connection, r: ReviewEgressRequest) -> Result<EgressDecisionSummary> {
    let id = validate_uuid(&r.decision_id, "decision ID")?;
    let hash = validate_sha256(&r.output_hash, "output hash")?;
    let actor = bounded_text(&r.actor_user_id, 1, 160, "actor user ID")?;
    let reason = bounded_text(&r.reason, 1, 500, "reason")?;
    let choice = validate_enum(&r.decision, &["approve", "reject"], "decision")?;
    let expected = if choice == "approve" {
        format!("APPROVE EGRESS {id}")
    } else {
        format!("REJECT EGRESS {id}")
    };
    ensure!(
        r.confirmation == expected,
        "egress review confirmation is invalid"
    );
    let tx = c.unchecked_transaction()?;
    let (stored, state, job): (Option<String>, String, String) = tx.query_row(
        "SELECT output_hash,state,job_id FROM egress_decisions WHERE decision_id=?1",
        params![id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    ensure!(
        state == "pending_review" && stored.as_deref() == Some(&hash),
        "egress decision or hash is invalid"
    );
    let now = now_utc();
    if choice == "approve" {
        tx.execute("UPDATE egress_decisions SET state='allowed',policy='allow',detail_code='user_approved',decided_at_utc=?1 WHERE decision_id=?2",params![now,id])?;
        tx.execute(
            "UPDATE wrapper_resource_projections SET state='active' WHERE decision_id=?1",
            params![id],
        )?;
        tx.execute("UPDATE egress_approvals SET state='consumed',decided_by_user_id=?1,decided_at_utc=?2,consumed_at_utc=?2,reason=?3 WHERE decision_id=?4",params![actor,now,reason,id])?;
        let p:Option<(String,String,i64)>=tx.query_row("SELECT safe_result_json,output_hash,source_count FROM wrapper_resource_projections WHERE decision_id=?1",params![id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        if let Some((safe, output, count)) = p {
            let provenance = json!({"source_count":count.max(0),"source_types":["private_knowledge"],"source_identifiers_included":false,"private_source_content_included":false,"approved_by_user":true});
            let pt = serde_json::to_string(&provenance)?;
            tx.execute("INSERT OR REPLACE INTO wrapper_job_safe_results(job_id,result_policy,safe_result_json,safe_result_hash,provenance_summary_json,provenance_summary_hash,filter_version,result_bytes,created_at_utc) SELECT j.job_id,j.result_policy,?1,?2,?3,?4,?5,length(?1),?6 FROM wrapper_jobs j WHERE j.job_id=?7",params![safe,output,pt,hash_text(&pt),FILTER_VERSION,now,job])?;
        }
    } else {
        tx.execute("UPDATE egress_decisions SET state='denied',policy='deny',detail_code='user_rejected',decided_at_utc=?1 WHERE decision_id=?2",params![now,id])?;
        tx.execute("UPDATE wrapper_resource_projections SET state='revoked',revoked_at_utc=?1 WHERE decision_id=?2",params![now,id])?;
        tx.execute("UPDATE egress_approvals SET state='rejected',decided_by_user_id=?1,decided_at_utc=?2,reason=?3 WHERE decision_id=?4",params![actor,now,reason,id])?;
        tx.execute("UPDATE wrapper_job_deliveries SET state='expired',updated_at_utc=?1 WHERE job_id=?2 AND state IN('pending','in_flight')",params![now,job])?;
    }
    tx.commit()?;
    read_decisions(c, None)?
        .into_iter()
        .find(|x| x.decision_id == id)
        .context("reviewed decision was not found")
}

fn purge_cache(c: &Connection, r: PurgeCacheRequest) -> Result<Value> {
    ensure!(
        r.confirmation == "PURGE PRIVACY CACHE",
        "cache purge confirmation is invalid"
    );
    let connection = r
        .connection_id
        .as_deref()
        .map(|v| validate_uuid(v, "connection ID"))
        .transpose()?;
    let selector = r
        .selector_id
        .as_deref()
        .map(|v| validate_uuid(v, "selector ID"))
        .transpose()?;
    let n=match(connection.as_deref(),selector.as_deref()){(Some(cn),Some(s))=>c.execute("UPDATE projection_cache_entries SET state='invalidated',invalidated_at_utc=?1 WHERE connection_id=?2 AND selector_id=?3 AND state='active'",params![now_utc(),cn,s])?,(Some(cn),None)=>c.execute("UPDATE projection_cache_entries SET state='invalidated',invalidated_at_utc=?1 WHERE connection_id=?2 AND state='active'",params![now_utc(),cn])?,(None,Some(s))=>c.execute("UPDATE projection_cache_entries SET state='invalidated',invalidated_at_utc=?1 WHERE selector_id=?2 AND state='active'",params![now_utc(),s])?,(None,None)=>c.execute("UPDATE projection_cache_entries SET state='invalidated',invalidated_at_utc=?1 WHERE state='active'",params![now_utc()])?};
    Ok(json!({"affected":n,"private_content_returned":false}))
}

fn validate_private_search_authority(
    c: &Connection,
    r: &PrivateSearchRequest,
) -> Result<SelectorAuthority> {
    let worker = validate_uuid(&r.worker_id, "worker ID")?;
    let job = validate_uuid(&r.job_id, "job ID")?;
    let lease = hash_text(&bounded_text(&r.lease_token, 32, 128, "lease token")?);
    let(selector,stored_worker,stored_hash,state,expires):(String,String,String,String,String)=c.query_row("SELECT b.selector_id,j.lease_owner_id,j.lease_token_hash,j.state,j.lease_expires_at_utc FROM wrapper_job_privacy_bindings b JOIN wrapper_jobs j ON j.job_id=b.job_id WHERE b.job_id=?1",params![job],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?;
    ensure!(
        worker == stored_worker
            && lease == stored_hash
            && state == "running"
            && parse_utc(&expires, "lease expiration")? > Utc::now(),
        "private search lease is invalid"
    );
    ensure!(
        job_privacy_authority_is_current(c, &job)?,
        "private search authority changed"
    );
    let selector = selector_authority(c, &selector)?;
    ensure!(
        selector.allowed_operations.iter().any(|v| v == "search"),
        "selector does not permit search"
    );
    Ok(selector)
}
fn finish_private_search(
    c: &Connection,
    r: &PrivateSearchRequest,
    s: &SelectorAuthority,
    search: semantic_vault::SemanticSearchResult,
) -> Result<PrivateSearchResult> {
    let allowed = selector_resource_ids(c, &s.selector_id)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut hits = Vec::new();
    let mut bytes = 0usize;
    for hit in search.hits {
        let id = format!("vault:{}", hit.document_id);
        if !allowed.contains(&id) {
            continue;
        }
        bytes = bytes.saturating_add(hit.snippet.len());
        if bytes > s.maximum_source_bytes as usize || hits.len() >= s.maximum_items as usize {
            break;
        }
        hits.push(PrivateSearchHit {
            resource_id: id,
            title: hit.title,
            snippet: hit.snippet,
            page_number: hit.page_number,
            combined_score: hit.combined_score,
        });
    }
    let qh = hash_text(r.query.trim());
    let rh = hash_json(&hits)?;
    c.execute("INSERT INTO private_knowledge_access_receipts(access_id,job_id,selector_id,connection_id,operation,query_hash,result_hash,source_count,source_bytes,created_at_utc) VALUES(?1,?2,?3,?4,'search',?5,?6,?7,?8,?9)",params![Uuid::new_v4().to_string(),r.job_id,s.selector_id,s.connection_id,qh,rh,hits.len() as i64,bytes as i64,now_utc()])?;
    Ok(PrivateSearchResult {
        job_id: r.job_id.clone(),
        selector_id: s.selector_id.clone(),
        query_hash: qh,
        hits,
        source_count: allowed.len().min(u32::MAX as usize) as u32,
        private_only: true,
    })
}

fn selector_authority(c: &Connection, id: &str) -> Result<SelectorAuthority> {
    c.query_row("SELECT wrapper_id,connection_id,grant_id,grant_revision,selector_revision,agent_id,agent_revision,allowed_operations_json,maximum_items,maximum_source_bytes,purpose_hash,output_schema,allow_citations,remote_model_mode,approved_remote_provider,egress_approval_mode,expires_at_utc FROM private_resource_selectors WHERE selector_id=?1 AND state='active'",params![id],|row|selector_authority_row(id,row)).context("active private selector was not found")
}
fn selector_authority_tx(c: &Transaction<'_>, id: &str) -> Result<SelectorAuthority> {
    c.query_row("SELECT wrapper_id,connection_id,grant_id,grant_revision,selector_revision,agent_id,agent_revision,allowed_operations_json,maximum_items,maximum_source_bytes,purpose_hash,output_schema,allow_citations,remote_model_mode,approved_remote_provider,egress_approval_mode,expires_at_utc FROM private_resource_selectors WHERE selector_id=?1 AND state='active'",params![id],|row|selector_authority_row(id,row)).context("active private selector was not found")
}
fn selector_authority_row(id: &str, r: &rusqlite::Row<'_>) -> rusqlite::Result<SelectorAuthority> {
    let gr: i64 = r.get(3)?;
    let sr: i64 = r.get(4)?;
    let ar: Option<i64> = r.get(6)?;
    let ops: String = r.get(7)?;
    let mi: i64 = r.get(8)?;
    let mb: i64 = r.get(9)?;
    let ac: i64 = r.get(12)?;
    Ok(SelectorAuthority {
        selector_id: id.to_owned(),
        wrapper_id: r.get(0)?,
        connection_id: r.get(1)?,
        grant_id: r.get(2)?,
        grant_revision: gr.max(0) as u64,
        selector_revision: sr.max(0) as u64,
        agent_id: r.get(5)?,
        agent_revision: ar.map(|v| v.max(0) as u64),
        allowed_operations: serde_json::from_str(&ops).unwrap_or_default(),
        maximum_items: mi.max(0) as u32,
        maximum_source_bytes: mb.max(0) as u64,
        purpose_hash: r.get(10)?,
        output_schema: r.get(11)?,
        allow_citations: ac != 0,
        remote_model_mode: r.get(13)?,
        approved_remote_provider: r.get(14)?,
        egress_approval_mode: r.get(15)?,
        expires_at_utc: r.get(16)?,
    })
}

fn scan_for_egress(
    tx: &Transaction<'_>,
    selector: &SelectorAuthority,
    value: &Value,
    path: &str,
    depth: usize,
) -> Result<ScanOutcome> {
    ensure!(depth <= 8, "egress result exceeds maximum nesting depth");
    let mut redactions = Vec::new();
    let mut denied = None;
    let value = match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => {
            ensure!(text.chars().count() <= 8000, "egress string is too long");
            let lower = text.to_ascii_lowercase();
            if contains_credential(&lower) {
                denied = Some("credential_material".to_owned());
                Value::String("[blocked]".to_owned())
            } else if contains_cross_wrapper_sentinel(&lower) {
                denied = Some("cross_wrapper_sentinel".to_owned());
                Value::String("[blocked]".to_owned())
            } else if looks_like_local_path(text) {
                redactions.push(redaction("local_path", path, text));
                Value::String("[local reference removed]".to_owned())
            } else {
                Value::String(text.clone())
            }
        }
        Value::Array(items) => {
            ensure!(items.len() <= 500, "egress array is too large");
            let mut out = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let child =
                    scan_for_egress(tx, selector, item, &format!("{path}[{index}]"), depth + 1)?;
                redactions.extend(child.redactions);
                if denied.is_none() {
                    denied = child.denied_category
                }
                out.push(child.value)
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            ensure!(map.len() <= 200, "egress object is too large");
            let mut out = Map::new();
            for (key, item) in map {
                let norm = key.to_ascii_lowercase().replace('-', '_');
                let child_path = format!("{path}.{key}");
                if is_private_key(&norm) {
                    if matches!(
                        norm.as_str(),
                        "resource_id" | "document_id" | "source_id" | "citation_id"
                    ) {
                        if let Some(local) = item.as_str() {
                            if let Some(alias) = alias_for_local_reference_tx(tx, selector, local)?
                            {
                                out.insert("reference".to_owned(), Value::String(alias));
                                redactions.push(redaction("local_identifier", &child_path, local));
                                continue;
                            }
                        }
                    }
                    redactions.push(redaction("private_field", &child_path, &norm));
                    continue;
                }
                if matches!(
                    norm.as_str(),
                    "file_name" | "filename" | "local_path" | "file_path"
                ) {
                    redactions.push(redaction(
                        "local_path",
                        &child_path,
                        item.as_str().unwrap_or("non_string"),
                    ));
                    continue;
                }
                let child = scan_for_egress(tx, selector, item, &child_path, depth + 1)?;
                redactions.extend(child.redactions);
                if denied.is_none() {
                    denied = child.denied_category
                }
                out.insert(key.clone(), child.value)
            }
            Value::Object(out)
        }
    };
    Ok(ScanOutcome {
        value,
        redactions,
        denied_category: denied,
    })
}
fn alias_for_local_reference_tx(
    tx: &Transaction<'_>,
    s: &SelectorAuthority,
    local: &str,
) -> Result<Option<String>> {
    if !s.allow_citations {
        return Ok(None);
    }
    for candidate in [local.to_owned(), format!("vault:{local}")] {
        let allowed:i64=tx.query_row("SELECT COUNT(*) FROM private_selector_resources WHERE selector_id=?1 AND resource_id=?2",params![s.selector_id,candidate],|r|r.get(0))?;
        if allowed == 0 {
            continue;
        }
        let existing:Option<String>=tx.query_row("SELECT alias_reference FROM private_resource_aliases WHERE connection_id=?1 AND resource_id=?2 AND state='active'",params![s.connection_id,candidate],|r|r.get(0)).optional()?;
        if let Some(alias) = existing {
            return Ok(Some(alias));
        }
        let alias = format!("hsref_{}", Uuid::new_v4().simple());
        tx.execute("INSERT INTO private_resource_aliases(alias_id,connection_id,resource_id,alias_reference,state,created_at_utc) VALUES(?1,?2,?3,?4,'active',?5)",params![Uuid::new_v4().to_string(),s.connection_id,candidate,alias,now_utc()])?;
        return Ok(Some(alias));
    }
    Ok(None)
}

fn expire_and_reconcile(c: &Connection) -> Result<()> {
    if !table_exists(c, "private_resource_selectors")? {
        return Ok(());
    }
    let now = now_utc();
    c.execute("UPDATE private_resource_selectors SET state='expired',selector_revision=selector_revision+1,updated_at_utc=?1 WHERE state IN('active','suspended') AND expires_at_utc<=?1",params![now])?;
    c.execute(
        "UPDATE egress_approvals SET state='expired' WHERE state='pending' AND expires_at_utc<=?1",
        params![now],
    )?;
    c.execute("UPDATE projection_cache_entries SET state='expired',invalidated_at_utc=?1 WHERE state='active' AND expires_at_utc<=?1",params![now])?;
    c.execute("UPDATE wrapper_resource_projections SET state='expired',revoked_at_utc=?1 WHERE state IN('active','pending_review') AND expires_at_utc<=?1",params![now])?;
    c.execute("UPDATE egress_decisions SET state='revoked',revoked_at_utc=?1 WHERE state IN('allowed','pending_review') AND selector_id IN(SELECT selector_id FROM private_resource_selectors WHERE state IN('expired','revoked'))",params![now])?;
    c.execute("UPDATE wrapper_job_deliveries SET state='expired',updated_at_utc=?1 WHERE state IN('pending','in_flight') AND job_id IN(SELECT b.job_id FROM wrapper_job_privacy_bindings b JOIN private_resource_selectors s ON s.selector_id=b.selector_id WHERE s.state IN('expired','revoked'))",params![now])?;
    Ok(())
}
fn process_deletion_queue(c: &Connection) -> Result<()> {
    let mut s=c.prepare("SELECT deletion_job_id,resource_id FROM deletion_propagation_jobs WHERE state IN('queued','running') ORDER BY created_at_utc LIMIT 100")?;
    let jobs = s
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(s);
    for (job, resource) in jobs {
        let tx = c.unchecked_transaction()?;
        tx.execute("UPDATE deletion_propagation_jobs SET state='running',attempt_count=attempt_count+1,updated_at_utc=?1 WHERE deletion_job_id=?2",params![now_utc(),job])?;
        invalidate_resource_tx(&tx, &resource, "source_deleted")?;
        tx.execute(
            "DELETE FROM private_selector_resources WHERE resource_id=?1",
            params![resource],
        )?;
        tx.execute("UPDATE private_resource_aliases SET state='revoked',revoked_at_utc=?1 WHERE resource_id=?2 AND state='active'",params![now_utc(),resource])?;
        tx.execute("UPDATE deletion_propagation_jobs SET state='completed',pending_targets_json='[]',completed_at_utc=?1,updated_at_utc=?1 WHERE deletion_job_id=?2",params![now_utc(),job])?;
        tx.commit()?
    }
    Ok(())
}
fn invalidate_resource_tx(tx: &Transaction<'_>, id: &str, detail: &str) -> Result<()> {
    let now = now_utc();
    tx.execute("UPDATE private_resource_selectors SET state='suspended',selector_revision=selector_revision+1,updated_at_utc=?1 WHERE selector_id IN(SELECT selector_id FROM private_selector_resources WHERE resource_id=?2) AND state='active'",params![now,id])?;
    tx.execute("UPDATE wrapper_resource_projections SET state='revoked',revoked_at_utc=?1 WHERE selector_id IN(SELECT selector_id FROM private_selector_resources WHERE resource_id=?2) AND state IN('active','pending_review')",params![now,id])?;
    tx.execute("UPDATE projection_cache_entries SET state='invalidated',invalidated_at_utc=?1 WHERE selector_id IN(SELECT selector_id FROM private_selector_resources WHERE resource_id=?2) AND state='active'",params![now,id])?;
    let evidence = hash_text(&format!("{id}:{detail}:{now}"));
    record_incident_tx(
        tx,
        None,
        None,
        None,
        None,
        "high",
        "resource_authority_changed",
        detail,
        &evidence,
    )
}
fn record_incident_tx(
    tx: &Transaction<'_>,
    wrapper: Option<&str>,
    connection: Option<&str>,
    job: Option<&str>,
    selector: Option<&str>,
    severity: &str,
    category: &str,
    detail: &str,
    evidence: &str,
) -> Result<()> {
    tx.execute("INSERT INTO privacy_boundary_incidents(incident_id,wrapper_id,connection_id,job_id,selector_id,severity,category,detail_code,evidence_hash,state,detected_at_utc) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'open',?10)",params![Uuid::new_v4().to_string(),wrapper,connection,job,selector,severity,category,detail,evidence,now_utc()])?;
    Ok(())
}

fn classification_set_hash(c: &Connection, selector: &str) -> Result<String> {
    let tx = c.unchecked_transaction()?;
    classification_set_hash_tx(&tx, selector)
}
fn classification_set_hash_tx(tx: &Transaction<'_>, selector: &str) -> Result<String> {
    let mut s=tx.prepare("SELECT r.resource_id,r.resource_revision,c.class_key,c.classification_revision FROM private_selector_resources x JOIN private_resource_catalog r ON r.resource_id=x.resource_id JOIN private_resource_classifications c ON c.resource_id=r.resource_id AND c.state='active' WHERE x.selector_id=?1 ORDER BY r.resource_id")?;
    let rows = s
        .query_map(params![selector], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    hash_json(&rows)
}
fn selector_resource_ids(c: &Connection, selector: &str) -> Result<Vec<String>> {
    let mut s=c.prepare("SELECT resource_id FROM private_selector_resources WHERE selector_id=?1 ORDER BY resource_id")?;
    s.query_map(params![selector], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}
fn validate_remote_model(s: &SelectorAuthority, provider: Option<&str>) -> Result<()> {
    match s.remote_model_mode.as_str() {
        "disabled" | "local_only" => {
            ensure!(provider.is_none(), "selector forbids remote model context")
        }
        "approved_provider" => ensure!(
            provider == s.approved_remote_provider.as_deref(),
            "remote model provider is not approved"
        ),
        _ => bail!("selector remote-model mode is invalid"),
    };
    Ok(())
}
fn job_privacy_authority_is_current(c: &Connection, job: &str) -> Result<bool> {
    let tx = c.unchecked_transaction()?;
    let cap: String = tx.query_row(
        "SELECT capability_key FROM wrapper_jobs WHERE job_id=?1",
        params![job],
        |r| r.get(0),
    )?;
    job_privacy_authority_is_current_tx(&tx, job, &cap)
}
fn table_exists(c: &Connection, name: &str) -> Result<bool> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![name],
        |r| r.get::<_, i64>(0),
    )? > 0)
}
fn table_exists_tx(c: &Transaction<'_>, name: &str) -> Result<bool> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
        params![name],
        |r| r.get::<_, i64>(0),
    )? > 0)
}
fn shape_only(v: &Value, depth: usize) -> Result<Value> {
    ensure!(depth <= 8, "private result shape is too deep");
    Ok(match v {
        Value::Null => Value::String("null".to_owned()),
        Value::Bool(_) => Value::String("bool".to_owned()),
        Value::Number(_) => Value::String("number".to_owned()),
        Value::String(s) => json!({"type":"string","length":s.len()}),
        Value::Array(a) => Value::Array(
            a.iter()
                .take(100)
                .map(|x| shape_only(x, depth + 1))
                .collect::<Result<Vec<_>>>()?,
        ),
        Value::Object(m) => {
            let mut out = Map::new();
            for (k, x) in m.iter().take(100) {
                out.insert(hash_text(k), shape_only(x, depth + 1)?);
            }
            Value::Object(out)
        }
    })
}
fn contains_credential(s: &str) -> bool {
    [
        "-----begin private key",
        "authorization: bearer",
        "api_key=",
        "api-key:",
        "client_secret",
        "refresh_token",
        "bearer eyj",
        "mghs_",
        "sk-",
    ]
    .iter()
    .any(|n| s.contains(n))
}
fn contains_cross_wrapper_sentinel(s: &str) -> bool {
    [
        "cross_wrapper_sentinel",
        "wrapper_secret_sentinel",
        "hs-private-sentinel",
        "do_not_egress",
    ]
    .iter()
    .any(|n| s.contains(n))
}
fn looks_like_local_path(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    s.starts_with("c:\\")
        || s.starts_with("/home/")
        || s.starts_with("/users/")
        || s.starts_with("file://")
        || s.contains("\\appdata\\")
        || s.contains("/mnt/")
}
fn is_private_key(k: &str) -> bool {
    [
        "source",
        "source_id",
        "source_text",
        "raw",
        "raw_text",
        "document",
        "document_id",
        "full_document",
        "full_text",
        "prompt",
        "system_prompt",
        "credential",
        "credentials",
        "secret",
        "token",
        "api_key",
        "memory",
        "private",
        "private_data",
        "conversation",
        "embedding",
        "file_path",
        "local_path",
        "email_body",
        "managed_path",
        "indexed_text",
    ]
    .iter()
    .any(|x| k == *x || k.ends_with(&format!("_{x}")))
}
fn redaction(category: &str, path: &str, value: &str) -> Redaction {
    Redaction {
        category: category.to_owned(),
        json_path_hash: hash_text(path),
        match_hash: hash_text(value),
    }
}
fn validate_enum(value: &str, allowed: &[&str], label: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(allowed.contains(&value.as_str()), "invalid {label}");
    Ok(value)
}
fn validate_symbol(value: &str, max: usize, label: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty() && value.len() <= max, "invalid {label}");
    ensure!(
        value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/')),
        "invalid {label}"
    );
    Ok(value.to_owned())
}
fn validate_symbol_list(
    values: Vec<String>,
    max_items: usize,
    max_chars: usize,
    label: &str,
) -> Result<Vec<String>> {
    ensure!(values.len() <= max_items, "too many {label} values");
    let mut set = BTreeSet::new();
    for value in values {
        set.insert(validate_symbol(&value, max_chars, label)?);
    }
    Ok(set.into_iter().collect())
}
fn validate_uuid(value: &str, label: &str) -> Result<String> {
    Uuid::parse_str(value).with_context(|| format!("invalid {label}"))?;
    Ok(value.to_owned())
}
fn validate_sha256(value: &str, label: &str) -> Result<String> {
    ensure!(
        value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()),
        "invalid {label}"
    );
    Ok(value.to_ascii_lowercase())
}
fn bounded_text(value: &str, min: usize, max: usize, label: &str) -> Result<String> {
    let value = value.trim();
    let count = value.chars().count();
    ensure!((min..=max).contains(&count), "invalid {label}");
    ensure!(!value.chars().any(char::is_control), "invalid {label}");
    Ok(value.to_owned())
}
fn hash_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn hash_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(hash_text(&serde_json::to_string(value)?))
}
fn now_utc() -> String {
    timestamp(Utc::now())
}
fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}
fn parse_utc(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.with_timezone(&Utc))
        .with_context(|| format!("invalid {label}"))
}
fn api_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(?error, code, "private knowledge operation failed");
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}
