use crate::AppState;
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
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../../database/migrations/0021_wrapper_capability_grants.sql");
const MIGRATION_KEY: &str = "0021_wrapper_capability_grants";
const MAX_CONTROL_BODY_BYTES: usize = 96 * 1024;
const MAX_GRANTS: i64 = 2_000;
const MAX_BRIDGES: i64 = 500;
const MAX_EVENTS: i64 = 20_000;
const MAX_RECEIPTS: i64 = 50_000;
const MAX_SCOPES_PER_GRANT: usize = 64;
const MAX_OPERATIONS: usize = 16;
const MAX_ALLOWED_FIELDS: usize = 128;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityCatalogEntry {
    pub capability_key: String,
    pub description: String,
    pub risk_tier: String,
    pub default_approval_mode: String,
    pub result_mode: String,
    pub requires_scope: bool,
    pub allowed_operations: Vec<String>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantScope {
    pub scope_id: String,
    pub grant_id: String,
    pub scope_kind: String,
    pub scope_value: String,
    pub allowed_fields: Vec<String>,
    pub filter: Value,
    pub result_policy: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceLimits {
    pub requests_per_minute: u32,
    pub max_result_bytes: u64,
    pub max_daily_tokens: u64,
    pub max_concurrent_jobs: u32,
    pub max_queued_jobs: u32,
    pub max_execution_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityGrant {
    pub grant_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub capability_key: String,
    pub grant_revision: u64,
    pub allowed_operations: Vec<String>,
    pub approval_mode: String,
    pub state: String,
    pub issued_by_user_id: String,
    pub reason: String,
    pub request_hash: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub approved_by_user_id: Option<String>,
    pub approved_at_utc: Option<String>,
    pub revoked_at_utc: Option<String>,
    pub supersedes_grant_id: Option<String>,
    pub superseded_by_grant_id: Option<String>,
    pub scopes: Vec<GrantScope>,
    pub limits: ResourceLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantApproval {
    pub approval_id: String,
    pub grant_id: Option<String>,
    pub bridge_id: Option<String>,
    pub approval_action: String,
    pub plan_hash: String,
    pub state: String,
    pub requested_by_user_id: String,
    pub decided_by_user_id: Option<String>,
    pub expires_at_utc: String,
    pub created_at_utc: String,
    pub decided_at_utc: Option<String>,
    pub consumed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeGrant {
    pub bridge_id: String,
    pub source_wrapper_id: String,
    pub source_connection_id: String,
    pub target_wrapper_id: String,
    pub target_connection_id: String,
    pub capability_key: String,
    pub allowed_operations: Vec<String>,
    pub scope_kind: String,
    pub scope_value: String,
    pub result_policy: String,
    pub approval_mode: String,
    pub state: String,
    pub issued_by_user_id: String,
    pub reason: String,
    pub request_hash: String,
    pub not_before_utc: String,
    pub expires_at_utc: String,
    pub approved_by_user_id: Option<String>,
    pub approved_at_utc: Option<String>,
    pub revoked_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantRegistrySnapshot {
    pub schema: String,
    pub catalog: Vec<CapabilityCatalogEntry>,
    pub grants: Vec<CapabilityGrant>,
    pub approvals: Vec<GrantApproval>,
    pub bridges: Vec<BridgeGrant>,
    pub active_grants: u64,
    pub pending_approvals: u64,
    pub active_bridges: u64,
    pub pairing_implies_authority: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthorizationDecision {
    pub allowed: bool,
    pub decision_id: String,
    pub wrapper_id: String,
    pub connection_id: String,
    pub grant_id: Option<String>,
    pub bridge_id: Option<String>,
    pub capability_key: String,
    pub operation: String,
    pub grant_revision: u64,
    pub result_policy: Option<String>,
    pub expires_at_utc: Option<String>,
    pub detail_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeInput {
    pub scope_kind: String,
    pub scope_value: String,
    #[serde(default)]
    pub allowed_fields: Vec<String>,
    #[serde(default = "empty_object")]
    pub filter: Value,
    pub result_policy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimitsInput {
    pub requests_per_minute: Option<u32>,
    pub max_result_bytes: Option<u64>,
    pub max_daily_tokens: Option<u64>,
    pub max_concurrent_jobs: Option<u32>,
    pub max_queued_jobs: Option<u32>,
    pub max_execution_seconds: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateGrantRequest {
    pub wrapper_id: String,
    pub connection_id: String,
    pub capability_key: String,
    pub allowed_operations: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<ScopeInput>,
    pub limits: Option<ResourceLimitsInput>,
    pub approval_mode: Option<String>,
    pub issued_by_user_id: String,
    pub reason: String,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RotateGrantRequest {
    pub grant_id: String,
    pub issued_by_user_id: String,
    pub reason: String,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevokeGrantRequest {
    pub grant_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RequestUseApprovalRequest {
    pub grant_id: String,
    pub requested_by_user_id: String,
    pub plan_hash: String,
    pub expires_minutes: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DecideApprovalRequest {
    pub approval_id: String,
    pub actor_user_id: String,
    pub plan_hash: String,
    pub decision: String,
    pub confirmation: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizeRequest {
    pub connection_id: String,
    pub capability_key: String,
    pub operation: String,
    pub scope_kind: Option<String>,
    pub scope_value: Option<String>,
    pub result_bytes: Option<u64>,
    pub token_count: Option<u64>,
    pub approval_id: Option<String>,
    pub plan_hash: Option<String>,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateBridgeRequest {
    pub source_wrapper_id: String,
    pub source_connection_id: String,
    pub target_wrapper_id: String,
    pub target_connection_id: String,
    pub capability_key: String,
    pub allowed_operations: Vec<String>,
    pub scope_kind: String,
    pub scope_value: String,
    pub result_policy: String,
    pub approval_mode: Option<String>,
    pub issued_by_user_id: String,
    pub reason: String,
    pub expires_minutes: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RevokeBridgeRequest {
    pub bridge_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizeBridgeRequest {
    pub source_connection_id: String,
    pub target_connection_id: String,
    pub capability_key: String,
    pub operation: String,
    pub scope_kind: String,
    pub scope_value: String,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ConnectionContext {
    wrapper_id: String,
    connection_id: String,
    grant_revision: u64,
}

#[derive(Debug, Clone)]
struct CapabilityRule {
    risk_tier: String,
    default_approval_mode: String,
    result_mode: String,
    requires_scope: bool,
    allowed_operations: Vec<String>,
}

#[derive(Debug, Clone)]
struct StoredGrant {
    grant_id: String,
    wrapper_id: String,
    connection_id: String,
    capability_key: String,
    grant_revision: u64,
    allowed_operations: Vec<String>,
    approval_mode: String,
    state: String,
    expires_at_utc: String,
    supersedes_grant_id: Option<String>,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    expire_stale_authority(connection)?;
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
        "wrapper capability-grant migration is not registered exactly once"
    );

    for table in [
        "wrapper_capability_catalog",
        "wrapper_capability_grants",
        "wrapper_dataset_scopes",
        "wrapper_resource_limits",
        "wrapper_grant_approvals",
        "wrapper_bridge_grants",
        "wrapper_grant_usage_windows",
        "wrapper_grant_revocation_fences",
        "wrapper_grant_events",
        "wrapper_authorization_receipts",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }

    let catalog_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_capability_catalog WHERE state='active'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        catalog_count >= 10,
        "wrapper capability catalog is incomplete"
    );

    let broad_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_capability_catalog WHERE capability_key IN ('admin','knowledge.all','tools.all','agent.execute_any','cross_wrapper.read') OR capability_key LIKE '%.all'",
        [],
        |row| row.get(0),
    )?;
    ensure!(broad_count == 0, "broad wrapper capabilities are forbidden");

    let orphan_grants: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_capability_grants g LEFT JOIN wrapper_connections c ON c.connection_id=g.connection_id AND c.wrapper_id=g.wrapper_id WHERE c.connection_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        orphan_grants == 0,
        "wrapper grants contain cross-wrapper bindings"
    );

    let invalid_bridges: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_bridge_grants WHERE source_wrapper_id=target_wrapper_id",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        invalid_bridges == 0,
        "same-wrapper bridge grants are invalid"
    );

    let active_expired: i64 = connection.query_row(
        "SELECT COUNT(*) FROM wrapper_capability_grants WHERE state='active' AND expires_at_utc<=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        [],
        |row| row.get(0),
    )?;
    ensure!(active_expired == 0, "expired grants remain active");
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    expire_stale_authority(connection)?;
    connection.execute(
        "DELETE FROM wrapper_grant_usage_windows WHERE (window_kind='minute' AND window_start_utc<strftime('%Y-%m-%dT%H:%M:00.000Z','now','-2 days')) OR (window_kind='day' AND window_start_utc<strftime('%Y-%m-%dT00:00:00.000Z','now','-90 days'))",
        [],
    )?;
    connection.execute(
        "DELETE FROM wrapper_grant_events WHERE created_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM wrapper_grant_events WHERE event_id NOT IN (SELECT event_id FROM wrapper_grant_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1)",
        params![MAX_EVENTS],
    )?;
    connection.execute(
        "DELETE FROM wrapper_authorization_receipts WHERE created_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM wrapper_authorization_receipts WHERE decision_id NOT IN (SELECT decision_id FROM wrapper_authorization_receipts ORDER BY created_at_utc DESC,decision_id DESC LIMIT ?1)",
        params![MAX_RECEIPTS],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/wrapper-grants", get(snapshot_handler))
        .route("/v1/wrapper-grants/create", post(create_grant_handler))
        .route("/v1/wrapper-grants/rotate", post(rotate_grant_handler))
        .route("/v1/wrapper-grants/revoke", post(revoke_grant_handler))
        .route(
            "/v1/wrapper-grants/approvals/request",
            post(request_use_approval_handler),
        )
        .route(
            "/v1/wrapper-grants/approvals/decide",
            post(decide_approval_handler),
        )
        .route("/v1/wrapper-grants/authorize", post(authorize_handler))
        .route("/v1/wrapper-bridges/create", post(create_bridge_handler))
        .route("/v1/wrapper-bridges/revoke", post(revoke_bridge_handler))
        .route(
            "/v1/wrapper-bridges/authorize",
            post(authorize_bridge_handler),
        )
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn snapshot_handler(State(state): State<Arc<AppState>>) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(move || snapshot(&state), "wrapper_grant_snapshot_failed").await
}

async fn create_grant_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateGrantRequest>,
) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            create_grant(&connection, request, None)?;
            snapshot_with_connection(&connection)
        },
        "wrapper_grant_create_failed",
    )
    .await
}

async fn rotate_grant_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RotateGrantRequest>,
) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            rotate_grant(&connection, request)?;
            snapshot_with_connection(&connection)
        },
        "wrapper_grant_rotate_failed",
    )
    .await
}

async fn revoke_grant_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RevokeGrantRequest>,
) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            revoke_grant(&connection, request)?;
            snapshot_with_connection(&connection)
        },
        "wrapper_grant_revoke_failed",
    )
    .await
}

async fn request_use_approval_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RequestUseApprovalRequest>,
) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            request_use_approval(&connection, request)?;
            snapshot_with_connection(&connection)
        },
        "wrapper_grant_use_approval_request_failed",
    )
    .await
}

async fn decide_approval_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DecideApprovalRequest>,
) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            decide_approval(&connection, request)?;
            snapshot_with_connection(&connection)
        },
        "wrapper_grant_approval_decision_failed",
    )
    .await
}

async fn authorize_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AuthorizeRequest>,
) -> ApiResult<AuthorizationDecision> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            authorize(&connection, request)
        },
        "wrapper_grant_authorization_failed",
    )
    .await
}

async fn create_bridge_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateBridgeRequest>,
) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            create_bridge(&connection, request)?;
            snapshot_with_connection(&connection)
        },
        "wrapper_bridge_create_failed",
    )
    .await
}

async fn revoke_bridge_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RevokeBridgeRequest>,
) -> ApiResult<GrantRegistrySnapshot> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            revoke_bridge(&connection, request)?;
            snapshot_with_connection(&connection)
        },
        "wrapper_bridge_revoke_failed",
    )
    .await
}

async fn authorize_bridge_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AuthorizeBridgeRequest>,
) -> ApiResult<AuthorizationDecision> {
    run_blocking(
        move || {
            let connection = state.connection()?;
            authorize_bridge(&connection, request)
        },
        "wrapper_bridge_authorization_failed",
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
        .map_err(|error| api_error(code, anyhow::anyhow!("grant task failed: {error}")))?
        .map(Json)
        .map_err(|error| api_error(code, error))
}

pub fn snapshot(state: &AppState) -> Result<GrantRegistrySnapshot> {
    let connection = state.connection()?;
    snapshot_with_connection(&connection)
}

fn snapshot_with_connection(connection: &Connection) -> Result<GrantRegistrySnapshot> {
    expire_stale_authority(connection)?;
    let catalog = read_catalog(connection)?;
    let mut grants = read_grants(connection)?;
    for grant in &mut grants {
        grant.scopes = read_scopes(connection, &grant.grant_id)?;
        grant.limits = read_limits(connection, &grant.grant_id)?;
    }
    let approvals = read_approvals(connection)?;
    let bridges = read_bridges(connection)?;
    Ok(GrantRegistrySnapshot {
        schema: "homeserver.wrapper-grants.v1".to_owned(),
        active_grants: grants
            .iter()
            .filter(|grant| grant.state == "active")
            .count() as u64,
        pending_approvals: approvals
            .iter()
            .filter(|approval| approval.state == "pending")
            .count() as u64,
        active_bridges: bridges
            .iter()
            .filter(|bridge| bridge.state == "active")
            .count() as u64,
        catalog,
        grants,
        approvals,
        bridges,
        pairing_implies_authority: false,
    })
}

include!("wrapper_grants_lifecycle.rs");
include!("wrapper_grants_approvals.rs");
include!("wrapper_grants_authorize.rs");
include!("wrapper_grants_bridge_create.rs");
include!("wrapper_grants_bridge_revoke.rs");
include!("wrapper_grants_bridge_authorize.rs");
include!("wrapper_grants_decision.rs");
include!("wrapper_grants_expiration.rs");
include!("wrapper_grants_context.rs");
include!("wrapper_grants_storage.rs");
include!("wrapper_grants_usage.rs");
include!("wrapper_grants_validation.rs");
include!("wrapper_grants_read.rs");
