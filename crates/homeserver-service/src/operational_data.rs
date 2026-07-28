use crate::AppState;
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

const OPERATIONAL_MIGRATION: &str =
    include_str!("../../../database/migrations/0012_operational_data_import.sql");
const OPERATIONAL_MIGRATION_KEY: &str = "0012_operational_data_import";
const MAX_CONTROL_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECORDS_PER_IMPORT: usize = 250;
const MAX_EVENTS_PER_IMPORT: usize = 250;
const MAX_RECORD_BYTES: usize = 128 * 1024;
const MAX_QUERY_LIMIT: u32 = 100;
const LOCAL_APPROVER: &str = "local_control_center";
const PERMITTED_AGENT_USES: &[&str] = &["read", "analyze", "goal_match", "report"];
const UNTRUSTED_PROVIDER_EVIDENCE: &str = "untrusted_provider_evidence";

#[derive(Debug, Clone, Copy)]
struct BuiltinDataset {
    key: &'static str,
    label: &'static str,
    description: &'static str,
    sensitivity: &'static str,
    retention_days: i64,
}

const MICROGIFTER_DATASETS: &[BuiltinDataset] = &[
    BuiltinDataset { key: "merchant.profile", label: "Merchant Profile", description: "Provider-authoritative merchant identity and public operating details.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "merchant.locations", label: "Merchant Locations", description: "Provider-authoritative merchant sites, geographic coordinates, and operating metadata.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "merchant.products", label: "Products", description: "Products, pricing, availability, and catalog attributes approved for local analysis.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "campaigns.summary", label: "Campaign Summary", description: "Campaign definitions and lifecycle summaries without publishing authority.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "campaigns.performance", label: "Campaign Performance", description: "Aggregated campaign engagement, claim, redemption, and conversion measures.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "rewards.summary", label: "Rewards Summary", description: "Aggregated reward definitions and performance measures.", sensitivity: "business", retention_days: 365 },
    BuiltinDataset { key: "claims.summary", label: "Claims Summary", description: "Aggregated claim activity without gift ownership or claim mutation authority.", sensitivity: "restricted", retention_days: 180 },
    BuiltinDataset { key: "redemptions.summary", label: "Redemptions Summary", description: "Aggregated redemption activity without redemption authority.", sensitivity: "restricted", retention_days: 180 },
    BuiltinDataset { key: "crm.lifecycle_summary", label: "CRM Lifecycle Summary", description: "Aggregated customer lifecycle stages and engagement indicators without contact details.", sensitivity: "restricted", retention_days: 180 },
    BuiltinDataset { key: "creator.attribution_summary", label: "Creator Attribution Summary", description: "Aggregated creator, referral, and campaign attribution measures.", sensitivity: "business", retention_days: 365 },
];

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalDatasetStatus {
    pub connection_id: String,
    pub connection_name: String,
    pub provider_key: String,
    pub tenant_id: Option<String>,
    pub site_id: Option<String>,
    pub dataset_key: String,
    pub label: String,
    pub description: String,
    pub authority: String,
    pub sensitivity: String,
    pub sync_modes: Vec<String>,
    pub grant_state: String,
    pub classification: String,
    pub retention_days: i64,
    pub permitted_agent_uses: Vec<String>,
    pub last_successful_sync_utc: Option<String>,
    pub cursor_value: Option<String>,
    pub source_revision: Option<String>,
    pub record_count: u64,
    pub event_count: u64,
    pub latest_source_updated_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalImportRunSummary {
    pub import_run_id: String,
    pub connection_id: String,
    pub provider_key: String,
    pub dataset_key: String,
    pub import_mode: String,
    pub state: String,
    pub cursor_before: Option<String>,
    pub cursor_after: Option<String>,
    pub source_revision: Option<String>,
    pub records_received: u64,
    pub records_imported: u64,
    pub records_rejected: u64,
    pub events_received: u64,
    pub failure_code: Option<String>,
    pub started_at_utc: String,
    pub completed_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalDataSnapshot {
    pub datasets: Vec<OperationalDatasetStatus>,
    pub recent_runs: Vec<OperationalImportRunSummary>,
    pub provider_manifests: u64,
    pub enabled_grants: u64,
    pub imported_records: u64,
    pub imported_events: u64,
    pub quarantined_errors: u64,
    pub local_only: bool,
    pub provider_authoritative: bool,
    pub imported_data_is_untrusted_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalEvidenceRecord {
    pub entity_id: String,
    pub connection_id: String,
    pub provider_key: String,
    pub tenant_id: Option<String>,
    pub site_id: Option<String>,
    pub dataset_key: String,
    pub source_object_type: String,
    pub source_object_id: String,
    pub source_revision: String,
    pub source_updated_at_utc: Option<String>,
    pub received_at_utc: String,
    pub payload_hash: String,
    pub payload: Value,
    pub citation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalQueryResult {
    pub records: Vec<OperationalEvidenceRecord>,
    pub available_records: u64,
    pub selected_connection_id: Option<String>,
    pub selected_dataset_key: Option<String>,
    pub generated_at_utc: String,
    pub provider_authoritative: bool,
    pub imported_data_is_untrusted_evidence: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateDatasetGrantRequest {
    pub connection_id: String,
    pub dataset_key: String,
    pub enabled: bool,
    pub retention_days: Option<i64>,
    pub classification: Option<String>,
    #[serde(default)]
    pub permitted_agent_uses: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRecordInput {
    pub source_object_type: String,
    pub source_object_id: String,
    pub source_revision: String,
    pub source_updated_at_utc: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEventInput {
    pub source_event_id: String,
    pub event_type: String,
    pub source_revision: Option<String>,
    pub occurred_at_utc: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImportOperationalBatchRequest {
    pub connection_id: String,
    pub provider_key: String,
    pub tenant_id: Option<String>,
    pub site_id: Option<String>,
    pub dataset_key: String,
    pub import_mode: String,
    pub cursor_after: Option<String>,
    pub source_revision: Option<String>,
    #[serde(default)]
    pub records: Vec<ProviderRecordInput>,
    #[serde(default)]
    pub events: Vec<ProviderEventInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportOperationalBatchResult {
    pub import_run_id: String,
    pub state: String,
    pub records_received: u64,
    pub records_imported: u64,
    pub records_rejected: u64,
    pub events_received: u64,
    pub cursor_after: Option<String>,
    pub source_revision: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OperationalQueryRequest {
    pub connection_id: Option<String>,
    pub dataset_key: Option<String>,
    pub source_object_type: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
struct ConnectionIdentity {
    provider_key: String,
    tenant_id: Option<String>,
    site_id: Option<String>,
    state: String,
}

#[derive(Debug, Clone)]
struct GrantIdentity {
    classification: String,
    retention_days: i64,
    permitted_agent_uses: Vec<String>,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(OPERATIONAL_MIGRATION)?;
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![OPERATIONAL_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "operational data migration is not registered exactly once"
    );
    seed_builtin_manifests(connection)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    for table in [
        "operational_provider_manifests",
        "operational_dataset_catalog",
        "operational_dataset_grants",
        "operational_import_runs",
        "operational_import_cursors",
        "operational_raw_records",
        "operational_entities",
        "operational_entity_versions",
        "operational_events",
        "operational_provenance",
        "operational_retention_policies",
        "operational_import_errors",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    for table in ["operational_raw_records", "operational_events"] {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE trust_state<>?1");
        let invalid: i64 =
            connection.query_row(&sql, params![UNTRUSTED_PROVIDER_EVIDENCE], |row| row.get(0))?;
        ensure!(
            invalid == 0,
            "operational provider evidence contains an invalid trust state"
        );
    }
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    let now = now_string();
    connection.execute(
        "DELETE FROM operational_entities WHERE retention_until_utc<=?1",
        params![now],
    )?;
    connection.execute(
        "DELETE FROM operational_events WHERE retention_until_utc<=?1",
        params![now],
    )?;
    connection.execute(
        "DELETE FROM operational_import_runs WHERE completed_at_utc IS NOT NULL AND completed_at_utc<strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days') AND import_run_id NOT IN (SELECT import_run_id FROM operational_raw_records UNION SELECT import_run_id FROM operational_events)",
        [],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/operational-data", get(snapshot))
        .route("/v1/operational-data/grants", post(update_grant))
        .route("/v1/operational-data/import", post(import_batch))
        .route("/v1/operational-data/query", post(query_records))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn snapshot(State(state): State<Arc<AppState>>) -> ApiResult<OperationalDataSnapshot> {
    tokio::task::spawn_blocking(move || snapshot_for_state(&state))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("operational_snapshot_failed", error))
}

async fn update_grant(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdateDatasetGrantRequest>,
) -> ApiResult<OperationalDataSnapshot> {
    tokio::task::spawn_blocking(move || {
        update_dataset_grant(&state, request)?;
        snapshot_for_state(&state)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| action_error("operational_grant_rejected", error))
}

async fn import_batch(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ImportOperationalBatchRequest>,
) -> ApiResult<ImportOperationalBatchResult> {
    tokio::task::spawn_blocking(move || import_operational_batch(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("operational_import_rejected", error))
}

async fn query_records(
    State(state): State<Arc<AppState>>,
    Json(request): Json<OperationalQueryRequest>,
) -> ApiResult<OperationalQueryResult> {
    tokio::task::spawn_blocking(move || query_operational_records(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("operational_query_rejected", error))
}

pub(crate) fn snapshot_for_state(state: &AppState) -> Result<OperationalDataSnapshot> {
    let connection = state.connection()?;
    snapshot_from_connection(&connection)
}

pub(crate) fn query_for_agent(
    state: &AppState,
    connection_ids: &[String],
    dataset_keys: &[String],
) -> Result<OperationalQueryResult> {
    let selections = agent_dataset_selections(dataset_keys)?;
    let connection = state.connection()?;
    if selections.is_empty() {
        return query_from_connection(
            &connection,
            connection_ids.first().map(String::as_str),
            None,
            None,
            25,
        );
    }

    let mut records = Vec::new();
    let mut available_records = 0_u64;
    for (connection_id, dataset_key) in &selections {
        let grant = enabled_grant(&connection, connection_id, dataset_key)?;
        ensure!(
            grant.permitted_agent_uses.iter().any(|use_name| {
                ["read", "analyze", "goal_match", "report"].contains(&use_name.as_str())
            }),
            "dataset grant does not permit Agent Workspace use"
        );
        let result = query_from_connection(
            &connection,
            Some(connection_id),
            Some(dataset_key),
            None,
            12,
        )?;
        available_records = available_records.saturating_add(result.available_records);
        for record in result.records {
            if records.len() >= 25 {
                break;
            }
            records.push(record);
        }
    }
    Ok(OperationalQueryResult {
        records,
        available_records,
        selected_connection_id: if selections.len() == 1 {
            Some(selections[0].0.clone())
        } else {
            None
        },
        selected_dataset_key: if selections.len() == 1 {
            Some(selections[0].1.clone())
        } else {
            None
        },
        generated_at_utc: now_string(),
        provider_authoritative: true,
        imported_data_is_untrusted_evidence: true,
    })
}

fn agent_dataset_selections(dataset_keys: &[String]) -> Result<Vec<(String, String)>> {
    let mut selections = Vec::new();
    for key in dataset_keys {
        let Some(rest) = key.strip_prefix("dataset:") else {
            continue;
        };
        let mut parts = rest.splitn(2, ':');
        let connection_id = parts.next().context("dataset connection id is missing")?;
        let dataset_key = parts.next().context("dataset key is missing")?;
        validate_uuid(connection_id, "dataset connection id")?;
        let selection = (
            connection_id.to_owned(),
            normalize_dataset_key(dataset_key)?,
        );
        if !selections.contains(&selection) {
            selections.push(selection);
        }
    }
    Ok(selections)
}

fn seed_builtin_manifests(connection: &Connection) -> Result<()> {
    let datasets = MICROGIFTER_DATASETS
        .iter()
        .map(|dataset| {
            json!({
                "key": dataset.key,
                "label": dataset.label,
                "description": dataset.description,
                "authority": "microgifter",
                "sensitivity": dataset.sensitivity,
                "sync_modes": ["snapshot", "incremental", "event"],
                "default_retention_days": dataset.retention_days,
            })
        })
        .collect::<Vec<_>>();
    let manifest = json!({
        "provider": "microgifter",
        "schema_version": "1.0",
        "authority": "microgifter",
        "datasets": datasets,
    });
    let manifest_json = canonical_json(&manifest)?;
    let manifest_hash = sha256_hex(manifest_json.as_bytes());
    connection.execute(
        "INSERT INTO operational_provider_manifests (provider_key,schema_version,manifest_json,manifest_hash,authority,state) VALUES ('microgifter','1.0',?1,?2,'microgifter','active') ON CONFLICT(provider_key) DO UPDATE SET schema_version=excluded.schema_version,manifest_json=excluded.manifest_json,manifest_hash=excluded.manifest_hash,authority=excluded.authority,state='active',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![manifest_json, manifest_hash],
    )?;
    for dataset in MICROGIFTER_DATASETS {
        connection.execute(
            "INSERT INTO operational_dataset_catalog (provider_key,dataset_key,label,description,authority,sensitivity,sync_modes_json,default_retention_days,enabled) VALUES ('microgifter',?1,?2,?3,'microgifter',?4,'[\"snapshot\",\"incremental\",\"event\"]',?5,1) ON CONFLICT(provider_key,dataset_key) DO UPDATE SET label=excluded.label,description=excluded.description,authority=excluded.authority,sensitivity=excluded.sensitivity,sync_modes_json=excluded.sync_modes_json,default_retention_days=excluded.default_retention_days,enabled=1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
            params![dataset.key, dataset.label, dataset.description, dataset.sensitivity, dataset.retention_days],
        )?;
    }
    Ok(())
}

fn update_dataset_grant(state: &AppState, request: UpdateDatasetGrantRequest) -> Result<()> {
    validate_uuid(&request.connection_id, "connection id")?;
    let dataset_key = normalize_dataset_key(&request.dataset_key)?;
    let connection = state.connection()?;
    let identity = connection_identity(&connection, &request.connection_id)?;
    ensure!(
        identity.state != "revoked" && identity.state != "disconnected",
        "cloud connection is inactive"
    );
    let (catalog_sensitivity, default_retention): (String, i64) = connection
        .query_row(
            "SELECT sensitivity,default_retention_days FROM operational_dataset_catalog WHERE provider_key=?1 AND dataset_key=?2 AND enabled=1",
            params![identity.provider_key, dataset_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .context("provider dataset is not declared in the active manifest")?;
    let classification = request
        .classification
        .as_deref()
        .map(normalize_classification)
        .transpose()?
        .unwrap_or_else(|| catalog_sensitivity.clone());
    ensure!(
        classification_rank(&classification) >= classification_rank(&catalog_sensitivity),
        "grant classification cannot be lower than the provider dataset sensitivity"
    );
    let retention_days = request
        .retention_days
        .unwrap_or(default_retention)
        .clamp(1, 3650);
    let uses = normalize_agent_uses(&request.permitted_agent_uses)?;
    let now = now_string();
    if request.enabled {
        connection.execute(
            "INSERT INTO operational_dataset_grants (grant_id,connection_id,provider_key,tenant_id,site_id,dataset_key,classification,retention_days,permitted_agent_uses_json,state,approved_by,approved_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'enabled',?10,?11) ON CONFLICT(connection_id,dataset_key) DO UPDATE SET provider_key=excluded.provider_key,tenant_id=excluded.tenant_id,site_id=excluded.site_id,classification=excluded.classification,retention_days=excluded.retention_days,permitted_agent_uses_json=excluded.permitted_agent_uses_json,state='enabled',approved_by=excluded.approved_by,approved_at_utc=excluded.approved_at_utc,updated_at_utc=excluded.approved_at_utc",
            params![Uuid::new_v4().to_string(), request.connection_id, identity.provider_key, identity.tenant_id, identity.site_id, dataset_key, classification, retention_days, serde_json::to_string(&uses)?, LOCAL_APPROVER, now],
        )?;
        connection.execute(
            "INSERT INTO operational_retention_policies (connection_id,dataset_key,retention_days,disconnect_policy,updated_by,updated_at_utc) VALUES (?1,?2,?3,'retain',?4,?5) ON CONFLICT(connection_id,dataset_key) DO UPDATE SET retention_days=excluded.retention_days,updated_by=excluded.updated_by,updated_at_utc=excluded.updated_at_utc",
            params![request.connection_id, dataset_key, retention_days, LOCAL_APPROVER, now],
        )?;
    } else {
        let affected = connection.execute(
            "UPDATE operational_dataset_grants SET state='paused',updated_at_utc=?3 WHERE connection_id=?1 AND dataset_key=?2 AND state!='revoked'",
            params![request.connection_id, dataset_key, now],
        )?;
        ensure!(affected == 1, "enabled dataset grant was not found");
    }
    Ok(())
}

fn import_operational_batch(
    state: &AppState,
    mut request: ImportOperationalBatchRequest,
) -> Result<ImportOperationalBatchResult> {
    validate_uuid(&request.connection_id, "connection id")?;
    request.provider_key = normalize_provider_key(&request.provider_key)?;
    request.dataset_key = normalize_dataset_key(&request.dataset_key)?;
    request.import_mode = request.import_mode.trim().to_ascii_lowercase();
    ensure!(
        ["snapshot", "incremental", "event"].contains(&request.import_mode.as_str()),
        "import mode is invalid"
    );
    ensure!(
        request.records.len() <= MAX_RECORDS_PER_IMPORT,
        "operational import contains too many records"
    );
    ensure!(
        request.events.len() <= MAX_EVENTS_PER_IMPORT,
        "operational import contains too many events"
    );
    ensure!(
        !request.records.is_empty() || !request.events.is_empty(),
        "operational import is empty"
    );
    request.tenant_id = sanitize_optional_text(request.tenant_id.as_deref(), 160, "tenant id")?;
    request.site_id = sanitize_optional_text(request.site_id.as_deref(), 160, "site id")?;
    request.cursor_after =
        sanitize_optional_text(request.cursor_after.as_deref(), 500, "import cursor")?;
    request.source_revision =
        sanitize_optional_text(request.source_revision.as_deref(), 300, "source revision")?;

    let mut connection = state.connection()?;
    let identity = connection_identity(&connection, &request.connection_id)?;
    ensure!(
        identity.provider_key == request.provider_key,
        "provider key does not match the paired connection"
    );
    ensure!(
        identity.state != "revoked" && identity.state != "disconnected",
        "cloud connection is inactive"
    );
    ensure_scope_matches(
        identity.tenant_id.as_deref(),
        request.tenant_id.as_deref(),
        "tenant",
    )?;
    ensure_scope_matches(
        identity.site_id.as_deref(),
        request.site_id.as_deref(),
        "site",
    )?;
    let grant = enabled_grant(&connection, &request.connection_id, &request.dataset_key)?;
    let supported_modes: String = connection.query_row(
        "SELECT sync_modes_json FROM operational_dataset_catalog WHERE provider_key=?1 AND dataset_key=?2 AND enabled=1",
        params![request.provider_key, request.dataset_key],
        |row| row.get(0),
    )?;
    let supported_modes: Vec<String> = serde_json::from_str(&supported_modes)?;
    ensure!(
        supported_modes.contains(&request.import_mode),
        "provider manifest does not allow this import mode"
    );

    let cursor_before = connection
        .query_row(
            "SELECT cursor_value FROM operational_import_cursors WHERE connection_id=?1 AND dataset_key=?2",
            params![request.connection_id, request.dataset_key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let import_run_id = Uuid::new_v4().to_string();
    let started_at = now_string();
    let retention_until = (Utc::now() + ChronoDuration::days(grant.retention_days))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO operational_import_runs (import_run_id,connection_id,provider_key,tenant_id,site_id,dataset_key,import_mode,state,cursor_before,cursor_after,source_revision,records_received,events_received,started_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'running',?8,?9,?10,?11,?12,?13)",
        params![import_run_id, request.connection_id, request.provider_key, request.tenant_id, request.site_id, request.dataset_key, request.import_mode, cursor_before, request.cursor_after, request.source_revision, request.records.len() as i64, request.events.len() as i64, started_at],
    )?;

    let mut imported = 0_u64;
    let mut rejected = 0_u64;
    for (index, record) in request.records.iter().enumerate() {
        match import_record(
            &transaction,
            &import_run_id,
            &request,
            &grant,
            record,
            &retention_until,
        ) {
            Ok(was_imported) => {
                if was_imported {
                    imported += 1;
                }
            }
            Err(error) => {
                rejected += 1;
                record_import_error(&transaction, &import_run_id, index, record, &error)?;
            }
        }
    }
    for event in &request.events {
        if let Err(error) = import_event(
            &transaction,
            &import_run_id,
            &request,
            &grant,
            event,
            &retention_until,
        ) {
            rejected += 1;
            transaction.execute(
                "INSERT INTO operational_import_errors (import_error_id,import_run_id,error_code,message,payload_hash) VALUES (?1,?2,'event_rejected',?3,?4)",
                params![Uuid::new_v4().to_string(), import_run_id, truncate_chars(&error.to_string(), 500), hash_value(&event.payload).ok()],
            )?;
        }
    }
    let completed_at = now_string();
    let state_value = if rejected == 0 {
        "completed"
    } else {
        "completed_with_errors"
    };
    transaction.execute(
        "UPDATE operational_import_runs SET state=?2,records_imported=?3,records_rejected=?4,completed_at_utc=?5 WHERE import_run_id=?1",
        params![import_run_id, state_value, imported, rejected, completed_at],
    )?;
    transaction.execute(
        "INSERT INTO operational_import_cursors (connection_id,dataset_key,cursor_value,source_revision,last_successful_sync_utc,last_attempt_utc,records_received,records_rejected) VALUES (?1,?2,?3,?4,?5,?5,?6,?7) ON CONFLICT(connection_id,dataset_key) DO UPDATE SET cursor_value=excluded.cursor_value,source_revision=excluded.source_revision,last_successful_sync_utc=excluded.last_successful_sync_utc,last_attempt_utc=excluded.last_attempt_utc,records_received=operational_import_cursors.records_received+excluded.records_received,records_rejected=operational_import_cursors.records_rejected+excluded.records_rejected",
        params![request.connection_id, request.dataset_key, request.cursor_after, request.source_revision, completed_at, request.records.len() as i64, rejected],
    )?;
    transaction.commit()?;
    Ok(ImportOperationalBatchResult {
        import_run_id,
        state: state_value.to_owned(),
        records_received: request.records.len() as u64,
        records_imported: imported,
        records_rejected: rejected,
        events_received: request.events.len() as u64,
        cursor_after: request.cursor_after,
        source_revision: request.source_revision,
    })
}

fn import_record(
    transaction: &rusqlite::Transaction<'_>,
    import_run_id: &str,
    request: &ImportOperationalBatchRequest,
    grant: &GrantIdentity,
    input: &ProviderRecordInput,
    retention_until: &str,
) -> Result<bool> {
    let object_type = normalize_object_key(&input.source_object_type, "source object type")?;
    let object_id = sanitize_required_text(&input.source_object_id, 300, "source object id")?;
    let revision = sanitize_required_text(&input.source_revision, 300, "source revision")?;
    let source_updated = sanitize_optional_text(
        input.source_updated_at_utc.as_deref(),
        80,
        "source update time",
    )?;
    ensure_json_object(&input.payload, MAX_RECORD_BYTES, "operational record")?;
    let payload_json = canonical_json(&input.payload)?;
    let payload_hash = sha256_hex(payload_json.as_bytes());
    let raw_record_id = Uuid::new_v4().to_string();
    let received_at = now_string();
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO operational_raw_records (raw_record_id,import_run_id,connection_id,provider_key,tenant_id,site_id,dataset_key,source_object_type,source_object_id,source_revision,source_updated_at_utc,classification,payload_json,payload_hash,received_at_utc,retention_until_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![raw_record_id, import_run_id, request.connection_id, request.provider_key, request.tenant_id, request.site_id, request.dataset_key, object_type, object_id, revision, source_updated, grant.classification, payload_json, payload_hash, received_at, retention_until],
    )?;
    if inserted == 0 {
        return Ok(false);
    }
    let entity_id = sha256_hex(
        format!(
            "{}|{}|{}|{}",
            request.connection_id, request.dataset_key, object_type, object_id
        )
        .as_bytes(),
    );
    transaction.execute(
        "INSERT INTO operational_entities (entity_id,connection_id,provider_key,tenant_id,site_id,dataset_key,source_object_type,source_object_id,current_source_revision,current_payload_hash,current_payload_json,classification,source_updated_at_utc,received_at_utc,retention_until_utc,state,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'active',?14,?14) ON CONFLICT(connection_id,dataset_key,source_object_type,source_object_id) DO UPDATE SET current_source_revision=excluded.current_source_revision,current_payload_hash=excluded.current_payload_hash,current_payload_json=excluded.current_payload_json,classification=excluded.classification,source_updated_at_utc=excluded.source_updated_at_utc,received_at_utc=excluded.received_at_utc,retention_until_utc=excluded.retention_until_utc,state='active',updated_at_utc=excluded.updated_at_utc",
        params![entity_id, request.connection_id, request.provider_key, request.tenant_id, request.site_id, request.dataset_key, object_type, object_id, revision, payload_hash, payload_json, grant.classification, source_updated, received_at, retention_until],
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO operational_entity_versions (version_id,entity_id,raw_record_id,source_revision,payload_hash,payload_json,effective_at_utc,received_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![Uuid::new_v4().to_string(), entity_id, raw_record_id, revision, payload_hash, payload_json, source_updated, received_at],
    )?;
    transaction.execute(
        "INSERT INTO operational_provenance (provenance_id,entity_id,import_run_id,connection_id,provider_key,tenant_id,site_id,dataset_key,source_object_type,source_object_id,source_revision,evidence_hash,received_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![Uuid::new_v4().to_string(), entity_id, import_run_id, request.connection_id, request.provider_key, request.tenant_id, request.site_id, request.dataset_key, object_type, object_id, revision, payload_hash, received_at],
    )?;
    Ok(true)
}

fn import_event(
    transaction: &rusqlite::Transaction<'_>,
    import_run_id: &str,
    request: &ImportOperationalBatchRequest,
    _grant: &GrantIdentity,
    input: &ProviderEventInput,
    retention_until: &str,
) -> Result<()> {
    let source_event_id = sanitize_required_text(&input.source_event_id, 300, "source event id")?;
    let event_type = normalize_object_key(&input.event_type, "event type")?;
    let revision = sanitize_optional_text(input.source_revision.as_deref(), 300, "event revision")?;
    let occurred_at = sanitize_required_text(&input.occurred_at_utc, 80, "event occurrence time")?;
    ensure_json_object(&input.payload, MAX_RECORD_BYTES, "operational event")?;
    let payload_json = canonical_json(&input.payload)?;
    let payload_hash = sha256_hex(payload_json.as_bytes());
    let event_id = Uuid::new_v4().to_string();
    let received_at = now_string();
    let inserted = transaction.execute(
        "INSERT OR IGNORE INTO operational_events (event_id,import_run_id,connection_id,provider_key,tenant_id,site_id,dataset_key,event_type,source_event_id,source_revision,occurred_at_utc,payload_json,payload_hash,received_at_utc,retention_until_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![event_id, import_run_id, request.connection_id, request.provider_key, request.tenant_id, request.site_id, request.dataset_key, event_type, source_event_id, revision, occurred_at, payload_json, payload_hash, received_at, retention_until],
    )?;
    if inserted == 1 {
        transaction.execute(
            "INSERT INTO operational_provenance (provenance_id,event_id,import_run_id,connection_id,provider_key,tenant_id,site_id,dataset_key,source_revision,evidence_hash,received_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![Uuid::new_v4().to_string(), event_id, import_run_id, request.connection_id, request.provider_key, request.tenant_id, request.site_id, request.dataset_key, revision, payload_hash, received_at],
        )?;
    }
    Ok(())
}

fn record_import_error(
    transaction: &rusqlite::Transaction<'_>,
    import_run_id: &str,
    index: usize,
    record: &ProviderRecordInput,
    error: &anyhow::Error,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO operational_import_errors (import_error_id,import_run_id,record_index,source_object_id,error_code,message,payload_hash) VALUES (?1,?2,?3,?4,'record_rejected',?5,?6)",
        params![Uuid::new_v4().to_string(), import_run_id, index as i64, truncate_chars(&record.source_object_id, 300), truncate_chars(&error.to_string(), 500), hash_value(&record.payload).ok()],
    )?;
    Ok(())
}

fn query_operational_records(
    state: &AppState,
    request: OperationalQueryRequest,
) -> Result<OperationalQueryResult> {
    let connection_id = request
        .connection_id
        .as_deref()
        .map(|value| {
            validate_uuid(value, "connection id")?;
            Ok::<String, anyhow::Error>(value.to_owned())
        })
        .transpose()?;
    let dataset_key = request
        .dataset_key
        .as_deref()
        .map(normalize_dataset_key)
        .transpose()?;
    let object_type = request
        .source_object_type
        .as_deref()
        .map(|value| normalize_object_key(value, "source object type"))
        .transpose()?;
    let connection = state.connection()?;
    query_from_connection(
        &connection,
        connection_id.as_deref(),
        dataset_key.as_deref(),
        object_type.as_deref(),
        request.limit.unwrap_or(50).clamp(1, MAX_QUERY_LIMIT),
    )
}

fn query_from_connection(
    connection: &Connection,
    connection_id: Option<&str>,
    dataset_key: Option<&str>,
    object_type: Option<&str>,
    limit: u32,
) -> Result<OperationalQueryResult> {
    if let Some(connection_id) = connection_id {
        validate_uuid(connection_id, "connection id")?;
    }
    if let (Some(connection_id), Some(dataset_key)) = (connection_id, dataset_key) {
        let _ = enabled_grant(connection, connection_id, dataset_key)?;
    }
    let available_records: i64 = connection.query_row(
        "SELECT COUNT(*) FROM operational_entities e JOIN operational_dataset_grants g ON g.connection_id=e.connection_id AND g.dataset_key=e.dataset_key AND g.state='enabled' WHERE e.state='active' AND (?1 IS NULL OR e.connection_id=?1) AND (?2 IS NULL OR e.dataset_key=?2) AND (?3 IS NULL OR e.source_object_type=?3)",
        params![connection_id, dataset_key, object_type],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(
        "SELECT e.entity_id,e.connection_id,e.provider_key,e.tenant_id,e.site_id,e.dataset_key,e.source_object_type,e.source_object_id,e.current_source_revision,e.source_updated_at_utc,e.received_at_utc,e.current_payload_hash,e.current_payload_json FROM operational_entities e JOIN operational_dataset_grants g ON g.connection_id=e.connection_id AND g.dataset_key=e.dataset_key AND g.state='enabled' WHERE e.state='active' AND (?1 IS NULL OR e.connection_id=?1) AND (?2 IS NULL OR e.dataset_key=?2) AND (?3 IS NULL OR e.source_object_type=?3) ORDER BY COALESCE(e.source_updated_at_utc,e.received_at_utc) DESC,e.entity_id DESC LIMIT ?4",
    )?;
    let rows = statement.query_map(
        params![connection_id, dataset_key, object_type, i64::from(limit)],
        map_evidence_record,
    )?;
    let records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(OperationalQueryResult {
        records,
        available_records: available_records.max(0) as u64,
        selected_connection_id: connection_id.map(ToOwned::to_owned),
        selected_dataset_key: dataset_key.map(ToOwned::to_owned),
        generated_at_utc: now_string(),
        provider_authoritative: true,
        imported_data_is_untrusted_evidence: true,
    })
}

fn snapshot_from_connection(connection: &Connection) -> Result<OperationalDataSnapshot> {
    let mut statement = connection.prepare(
        "SELECT c.connection_id,c.display_name,c.provider_key,c.tenant_id,c.site_id,d.dataset_key,d.label,d.description,d.authority,d.sensitivity,d.sync_modes_json,COALESCE(g.state,'not_granted'),COALESCE(g.classification,d.sensitivity),COALESCE(g.retention_days,d.default_retention_days),COALESCE(g.permitted_agent_uses_json,'[]'),u.last_successful_sync_utc,u.cursor_value,u.source_revision,(SELECT COUNT(*) FROM operational_entities e WHERE e.connection_id=c.connection_id AND e.dataset_key=d.dataset_key AND e.state='active'),(SELECT COUNT(*) FROM operational_events v WHERE v.connection_id=c.connection_id AND v.dataset_key=d.dataset_key),(SELECT MAX(source_updated_at_utc) FROM operational_entities e WHERE e.connection_id=c.connection_id AND e.dataset_key=d.dataset_key) FROM cloud_connections c JOIN operational_dataset_catalog d ON d.provider_key=c.provider_key AND d.enabled=1 LEFT JOIN operational_dataset_grants g ON g.connection_id=c.connection_id AND g.dataset_key=d.dataset_key LEFT JOIN operational_import_cursors u ON u.connection_id=c.connection_id AND u.dataset_key=d.dataset_key WHERE c.state!='revoked' ORDER BY c.display_name,d.label",
    )?;
    let datasets = statement
        .query_map([], map_dataset_status)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut run_statement = connection.prepare(
        "SELECT import_run_id,connection_id,provider_key,dataset_key,import_mode,state,cursor_before,cursor_after,source_revision,records_received,records_imported,records_rejected,events_received,failure_code,started_at_utc,completed_at_utc FROM operational_import_runs ORDER BY started_at_utc DESC,import_run_id DESC LIMIT 50",
    )?;
    let recent_runs = run_statement
        .query_map([], map_import_run)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let provider_manifests = scalar_count(
        connection,
        "SELECT COUNT(*) FROM operational_provider_manifests WHERE state='active'",
    )?;
    let enabled_grants = scalar_count(
        connection,
        "SELECT COUNT(*) FROM operational_dataset_grants WHERE state='enabled'",
    )?;
    let imported_records = scalar_count(
        connection,
        "SELECT COUNT(*) FROM operational_entities WHERE state='active'",
    )?;
    let imported_events = scalar_count(connection, "SELECT COUNT(*) FROM operational_events")?;
    let quarantined_errors =
        scalar_count(connection, "SELECT COUNT(*) FROM operational_import_errors")?;
    Ok(OperationalDataSnapshot {
        datasets,
        recent_runs,
        provider_manifests,
        enabled_grants,
        imported_records,
        imported_events,
        quarantined_errors,
        local_only: true,
        provider_authoritative: true,
        imported_data_is_untrusted_evidence: true,
    })
}

fn map_dataset_status(row: &Row<'_>) -> rusqlite::Result<OperationalDatasetStatus> {
    Ok(OperationalDatasetStatus {
        connection_id: row.get(0)?,
        connection_name: row.get(1)?,
        provider_key: row.get(2)?,
        tenant_id: row.get(3)?,
        site_id: row.get(4)?,
        dataset_key: row.get(5)?,
        label: row.get(6)?,
        description: row.get(7)?,
        authority: row.get(8)?,
        sensitivity: row.get(9)?,
        sync_modes: parse_json_column(row.get::<_, String>(10)?),
        grant_state: row.get(11)?,
        classification: row.get(12)?,
        retention_days: row.get(13)?,
        permitted_agent_uses: parse_json_column(row.get::<_, String>(14)?),
        last_successful_sync_utc: row.get(15)?,
        cursor_value: row.get(16)?,
        source_revision: row.get(17)?,
        record_count: row.get::<_, i64>(18)?.max(0) as u64,
        event_count: row.get::<_, i64>(19)?.max(0) as u64,
        latest_source_updated_at_utc: row.get(20)?,
    })
}

fn map_import_run(row: &Row<'_>) -> rusqlite::Result<OperationalImportRunSummary> {
    Ok(OperationalImportRunSummary {
        import_run_id: row.get(0)?,
        connection_id: row.get(1)?,
        provider_key: row.get(2)?,
        dataset_key: row.get(3)?,
        import_mode: row.get(4)?,
        state: row.get(5)?,
        cursor_before: row.get(6)?,
        cursor_after: row.get(7)?,
        source_revision: row.get(8)?,
        records_received: row.get::<_, i64>(9)?.max(0) as u64,
        records_imported: row.get::<_, i64>(10)?.max(0) as u64,
        records_rejected: row.get::<_, i64>(11)?.max(0) as u64,
        events_received: row.get::<_, i64>(12)?.max(0) as u64,
        failure_code: row.get(13)?,
        started_at_utc: row.get(14)?,
        completed_at_utc: row.get(15)?,
    })
}

fn map_evidence_record(row: &Row<'_>) -> rusqlite::Result<OperationalEvidenceRecord> {
    let provider_key: String = row.get(2)?;
    let connection_id: String = row.get(1)?;
    let dataset_key: String = row.get(5)?;
    let object_type: String = row.get(6)?;
    let object_id: String = row.get(7)?;
    let revision: String = row.get(8)?;
    let payload_json: String = row.get(12)?;
    Ok(OperationalEvidenceRecord {
        entity_id: row.get(0)?,
        connection_id: connection_id.clone(),
        provider_key: provider_key.clone(),
        tenant_id: row.get(3)?,
        site_id: row.get(4)?,
        dataset_key: dataset_key.clone(),
        source_object_type: object_type.clone(),
        source_object_id: object_id.clone(),
        source_revision: revision.clone(),
        source_updated_at_utc: row.get(9)?,
        received_at_utc: row.get(10)?,
        payload_hash: row.get(11)?,
        payload: serde_json::from_str(&payload_json)
            .unwrap_or_else(|_| json!({ "invalid_payload": true })),
        citation: format!(
            "{provider_key}:{connection_id}/{dataset_key}/{object_type}/{object_id}@{revision}"
        ),
    })
}

fn connection_identity(connection: &Connection, connection_id: &str) -> Result<ConnectionIdentity> {
    connection
        .query_row(
            "SELECT provider_key,display_name,tenant_id,site_id,state FROM cloud_connections WHERE connection_id=?1",
            params![connection_id],
            |row| {
                Ok(ConnectionIdentity {
                    provider_key: row.get(0)?,
                    tenant_id: row.get(2)?,
                    site_id: row.get(3)?,
                    state: row.get(4)?,
                })
            },
        )
        .context("cloud connection was not found")
}

fn enabled_grant(
    connection: &Connection,
    connection_id: &str,
    dataset_key: &str,
) -> Result<GrantIdentity> {
    connection
        .query_row(
            "SELECT classification,retention_days,permitted_agent_uses_json FROM operational_dataset_grants WHERE connection_id=?1 AND dataset_key=?2 AND state='enabled'",
            params![connection_id, dataset_key],
            |row| {
                Ok(GrantIdentity {
                    classification: row.get(0)?,
                    retention_days: row.get(1)?,
                    permitted_agent_uses: parse_json_column(row.get::<_, String>(2)?),
                })
            },
        )
        .context("dataset is not authorized for import")
}

fn ensure_scope_matches(expected: Option<&str>, received: Option<&str>, label: &str) -> Result<()> {
    match (expected, received) {
        (Some(expected), Some(received)) => ensure!(
            expected == received,
            "{label} scope does not match the paired connection"
        ),
        (Some(_), None) => bail!("{label} scope is required for this paired connection"),
        (None, Some(_)) => bail!("{label} scope cannot be added by an import payload"),
        (None, None) => {}
    }
    Ok(())
}

fn normalize_agent_uses(values: &[String]) -> Result<Vec<String>> {
    let source = if values.is_empty() {
        PERMITTED_AGENT_USES
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>()
    } else {
        values.to_vec()
    };
    ensure!(
        source.len() <= PERMITTED_AGENT_USES.len(),
        "too many agent uses were supplied"
    );
    let mut normalized = Vec::new();
    for value in source {
        let value = value.trim().to_ascii_lowercase();
        ensure!(
            PERMITTED_AGENT_USES.contains(&value.as_str()),
            "agent use is not permitted for imported data"
        );
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    ensure!(
        !normalized.is_empty(),
        "at least one permitted agent use is required"
    );
    Ok(normalized)
}

fn normalize_provider_key(value: &str) -> Result<String> {
    let value = normalize_object_key(value, "provider key")?;
    ensure!(value == "microgifter", "provider adapter is not installed");
    Ok(value)
}

fn normalize_dataset_key(value: &str) -> Result<String> {
    normalize_object_key(value, "dataset key")
}

fn normalize_object_key(value: &str, label: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        (2..=160).contains(&value.len()),
        "{label} length is invalid"
    );
    ensure!(
        value
            .chars()
            .all(|character| character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | '-')),
        "{label} contains unsupported characters"
    );
    Ok(value)
}

fn normalize_classification(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        ["public", "business", "restricted", "sensitive"].contains(&value.as_str()),
        "classification is invalid"
    );
    Ok(value)
}

fn classification_rank(value: &str) -> u8 {
    match value {
        "public" => 0,
        "business" => 1,
        "restricted" => 2,
        "sensitive" => 3,
        _ => 4,
    }
}

fn sanitize_required_text(value: &str, maximum: usize, label: &str) -> Result<String> {
    sanitize_optional_text(Some(value), maximum, label)?.context(format!("{label} is required"))
}

fn sanitize_optional_text(
    value: Option<&str>,
    maximum: usize,
    label: &str,
) -> Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    ensure!(
        value.chars().count() <= maximum,
        "{label} exceeds the size limit"
    );
    ensure!(
        !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t')),
        "{label} contains unsupported control characters"
    );
    Ok(Some(value.to_owned()))
}

fn ensure_json_object(value: &Value, maximum_bytes: usize, label: &str) -> Result<()> {
    ensure!(value.is_object(), "{label} must be one JSON object");
    let encoded = serde_json::to_vec(value)?;
    ensure!(
        encoded.len() <= maximum_bytes,
        "{label} exceeds the size limit"
    );
    Ok(())
}

fn canonical_json(value: &Value) -> Result<String> {
    fn sort_value(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                let mut sorted = Map::new();
                for key in keys {
                    sorted.insert(key.clone(), sort_value(&object[&key]));
                }
                Value::Object(sorted)
            }
            Value::Array(values) => Value::Array(values.iter().map(sort_value).collect()),
            _ => value.clone(),
        }
    }
    serde_json::to_string(&sort_value(value)).map_err(Into::into)
}

fn hash_value(value: &Value) -> Result<String> {
    Ok(sha256_hex(canonical_json(value)?.as_bytes()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn scalar_count(connection: &Connection, sql: &str) -> Result<u64> {
    let count: i64 = connection.query_row(sql, [], |row| row.get(0))?;
    Ok(count.max(0) as u64)
}

fn parse_json_column<T: for<'de> Deserialize<'de> + Default>(value: String) -> T {
    serde_json::from_str(&value).unwrap_or_default()
}

fn validate_uuid(value: &str, label: &str) -> Result<()> {
    ensure!(Uuid::parse_str(value).is_ok(), "{label} is invalid");
    Ok(())
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("operational_task_failed", error.into())
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(?error, code, "HomeServer operational data operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: "HomeServer could not complete the operational data operation.".to_owned(),
        }),
    )
}

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(ApiError {
            ok: false,
            error: code,
            message: truncate_chars(&error.to_string(), 500),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> crate::config::AppConfig {
        let root = std::env::temp_dir().join(format!(
            "microgifter-homeserver-operational-test-{}",
            Uuid::new_v4().simple()
        ));
        let logs_dir = root.join("logs");
        let backups_dir = root.join("backups");
        let recovery_dir = root.join("recovery-packages");
        let restore_dir = root.join("restore");
        let staging_dir = root.join("staging");
        let imports_dir = staging_dir.join("recovery-imports");
        let updates_dir = root.join("updates");
        let update_staging_dir = updates_dir.join("staging");
        let update_rollback_dir = updates_dir.join("rollback");
        let update_installed_dir = updates_dir.join("installed");
        for directory in [
            &root,
            &logs_dir,
            &backups_dir,
            &recovery_dir,
            &restore_dir,
            &staging_dir,
            &imports_dir,
            &updates_dir,
            &update_staging_dir,
            &update_rollback_dir,
            &update_installed_dir,
        ] {
            std::fs::create_dir_all(directory).expect("create test directory");
        }
        crate::config::AppConfig {
            database_path: root.join("homeserver.sqlite3"),
            data_dir: root,
            logs_dir,
            backups_dir,
            recovery_dir,
            restore_dir,
            staging_dir,
            imports_dir,
            updates_dir,
            update_staging_dir,
            update_rollback_dir,
            update_installed_dir,
            update_manifest_url: "https://updates.microgifter.com/homeserver/stable/manifest.json"
                .to_owned(),
            server_name: "Operational Data Test".to_owned(),
        }
    }

    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().expect("open database");
        connection.execute_batch(
            "CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);\
             CREATE TABLE cloud_connections (\
                connection_id TEXT PRIMARY KEY,provider_key TEXT NOT NULL,display_name TEXT NOT NULL,cloud_base_url TEXT NOT NULL,tenant_id TEXT,site_id TEXT,device_id TEXT NOT NULL,state TEXT NOT NULL,scopes_json TEXT NOT NULL,is_default INTEGER NOT NULL DEFAULT 0,paired_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),last_success_utc TEXT,last_error TEXT,credential_key TEXT NOT NULL,public_key_base64 TEXT NOT NULL,created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))\
             );",
        ).expect("base schema");
        initialize(&connection).expect("initialize operational data");
        connection.execute(
            "INSERT INTO cloud_connections (connection_id,provider_key,display_name,cloud_base_url,tenant_id,site_id,device_id,state,scopes_json,credential_key,public_key_base64) VALUES (?1,'microgifter','Test Merchant','https://example.test','tenant-1','site-1',?2,'connected','[]','credential','public')",
            params![Uuid::new_v4().to_string(), Uuid::new_v4().to_string()],
        ).expect("insert connection");
        connection
    }

    fn connection_id(connection: &Connection) -> String {
        connection
            .query_row("SELECT connection_id FROM cloud_connections", [], |row| {
                row.get(0)
            })
            .expect("connection id")
    }

    #[test]
    fn builtin_manifest_is_seeded() {
        let connection = fixture();
        let manifest_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM operational_provider_manifests",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let dataset_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM operational_dataset_catalog",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(manifest_count, 1);
        assert_eq!(dataset_count, MICROGIFTER_DATASETS.len() as i64);
    }

    #[test]
    fn grant_import_and_query_preserve_evidence() {
        let connection = fixture();
        let id = connection_id(&connection);
        let state = AppState {
            config: test_config(),
            connection: std::sync::Mutex::new(connection),
        };
        update_dataset_grant(
            &state,
            UpdateDatasetGrantRequest {
                connection_id: id.clone(),
                dataset_key: "merchant.products".to_owned(),
                enabled: true,
                retention_days: Some(30),
                classification: None,
                permitted_agent_uses: vec![],
            },
        )
        .expect("grant dataset");
        let result = import_operational_batch(
            &state,
            ImportOperationalBatchRequest {
                connection_id: id.clone(),
                provider_key: "microgifter".to_owned(),
                tenant_id: Some("tenant-1".to_owned()),
                site_id: Some("site-1".to_owned()),
                dataset_key: "merchant.products".to_owned(),
                import_mode: "snapshot".to_owned(),
                cursor_after: Some("cursor-1".to_owned()),
                source_revision: Some("catalog-1".to_owned()),
                records: vec![ProviderRecordInput {
                    source_object_type: "product".to_owned(),
                    source_object_id: "product-42".to_owned(),
                    source_revision: "rev-1".to_owned(),
                    source_updated_at_utc: Some("2026-07-27T20:00:00Z".to_owned()),
                    payload: json!({"name":"Lunch Special","price":25}),
                }],
                events: vec![],
            },
        )
        .expect("import data");
        assert_eq!(result.records_imported, 1);
        let query = query_operational_records(
            &state,
            OperationalQueryRequest {
                connection_id: Some(id),
                dataset_key: Some("merchant.products".to_owned()),
                source_object_type: Some("product".to_owned()),
                limit: Some(10),
            },
        )
        .expect("query data");
        assert_eq!(query.records.len(), 1);
        assert_eq!(query.records[0].payload["name"], "Lunch Special");
        assert!(query.records[0].citation.contains("product-42@rev-1"));
        assert!(query.imported_data_is_untrusted_evidence);
    }

    #[test]
    fn ungranted_or_cross_scope_imports_fail_closed() {
        let connection = fixture();
        let id = connection_id(&connection);
        let state = AppState {
            config: test_config(),
            connection: std::sync::Mutex::new(connection),
        };
        let request = ImportOperationalBatchRequest {
            connection_id: id,
            provider_key: "microgifter".to_owned(),
            tenant_id: Some("wrong-tenant".to_owned()),
            site_id: Some("site-1".to_owned()),
            dataset_key: "merchant.products".to_owned(),
            import_mode: "snapshot".to_owned(),
            cursor_after: None,
            source_revision: None,
            records: vec![ProviderRecordInput {
                source_object_type: "product".to_owned(),
                source_object_id: "1".to_owned(),
                source_revision: "1".to_owned(),
                source_updated_at_utc: None,
                payload: json!({"name":"Rejected"}),
            }],
            events: vec![],
        };
        assert!(import_operational_batch(&state, request).is_err());
    }

    #[test]
    fn duplicate_source_revision_is_idempotent() {
        let connection = fixture();
        let id = connection_id(&connection);
        let state = AppState {
            config: test_config(),
            connection: std::sync::Mutex::new(connection),
        };
        update_dataset_grant(
            &state,
            UpdateDatasetGrantRequest {
                connection_id: id.clone(),
                dataset_key: "merchant.profile".to_owned(),
                enabled: true,
                retention_days: None,
                classification: None,
                permitted_agent_uses: vec![],
            },
        )
        .unwrap();
        let make_request = || ImportOperationalBatchRequest {
            connection_id: id.clone(),
            provider_key: "microgifter".to_owned(),
            tenant_id: Some("tenant-1".to_owned()),
            site_id: Some("site-1".to_owned()),
            dataset_key: "merchant.profile".to_owned(),
            import_mode: "incremental".to_owned(),
            cursor_after: Some("2".to_owned()),
            source_revision: Some("2".to_owned()),
            records: vec![ProviderRecordInput {
                source_object_type: "merchant".to_owned(),
                source_object_id: "merchant-1".to_owned(),
                source_revision: "rev-1".to_owned(),
                source_updated_at_utc: None,
                payload: json!({"name":"Test"}),
            }],
            events: vec![],
        };
        assert_eq!(
            import_operational_batch(&state, make_request())
                .unwrap()
                .records_imported,
            1
        );
        assert_eq!(
            import_operational_batch(&state, make_request())
                .unwrap()
                .records_imported,
            0
        );
    }
}
