use crate::{backup_key, config::AppConfig, database, AppState};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{ensure, Context, Result};
use axum::{
    body::Body,
    extract::{Path as AxumPath, State},
    http::{header, HeaderValue, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, types::ValueRef, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tar::{Archive, Builder, Header};
use tokio_util::io::ReaderStream;
use uuid::Uuid;
use zeroize::Zeroizing;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0029_tamper_evident_evidence_archive.sql");
const MIGRATION_KEY: &str = "0029_tamper_evident_evidence_archive";
const PACKAGE_MAGIC: &[u8; 8] = b"MGHEAR01";
const PACKAGE_VERSION: u32 = 1;
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024;
const MAX_RECORDS_HARD: usize = 50_000;
const MAX_PACKAGE_HARD_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ARCHIVES_SNAPSHOT: i64 = 100;
const MAX_EVENTS_SNAPSHOT: i64 = 100;
const ARCHIVE_DIRECTORY: &str = "evidence-archives";
const REVIEWED_EVIDENCE_TABLES: &[&str] = &[
    "service_events",
    "wrapper_events",
    "wrapper_grant_events",
    "wrapper_authorization_receipts",
    "wrapper_job_events",
    "wrapper_job_execution_receipts",
    "agent_action_receipts",
    "agent_lifecycle_events",
    "private_knowledge_access_receipts",
    "agent_runtime_receipts",
    "agent_runtime_events",
    "agent_runtime_audit_records",
    "agent_supervised_action_receipts",
    "agent_supervised_compensation_receipts",
    "agent_supervised_action_events",
    "agent_schedule_event_inbox",
    "agent_schedule_receipts",
    "agent_schedule_audit_events",
    "model_provider_usage_receipts",
    "model_inference_receipts",
    "model_inference_events",
];
const PACKAGE_CONTENT_TYPE: &str = "application/vnd.microgifter.homeserver-evidence-archive";

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;
type ResponseResult = Result<Response, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceArchivePolicySummary {
    pub policy_id: String,
    pub policy_revision: u64,
    pub enabled: bool,
    pub interval_hours: u32,
    pub max_records_per_archive: u32,
    pub retention_count: u32,
    pub max_package_bytes: u64,
    pub policy_hash: String,
    pub created_by_user_id: String,
    pub reason: String,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceArchiveSummary {
    pub archive_id: String,
    pub archive_sequence: u64,
    pub state: String,
    pub storage_state: String,
    pub previous_archive_hash: String,
    pub record_count: u64,
    pub table_count: u64,
    pub first_record_at_utc: Option<String>,
    pub last_record_at_utc: Option<String>,
    pub chain_root_hash: Option<String>,
    pub manifest_sha256: Option<String>,
    pub package_sha256: Option<String>,
    pub package_size_bytes: Option<u64>,
    pub file_name: String,
    pub created_by_type: String,
    pub created_by_id: String,
    pub failure_code: Option<String>,
    pub created_at_utc: String,
    pub completed_at_utc: Option<String>,
    pub verified_at_utc: Option<String>,
    pub export_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceArchiveEventSummary {
    pub event_id: String,
    pub archive_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceArchiveSnapshot {
    pub schema: String,
    pub policy: EvidenceArchivePolicySummary,
    pub archives: Vec<EvidenceArchiveSummary>,
    pub events: Vec<EvidenceArchiveEventSummary>,
    pub eligible_table_count: u64,
    pub unarchived_record_count: u64,
    pub next_archive_due_at_utc: Option<String>,
    pub source_evidence_deleted: bool,
    pub private_content_exposed: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateEvidenceArchivePolicyRequest {
    pub enabled: bool,
    pub interval_hours: u32,
    pub max_records_per_archive: u32,
    pub retention_count: u32,
    pub max_package_bytes: u64,
    pub created_by_user_id: String,
    pub reason: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateEvidenceArchiveRequest {
    pub idempotency_key: Option<String>,
    pub actor_user_id: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvidenceArchiveReferenceRequest {
    pub archive_id: String,
    pub actor_user_id: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordEvidenceArchiveExportRequest {
    pub archive_id: String,
    pub package_sha256: String,
    pub destination_file_name: String,
    pub actor_user_id: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceArchiveActionResult {
    pub ok: bool,
    pub message: String,
    pub archive: Option<EvidenceArchiveSummary>,
    pub snapshot: EvidenceArchiveSnapshot,
}

#[derive(Debug, Clone)]
pub struct EvidenceArchivePackage {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
    pub package_sha256: String,
}

#[derive(Debug, Clone)]
struct PolicyRecord {
    policy_id: String,
    policy_revision: u64,
    enabled: bool,
    interval_hours: u32,
    max_records_per_archive: u32,
    retention_count: u32,
    max_package_bytes: u64,
    policy_hash: String,
    created_by_user_id: String,
    reason: String,
    created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchivePackageHeader {
    format_version: u32,
    archive_id: String,
    archive_sequence: u64,
    created_at_utc: String,
    encryption: String,
    nonce_base64: String,
    manifest_sha256: String,
    compressed_payload_sha256: String,
    previous_archive_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchiveManifest {
    schema: String,
    format_version: u32,
    archive_id: String,
    archive_sequence: u64,
    policy_id: String,
    policy_revision: u64,
    policy_hash: String,
    installation_id_hash: String,
    application_version: String,
    created_at_utc: String,
    previous_archive_id: Option<String>,
    previous_archive_hash: String,
    record_count: u64,
    table_count: u64,
    table_counts: BTreeMap<String, u64>,
    first_record_at_utc: Option<String>,
    last_record_at_utc: Option<String>,
    records_sha256: String,
    chain_root_hash: String,
    source_evidence_deleted: bool,
    private_content_included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArchiveRecord {
    schema: String,
    ordinal: u64,
    source_table: String,
    source_key: String,
    source_created_at_utc: Option<String>,
    record_sha256: String,
    chain_hash: String,
    fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
struct CollectedRecord {
    source_table: String,
    source_key: String,
    source_created_at_utc: Option<String>,
    record_sha256: String,
    chain_hash: String,
    fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
struct TableShape {
    table_name: String,
    primary_key: String,
    timestamp_column: Option<String>,
    columns: Vec<String>,
}

#[derive(Debug)]
struct VerifiedPackage {
    manifest: ArchiveManifest,
    package_sha256: String,
    size_bytes: u64,
}

pub fn initialize(connection: &Connection, config: &AppConfig) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    fs::create_dir_all(archive_directory(config))?;
    recover_interrupted_archives(connection, config)?;
    health_check(connection, config)
}

pub fn health_check(connection: &Connection, config: &AppConfig) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "Phase 21 migration is not registered exactly once"
    );

    let incomplete_terminal: i64 = connection.query_row(
        "SELECT COUNT(*) FROM evidence_archives WHERE state='verified' AND (manifest_sha256 IS NULL OR package_sha256 IS NULL OR chain_root_hash IS NULL OR completed_at_utc IS NULL OR verified_at_utc IS NULL)",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        incomplete_terminal == 0,
        "verified evidence archive metadata is incomplete"
    );

    let duplicate_members: i64 = connection.query_row(
        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        duplicate_members == 0,
        "evidence archive source membership is ambiguous"
    );

    let policy = latest_policy(connection)?;
    ensure!(
        hash_policy(&policy)? == policy.policy_hash,
        "evidence archive policy hash is invalid"
    );
    verify_archive_chain(connection)?;

    let latest = connection
        .query_row(
            "SELECT storage_path,package_sha256 FROM evidence_archives a JOIN evidence_archive_storage s ON s.archive_id=a.archive_id WHERE a.state='verified' AND s.state IN ('present','exported') ORDER BY a.archive_sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    if let Some((path, expected_hash)) = latest {
        let path = canonical_managed_archive_path(config, Path::new(&path))?;
        ensure!(
            path.is_file(),
            "latest retained evidence archive package is missing"
        );
        ensure!(
            sha256_file(&path)? == expected_hash,
            "latest evidence archive package hash is invalid"
        );
    }
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/evidence-archives", get(snapshot_handler))
        .route(
            "/v1/evidence-archives/policies",
            post(update_policy_handler),
        )
        .route("/v1/evidence-archives/create", post(create_archive_handler))
        .route("/v1/evidence-archives/verify", post(verify_archive_handler))
        .route("/v1/evidence-archives/exports", post(record_export_handler))
        .route(
            "/v1/evidence-archives/{archive_id}/package",
            get(export_package_handler),
        )
        .with_state(state)
}

pub fn snapshot(state: &AppState) -> Result<EvidenceArchiveSnapshot> {
    let connection = state.connection()?;
    snapshot_with_connection(&connection)
}

pub fn create_automatic_if_due(
    state: Arc<AppState>,
) -> Result<Option<EvidenceArchiveActionResult>> {
    let due = {
        let connection = state.connection()?;
        let policy = latest_policy(&connection)?;
        if !policy.enabled {
            return Ok(None);
        }
        let last_verified = connection
            .query_row(
                "SELECT verified_at_utc FROM evidence_archives WHERE state='verified' ORDER BY archive_sequence DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        last_verified
            .map(|value| {
                parse_utc(&value).map(|last| {
                    Utc::now() - last >= Duration::hours(i64::from(policy.interval_hours))
                })
            })
            .transpose()?
            .unwrap_or(true)
    };
    if !due {
        return Ok(None);
    }
    let hour = Utc::now().format("%Y%m%dT%H").to_string();
    create_archive_internal(&state, format!("automatic:{hour}"), "system", "system").map(Some)
}

pub fn package_for_export(state: &AppState, archive_id: &str) -> Result<EvidenceArchivePackage> {
    let archive_id = validate_uuid(archive_id, "archive ID")?;
    let connection = state.connection()?;
    let package = connection
        .query_row(
            "SELECT a.storage_path,a.file_name,a.package_size_bytes,a.package_sha256,s.state FROM evidence_archives a JOIN evidence_archive_storage s ON s.archive_id=a.archive_id WHERE a.archive_id=?1 AND a.state='verified'",
            params![archive_id],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            )),
        )
        .optional()?
        .context("verified evidence archive was not found")?;
    ensure!(
        package.4 == "present" || package.4 == "exported",
        "evidence archive package is not retained locally"
    );
    let path = canonical_managed_archive_path(&state.config, Path::new(&package.0))?;
    ensure!(path.is_file(), "evidence archive package is missing");
    let metadata = fs::metadata(&path)?;
    ensure!(
        metadata.len() == package.2.max(0) as u64,
        "evidence archive package size changed"
    );
    ensure!(
        sha256_file(&path)? == package.3,
        "evidence archive package hash changed"
    );
    Ok(EvidenceArchivePackage {
        path,
        file_name: safe_archive_file_name(&package.1),
        size_bytes: metadata.len(),
        package_sha256: package.3,
    })
}

async fn snapshot_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<EvidenceArchiveSnapshot> {
    tokio::task::spawn_blocking(move || snapshot(&state))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("evidence_archive_snapshot_failed", error))
}

async fn update_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdateEvidenceArchivePolicyRequest>,
) -> ApiResult<EvidenceArchiveActionResult> {
    tokio::task::spawn_blocking(move || update_policy(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("evidence_archive_policy_failed", error))
}

async fn create_archive_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateEvidenceArchiveRequest>,
) -> ApiResult<EvidenceArchiveActionResult> {
    tokio::task::spawn_blocking(move || create_archive(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("evidence_archive_creation_failed", error))
}

async fn verify_archive_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EvidenceArchiveReferenceRequest>,
) -> ApiResult<EvidenceArchiveActionResult> {
    tokio::task::spawn_blocking(move || verify_archive(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("evidence_archive_verification_failed", error))
}

async fn record_export_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RecordEvidenceArchiveExportRequest>,
) -> ApiResult<EvidenceArchiveActionResult> {
    tokio::task::spawn_blocking(move || record_export(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("evidence_archive_export_receipt_failed", error))
}

async fn export_package_handler(
    State(state): State<Arc<AppState>>,
    AxumPath(archive_id): AxumPath<String>,
) -> ResponseResult {
    let package = tokio::task::spawn_blocking(move || package_for_export(&state, &archive_id))
        .await
        .map_err(task_error)?
        .map_err(|error| action_error("evidence_archive_export_failed", error))?;
    let file = tokio::fs::File::open(&package.path)
        .await
        .map_err(|error| internal_error("evidence_archive_export_open_failed", error.into()))?;
    let content_disposition = format!("attachment; filename=\"{}\"", package.file_name);
    let mut response = Response::new(Body::from_stream(ReaderStream::new(file)));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PACKAGE_CONTENT_TYPE),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition).map_err(|error| {
            internal_error("evidence_archive_export_header_failed", error.into())
        })?,
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&package.size_bytes.to_string()).map_err(|error| {
            internal_error("evidence_archive_export_header_failed", error.into())
        })?,
    );
    Ok(response)
}

fn update_policy(
    state: &AppState,
    request: UpdateEvidenceArchivePolicyRequest,
) -> Result<EvidenceArchiveActionResult> {
    ensure!(
        request.confirmation == "UPDATE EVIDENCE ARCHIVE POLICY",
        "type UPDATE EVIDENCE ARCHIVE POLICY to continue"
    );
    ensure!(
        (1..=720).contains(&request.interval_hours),
        "archive interval is invalid"
    );
    ensure!(
        (100..=50_000).contains(&request.max_records_per_archive),
        "archive record limit is invalid"
    );
    ensure!(
        (1..=365).contains(&request.retention_count),
        "archive retention count is invalid"
    );
    ensure!(
        (1_048_576..=MAX_PACKAGE_HARD_BYTES).contains(&request.max_package_bytes),
        "archive package limit is invalid"
    );
    let actor = bounded_text(&request.created_by_user_id, 1, 160, "policy actor")?;
    let reason = bounded_text(&request.reason, 1, 500, "policy reason")?;
    let connection = state.connection()?;
    let previous_revision: i64 = connection.query_row(
        "SELECT COALESCE(MAX(policy_revision),0) FROM evidence_archive_policies",
        [],
        |row| row.get(0),
    )?;
    let revision = previous_revision.max(0) as u64 + 1;
    let policy_record = PolicyRecord {
        policy_id: String::new(),
        policy_revision: revision,
        enabled: request.enabled,
        interval_hours: request.interval_hours,
        max_records_per_archive: request.max_records_per_archive,
        retention_count: request.retention_count,
        max_package_bytes: request.max_package_bytes,
        policy_hash: String::new(),
        created_by_user_id: actor.clone(),
        reason: reason.clone(),
        created_at_utc: String::new(),
    };
    let policy_hash = hash_policy(&policy_record)?;
    let policy_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO evidence_archive_policies (policy_id,policy_revision,enabled,interval_hours,max_records_per_archive,retention_count,max_package_bytes,policy_hash,created_by_user_id,reason,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            policy_id,
            revision as i64,
            i64::from(request.enabled),
            request.interval_hours as i64,
            request.max_records_per_archive as i64,
            request.retention_count as i64,
            request.max_package_bytes as i64,
            policy_hash,
            actor,
            reason,
            now
        ],
    )?;
    record_event_tx(
        &transaction,
        None,
        Some(&policy_id),
        "evidence.archive_policy_created",
        "success",
        "local_user",
        &actor,
        "policy_revision_created",
        json!({"policy_revision": revision,"policy_hash": policy_hash}),
    )?;
    transaction.commit()?;
    Ok(EvidenceArchiveActionResult {
        ok: true,
        message: "Evidence archive policy revision created.".to_owned(),
        archive: None,
        snapshot: snapshot_with_connection(&connection)?,
    })
}

fn create_archive(
    state: &AppState,
    request: CreateEvidenceArchiveRequest,
) -> Result<EvidenceArchiveActionResult> {
    ensure!(
        request.confirmation == "CREATE EVIDENCE ARCHIVE",
        "type CREATE EVIDENCE ARCHIVE to continue"
    );
    let actor = bounded_text(&request.actor_user_id, 1, 160, "archive actor")?;
    let idempotency = request
        .idempotency_key
        .as_deref()
        .map(|value| bounded_text(value, 8, 160, "archive idempotency key"))
        .transpose()?
        .unwrap_or_else(|| format!("manual:{}", Uuid::new_v4()));
    create_archive_internal(state, idempotency, "local_user", &actor)
}

fn create_archive_internal(
    state: &AppState,
    idempotency_key: String,
    actor_type: &str,
    actor_id: &str,
) -> Result<EvidenceArchiveActionResult> {
    ensure!(
        matches!(actor_type, "local_user" | "system"),
        "archive actor type is invalid"
    );
    let connection = state.connection()?;
    if let Some(archive_id) = connection
        .query_row(
            "SELECT archive_id FROM evidence_archives WHERE idempotency_key=?1",
            params![idempotency_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(EvidenceArchiveActionResult {
            ok: true,
            message: "The idempotent evidence archive request already exists.".to_owned(),
            archive: Some(archive_summary(&connection, &archive_id)?),
            snapshot: snapshot_with_connection(&connection)?,
        });
    }
    let policy = latest_policy(&connection)?;
    let previous = latest_verified_archive(&connection)?;
    let previous_archive_id = previous.as_ref().map(|value| value.0.clone());
    let previous_archive_hash = previous
        .as_ref()
        .map(|value| value.1.clone())
        .unwrap_or_else(|| ZERO_HASH.to_owned());
    let archive_sequence: i64 = connection.query_row(
        "SELECT COALESCE(MAX(archive_sequence),0)+1 FROM evidence_archives",
        [],
        |row| row.get(0),
    )?;
    let collected = collect_evidence(
        &connection,
        policy.max_records_per_archive.min(MAX_RECORDS_HARD as u32) as usize,
        &previous_archive_hash,
    )?;
    if collected.is_empty() {
        return Ok(EvidenceArchiveActionResult {
            ok: true,
            message: "No new reviewed evidence is available to archive.".to_owned(),
            archive: None,
            snapshot: snapshot_with_connection(&connection)?,
        });
    }

    let archive_id = Uuid::new_v4().to_string();
    let created_at = now_utc();
    let file_name = format!(
        "Microgifter-HomeServer-Evidence-{}-{}.mgha",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        &archive_id[..8]
    );
    let directory = archive_directory(&state.config);
    fs::create_dir_all(&directory)?;
    let final_path = directory.join(&file_name);
    let temporary_path = final_path.with_extension("mgha.tmp");
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO evidence_archives (archive_id,idempotency_key,policy_id,policy_revision,state,previous_archive_id,previous_archive_hash,archive_sequence,file_name,storage_path,created_by_type,created_by_id,created_at_utc) VALUES (?1,?2,?3,?4,'collecting',?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            archive_id,
            idempotency_key,
            policy.policy_id,
            policy.policy_revision as i64,
            previous_archive_id,
            previous_archive_hash,
            archive_sequence,
            file_name,
            final_path.to_string_lossy(),
            actor_type,
            actor_id,
            created_at
        ],
    )?;
    transaction.execute(
        "INSERT INTO evidence_archive_storage (archive_id,state,updated_at_utc) VALUES (?1,'creating',?2)",
        params![archive_id,created_at],
    )?;
    transaction.commit()?;

    let package_result = build_package(
        &connection,
        &state.config,
        &archive_id,
        archive_sequence.max(1) as u64,
        &policy,
        previous_archive_id.as_deref(),
        &previous_archive_hash,
        &collected,
        &created_at,
        policy.max_package_bytes,
    )
    .and_then(|built| {
        write_atomic(&temporary_path, &final_path, &built.0)?;
        let verified =
            verify_package_file(&connection, &state.config, &final_path, Some(&archive_id))?;
        ensure!(
            verified.package_sha256 == built.1,
            "evidence archive package hash changed after write"
        );
        ensure!(
            verified.size_bytes == built.0.len() as u64,
            "evidence archive package size changed after write"
        );
        Ok((built, verified))
    });

    let ((package_bytes, package_sha256, manifest, manifest_sha256), verified) =
        match package_result {
            Ok(value) => value,
            Err(error) => {
                let _ = fs::remove_file(&temporary_path);
                let _ = fs::remove_file(&final_path);
                let failure = bounded_failure_code(&error.to_string());
                mark_archive_failed(
                    &connection,
                    &archive_id,
                    &policy.policy_id,
                    actor_type,
                    actor_id,
                    &failure,
                )?;
                return Err(error);
            }
        };

    ensure!(
        verified.manifest.archive_id == archive_id,
        "verified evidence archive identity changed"
    );
    let completed_at = now_utc();
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE evidence_archives SET state='verified',record_count=?1,table_count=?2,first_record_at_utc=?3,last_record_at_utc=?4,records_sha256=?5,chain_root_hash=?6,manifest_sha256=?7,package_sha256=?8,package_size_bytes=?9,completed_at_utc=?10,verified_at_utc=?10 WHERE archive_id=?11 AND state='collecting'",
        params![
            manifest.record_count as i64,
            manifest.table_count as i64,
            manifest.first_record_at_utc,
            manifest.last_record_at_utc,
            manifest.records_sha256,
            manifest.chain_root_hash,
            manifest_sha256,
            package_sha256,
            package_bytes.len() as i64,
            completed_at,
            archive_id
        ],
    )?;
    ensure!(changed == 1, "collecting evidence archive was not found");
    for (index, record) in collected.iter().enumerate() {
        transaction.execute(
            "INSERT INTO evidence_archive_members (member_id,archive_id,ordinal,source_table,source_key,source_created_at_utc,record_sha256,chain_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                Uuid::new_v4().to_string(),
                archive_id,
                (index + 1) as i64,
                record.source_table,
                record.source_key,
                record.source_created_at_utc,
                record.record_sha256,
                record.chain_hash,
                completed_at
            ],
        )?;
    }
    transaction.execute(
        "UPDATE evidence_archive_storage SET state='present',last_verified_at_utc=?1,updated_at_utc=?1 WHERE archive_id=?2 AND state='creating'",
        params![completed_at,archive_id],
    )?;
    record_event_tx(
        &transaction,
        Some(&archive_id),
        Some(&policy.policy_id),
        "evidence.archive_verified",
        "success",
        actor_type,
        actor_id,
        "archive_created_and_verified",
        json!({
            "archive_sequence": manifest.archive_sequence,
            "record_count": manifest.record_count,
            "table_count": manifest.table_count,
            "manifest_sha256": manifest_sha256,
            "package_sha256": package_sha256,
            "chain_root_hash": manifest.chain_root_hash,
            "private_content_included": false,
            "source_evidence_deleted": false
        }),
    )?;
    transaction.commit()?;
    enforce_retention(&connection, &state.config, &policy, actor_type, actor_id)?;
    Ok(EvidenceArchiveActionResult {
        ok: true,
        message: "Tamper-evident evidence archive created and verified.".to_owned(),
        archive: Some(archive_summary(&connection, &archive_id)?),
        snapshot: snapshot_with_connection(&connection)?,
    })
}

fn verify_archive(
    state: &AppState,
    request: EvidenceArchiveReferenceRequest,
) -> Result<EvidenceArchiveActionResult> {
    let archive_id = validate_uuid(&request.archive_id, "archive ID")?;
    ensure!(
        request.confirmation == format!("VERIFY EVIDENCE ARCHIVE {archive_id}"),
        "archive verification confirmation is invalid"
    );
    let actor = bounded_text(&request.actor_user_id, 1, 160, "verification actor")?;
    let package = package_for_export(state, &archive_id)?;
    let connection = state.connection()?;
    let verified =
        verify_package_file(&connection, &state.config, &package.path, Some(&archive_id))?;
    ensure!(
        verified.size_bytes == package.size_bytes,
        "evidence archive package size is not the recorded size"
    );
    ensure!(
        verified.package_sha256 == package.package_sha256,
        "evidence archive package hash is not the recorded hash"
    );
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE evidence_archive_storage SET last_verified_at_utc=?1,updated_at_utc=?1 WHERE archive_id=?2 AND state IN ('present','exported')",
        params![now,archive_id],
    )?;
    record_event_tx(
        &transaction,
        Some(&archive_id),
        None,
        "evidence.archive_verified_again",
        "success",
        "local_user",
        &actor,
        "package_chain_verified",
        json!({
            "manifest_sha256": hash_json(&verified.manifest)?,
            "package_sha256": verified.package_sha256,
            "record_count": verified.manifest.record_count
        }),
    )?;
    transaction.commit()?;
    Ok(EvidenceArchiveActionResult {
        ok: true,
        message: "Evidence archive package and hash chain verified.".to_owned(),
        archive: Some(archive_summary(&connection, &archive_id)?),
        snapshot: snapshot_with_connection(&connection)?,
    })
}

fn record_export(
    state: &AppState,
    request: RecordEvidenceArchiveExportRequest,
) -> Result<EvidenceArchiveActionResult> {
    let archive_id = validate_uuid(&request.archive_id, "archive ID")?;
    ensure!(
        request.confirmation == format!("EXPORT EVIDENCE ARCHIVE {archive_id}"),
        "archive export confirmation is invalid"
    );
    let actor = bounded_text(&request.actor_user_id, 1, 160, "export actor")?;
    let destination = safe_archive_file_name(&request.destination_file_name);
    let package_hash = validate_hash(&request.package_sha256, "package hash")?;
    let connection = state.connection()?;
    let recorded_hash: String = connection
        .query_row(
            "SELECT package_sha256 FROM evidence_archives WHERE archive_id=?1 AND state='verified'",
            params![archive_id],
            |row| row.get(0),
        )
        .optional()?
        .context("verified evidence archive was not found")?;
    ensure!(
        recorded_hash == package_hash,
        "exported package hash does not match the verified archive"
    );
    let export_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let receipt_hash = hash_json(&json!({
        "schema": "homeserver.evidence-archive-export-receipt.v1",
        "export_id": &export_id,
        "archive_id": &archive_id,
        "package_sha256": &package_hash,
        "destination_file_name": &destination,
        "exported_by_user_id": &actor,
        "created_at_utc": &now
    }))?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO evidence_archive_exports (export_id,archive_id,package_sha256,destination_file_name,exported_by_user_id,export_receipt_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![export_id,archive_id,package_hash,destination,actor,receipt_hash,now],
    )?;
    transaction.execute(
        "UPDATE evidence_archive_storage SET state='exported',exported_at_utc=?1,updated_at_utc=?1 WHERE archive_id=?2 AND state='present'",
        params![now,archive_id],
    )?;
    record_event_tx(
        &transaction,
        Some(&archive_id),
        None,
        "evidence.archive_exported",
        "success",
        "local_user",
        &actor,
        "verified_package_exported",
        json!({"destination_file_name": destination,"package_sha256": package_hash,"export_receipt_hash": receipt_hash}),
    )?;
    transaction.commit()?;
    let policy = latest_policy(&connection)?;
    enforce_retention(
        &connection,
        &state.config,
        &policy,
        "system",
        "evidence_archive_retention",
    )?;
    Ok(EvidenceArchiveActionResult {
        ok: true,
        message: "Evidence archive export receipt recorded.".to_owned(),
        archive: Some(archive_summary(&connection, &archive_id)?),
        snapshot: snapshot_with_connection(&connection)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_package(
    connection: &Connection,
    config: &AppConfig,
    archive_id: &str,
    archive_sequence: u64,
    policy: &PolicyRecord,
    previous_archive_id: Option<&str>,
    previous_archive_hash: &str,
    records: &[CollectedRecord],
    created_at: &str,
    max_package_bytes: u64,
) -> Result<(Vec<u8>, String, ArchiveManifest, String)> {
    let records_bytes = records_ndjson(records)?;
    let records_sha256 = sha256_bytes(&records_bytes);
    let chain_root_hash = records
        .last()
        .map(|record| record.chain_hash.clone())
        .context("evidence archive requires at least one record")?;
    let mut table_counts = BTreeMap::new();
    let mut first_record_at = None::<String>;
    let mut last_record_at = None::<String>;
    for record in records {
        *table_counts
            .entry(record.source_table.clone())
            .or_insert(0_u64) += 1;
        if let Some(value) = &record.source_created_at_utc {
            if first_record_at
                .as_ref()
                .map_or(true, |current| value < current)
            {
                first_record_at = Some(value.clone());
            }
            if last_record_at
                .as_ref()
                .map_or(true, |current| value > current)
            {
                last_record_at = Some(value.clone());
            }
        }
    }
    let installation_id = database::installation_id(connection)?;
    let manifest = ArchiveManifest {
        schema: "homeserver.evidence-archive-manifest.v1".to_owned(),
        format_version: PACKAGE_VERSION,
        archive_id: archive_id.to_owned(),
        archive_sequence,
        policy_id: policy.policy_id.clone(),
        policy_revision: policy.policy_revision,
        policy_hash: policy.policy_hash.clone(),
        installation_id_hash: hash_text(&installation_id),
        application_version: env!("CARGO_PKG_VERSION").to_owned(),
        created_at_utc: created_at.to_owned(),
        previous_archive_id: previous_archive_id.map(str::to_owned),
        previous_archive_hash: previous_archive_hash.to_owned(),
        record_count: records.len() as u64,
        table_count: table_counts.len() as u64,
        table_counts,
        first_record_at_utc: first_record_at,
        last_record_at_utc: last_record_at,
        records_sha256,
        chain_root_hash,
        source_evidence_deleted: false,
        private_content_included: false,
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let manifest_sha256 = sha256_bytes(&manifest_bytes);
    let compressed = create_tar_gz(&manifest_bytes, &records_bytes)?;
    ensure!(
        compressed.len() as u64 <= max_package_bytes,
        "evidence archive compressed payload exceeds the policy limit"
    );
    let compressed_payload_sha256 = sha256_bytes(&compressed);
    let key = Zeroizing::new(archive_key(config, connection)?);
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("unable to initialize evidence archive encryption"))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), compressed.as_ref())
        .map_err(|_| anyhow::anyhow!("unable to encrypt evidence archive"))?;
    let header = ArchivePackageHeader {
        format_version: PACKAGE_VERSION,
        archive_id: archive_id.to_owned(),
        archive_sequence,
        created_at_utc: created_at.to_owned(),
        encryption: "device_key_aes256gcm".to_owned(),
        nonce_base64: URL_SAFE_NO_PAD.encode(nonce),
        manifest_sha256: manifest_sha256.clone(),
        compressed_payload_sha256,
        previous_archive_hash: previous_archive_hash.to_owned(),
    };
    let header_bytes = serde_json::to_vec(&header)?;
    ensure!(
        header_bytes.len() <= MAX_HEADER_BYTES,
        "evidence archive header is too large"
    );
    let mut package = Vec::with_capacity(12 + header_bytes.len() + ciphertext.len());
    package.extend_from_slice(PACKAGE_MAGIC);
    package.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
    package.extend_from_slice(&header_bytes);
    package.extend_from_slice(&ciphertext);
    ensure!(
        package.len() as u64 <= max_package_bytes,
        "evidence archive package exceeds the policy limit"
    );
    ensure!(
        package.len() as u64 <= MAX_PACKAGE_HARD_BYTES,
        "evidence archive package exceeds the hard limit"
    );
    let package_sha256 = sha256_bytes(&package);
    Ok((package, package_sha256, manifest, manifest_sha256))
}

fn verify_package_file(
    connection: &Connection,
    config: &AppConfig,
    path: &Path,
    expected_archive_id: Option<&str>,
) -> Result<VerifiedPackage> {
    let metadata = fs::metadata(path)?;
    ensure!(
        metadata.len() > 12 && metadata.len() <= MAX_PACKAGE_HARD_BYTES,
        "evidence archive package size is invalid"
    );
    let package = fs::read(path)?;
    let package_sha256 = sha256_bytes(&package);
    ensure!(
        &package[..8] == PACKAGE_MAGIC,
        "evidence archive package magic is invalid"
    );
    let header_length = u32::from_be_bytes(package[8..12].try_into()?) as usize;
    ensure!(
        header_length > 0 && header_length <= MAX_HEADER_BYTES,
        "evidence archive header length is invalid"
    );
    let header_end = 12_usize
        .checked_add(header_length)
        .context("evidence archive header size overflow")?;
    ensure!(
        header_end < package.len(),
        "evidence archive package is truncated"
    );
    let header: ArchivePackageHeader = serde_json::from_slice(&package[12..header_end])?;
    ensure!(
        header.format_version == PACKAGE_VERSION,
        "evidence archive package version is unsupported"
    );
    ensure!(
        header.encryption == "device_key_aes256gcm",
        "evidence archive encryption mode is unsupported"
    );
    if let Some(expected) = expected_archive_id {
        ensure!(
            header.archive_id == expected,
            "evidence archive package identity is invalid"
        );
    }
    let nonce = URL_SAFE_NO_PAD.decode(&header.nonce_base64)?;
    ensure!(nonce.len() == 12, "evidence archive nonce is invalid");
    let key = Zeroizing::new(archive_key(config, connection)?);
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("unable to initialize evidence archive decryption"))?;
    let compressed = cipher
        .decrypt(Nonce::from_slice(&nonce), &package[header_end..])
        .map_err(|_| anyhow::anyhow!("evidence archive authentication failed"))?;
    ensure!(
        sha256_bytes(&compressed) == header.compressed_payload_sha256,
        "evidence archive compressed payload hash is invalid"
    );
    let (manifest_bytes, records_bytes) = extract_tar_gz(&compressed)?;
    ensure!(
        sha256_bytes(&manifest_bytes) == header.manifest_sha256,
        "evidence archive manifest hash is invalid"
    );
    let manifest: ArchiveManifest = serde_json::from_slice(&manifest_bytes)?;
    ensure!(
        manifest.schema == "homeserver.evidence-archive-manifest.v1",
        "evidence archive manifest schema is invalid"
    );
    ensure!(
        manifest.archive_id == header.archive_id,
        "evidence archive manifest identity is invalid"
    );
    ensure!(
        manifest.archive_sequence == header.archive_sequence,
        "evidence archive sequence is invalid"
    );
    ensure!(
        manifest.previous_archive_hash == header.previous_archive_hash,
        "evidence archive predecessor hash is invalid"
    );
    ensure!(
        !manifest.private_content_included && !manifest.source_evidence_deleted,
        "evidence archive privacy boundary is invalid"
    );
    ensure!(
        sha256_bytes(&records_bytes) == manifest.records_sha256,
        "evidence archive records hash is invalid"
    );
    let parsed = parse_records(&records_bytes, &manifest.previous_archive_hash)?;
    ensure!(
        parsed.0 == manifest.record_count,
        "evidence archive record count is invalid"
    );
    ensure!(
        parsed.1 == manifest.chain_root_hash,
        "evidence archive chain root is invalid"
    );
    ensure!(
        parsed.2 == manifest.table_counts,
        "evidence archive table counts are invalid"
    );
    Ok(VerifiedPackage {
        manifest,
        package_sha256,
        size_bytes: metadata.len(),
    })
}

fn collect_evidence(
    connection: &Connection,
    limit: usize,
    previous_archive_hash: &str,
) -> Result<Vec<CollectedRecord>> {
    ensure!(
        limit > 0 && limit <= MAX_RECORDS_HARD,
        "evidence archive record limit is invalid"
    );
    let shapes = eligible_table_shapes(connection)?;
    let mut collected = Vec::new();
    let mut chain_hash = previous_archive_hash.to_owned();
    for shape in shapes {
        if collected.len() >= limit {
            break;
        }
        let remaining = limit - collected.len();
        let records = collect_table(connection, &shape, remaining)?;
        for mut record in records {
            chain_hash = hash_chain(&chain_hash, &record.record_sha256);
            record.chain_hash = chain_hash.clone();
            collected.push(record);
        }
    }
    Ok(collected)
}

fn eligible_table_shapes(connection: &Connection) -> Result<Vec<TableShape>> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let table_names = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut shapes = Vec::new();
    for table_name in table_names {
        if !is_allowed_evidence_table(&table_name) {
            continue;
        }
        let columns = table_columns(connection, &table_name)?;
        let primary_keys = columns
            .iter()
            .filter(|column| column.2 > 0)
            .collect::<Vec<_>>();
        if primary_keys.len() != 1 {
            continue;
        }
        let names = columns
            .iter()
            .map(|column| column.0.clone())
            .collect::<Vec<_>>();
        let timestamp_column = [
            "created_at_utc",
            "completed_at_utc",
            "occurred_at_utc",
            "recorded_at_utc",
            "updated_at_utc",
        ]
        .iter()
        .find(|candidate| names.iter().any(|name| name == **candidate))
        .map(|value| (*value).to_owned());
        shapes.push(TableShape {
            table_name,
            primary_key: primary_keys[0].0.clone(),
            timestamp_column,
            columns: names,
        });
    }
    Ok(shapes)
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<(String, String, i64)>> {
    let table = quote_identifier(table)?;
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let columns = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns)
}

fn collect_table(
    connection: &Connection,
    shape: &TableShape,
    limit: usize,
) -> Result<Vec<CollectedRecord>> {
    let table = quote_identifier(&shape.table_name)?;
    let primary = quote_identifier(&shape.primary_key)?;
    let order = if let Some(timestamp) = &shape.timestamp_column {
        format!("{},CAST({primary} AS TEXT)", quote_identifier(timestamp)?)
    } else {
        format!("CAST({primary} AS TEXT)")
    };
    let sql = format!(
        "SELECT * FROM {table} AS source WHERE NOT EXISTS (SELECT 1 FROM evidence_archive_members AS member WHERE member.source_table=?1 AND member.source_key=CAST(source.{primary} AS TEXT)) ORDER BY {order} LIMIT ?2"
    );
    let mut statement = connection.prepare(&sql)?;
    let column_names = statement
        .column_names()
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    ensure!(
        column_names == shape.columns,
        "evidence table column order changed during collection"
    );
    let mut rows = statement.query(params![shape.table_name, limit as i64])?;
    let mut records = Vec::new();
    while let Some(row) = rows.next()? {
        let mut fields = BTreeMap::new();
        for (index, column) in column_names.iter().enumerate() {
            fields.insert(column.clone(), value_to_json(row.get_ref(index)?));
        }
        let key = fields
            .get(&shape.primary_key)
            .map(canonical_scalar)
            .context("evidence record primary key is unavailable")?;
        ensure!(
            !key.is_empty() && key.chars().count() <= 500,
            "evidence record key is invalid"
        );
        let created_at = shape
            .timestamp_column
            .as_ref()
            .and_then(|column| fields.get(column))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let canonical = serde_json::to_vec(&fields)?;
        ensure!(
            canonical.len() <= MAX_RECORD_BYTES,
            "evidence record exceeds the canonical row size limit"
        );
        let record_hash = hash_bytes_parts(&[
            shape.table_name.as_bytes(),
            b"\n",
            key.as_bytes(),
            b"\n",
            &canonical,
        ]);
        records.push(CollectedRecord {
            source_table: shape.table_name.clone(),
            source_key: key,
            source_created_at_utc: created_at,
            record_sha256: record_hash,
            chain_hash: String::new(),
            fields,
        });
    }
    Ok(records)
}

fn records_ndjson(records: &[CollectedRecord]) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let line = ArchiveRecord {
            schema: "homeserver.evidence-archive-record.v1".to_owned(),
            ordinal: (index + 1) as u64,
            source_table: record.source_table.clone(),
            source_key: record.source_key.clone(),
            source_created_at_utc: record.source_created_at_utc.clone(),
            record_sha256: record.record_sha256.clone(),
            chain_hash: record.chain_hash.clone(),
            fields: record.fields.clone(),
        };
        serde_json::to_writer(&mut output, &line)?;
        output.push(b'\n');
    }
    Ok(output)
}

fn parse_records(
    bytes: &[u8],
    previous_archive_hash: &str,
) -> Result<(u64, String, BTreeMap<String, u64>)> {
    let text = std::str::from_utf8(bytes)?;
    let mut count = 0_u64;
    let mut chain = previous_archive_hash.to_owned();
    let mut tables = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for line in text.lines() {
        ensure!(
            !line.trim().is_empty(),
            "evidence archive contains an empty record line"
        );
        let record: ArchiveRecord = serde_json::from_str(line)?;
        count = count.saturating_add(1);
        ensure!(
            record.ordinal == count,
            "evidence archive record ordinal is invalid"
        );
        ensure!(
            record.schema == "homeserver.evidence-archive-record.v1",
            "evidence archive record schema is invalid"
        );
        ensure!(
            is_allowed_evidence_table(&record.source_table),
            "evidence archive contains a forbidden table"
        );
        ensure!(
            seen.insert((record.source_table.clone(), record.source_key.clone())),
            "evidence archive contains duplicate source membership"
        );
        let canonical = serde_json::to_vec(&record.fields)?;
        let calculated_record_hash = hash_bytes_parts(&[
            record.source_table.as_bytes(),
            b"\n",
            record.source_key.as_bytes(),
            b"\n",
            &canonical,
        ]);
        ensure!(
            calculated_record_hash == record.record_sha256,
            "evidence archive record hash is invalid"
        );
        chain = hash_chain(&chain, &record.record_sha256);
        ensure!(
            chain == record.chain_hash,
            "evidence archive record chain is invalid"
        );
        *tables.entry(record.source_table).or_insert(0) += 1;
    }
    ensure!(count > 0, "evidence archive contains no records");
    Ok((count, chain, tables))
}

fn create_tar_gz(manifest: &[u8], records: &[u8]) -> Result<Vec<u8>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::best());
    let mut builder = Builder::new(encoder);
    append_tar_entry(&mut builder, "manifest.json", manifest)?;
    append_tar_entry(&mut builder, "records.ndjson", records)?;
    let encoder = builder.into_inner()?;
    encoder.finish().map_err(Into::into)
}

fn append_tar_entry<W: Write>(builder: &mut Builder<W>, name: &str, bytes: &[u8]) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, name, Cursor::new(bytes))?;
    Ok(())
}

fn extract_tar_gz(bytes: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    let mut manifest = None;
    let mut records = None;
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        ensure!(
            path == "manifest.json" || path == "records.ndjson",
            "evidence archive contains an unexpected path"
        );
        let limit = if path == "manifest.json" {
            MAX_HEADER_BYTES as u64
        } else {
            MAX_PACKAGE_HARD_BYTES
        };
        ensure!(
            entry.size() <= limit,
            "evidence archive entry exceeds its size limit"
        );
        let mut output = Vec::new();
        entry.take(limit + 1).read_to_end(&mut output)?;
        ensure!(
            output.len() as u64 <= limit,
            "evidence archive entry exceeds its size limit"
        );
        match path.as_str() {
            "manifest.json" => {
                ensure!(
                    manifest.is_none(),
                    "evidence archive contains duplicate manifests"
                );
                manifest = Some(output);
            }
            "records.ndjson" => {
                ensure!(
                    records.is_none(),
                    "evidence archive contains duplicate record streams"
                );
                records = Some(output);
            }
            _ => unreachable!(),
        }
    }
    Ok((
        manifest.context("evidence archive manifest is missing")?,
        records.context("evidence archive record stream is missing")?,
    ))
}

fn snapshot_with_connection(connection: &Connection) -> Result<EvidenceArchiveSnapshot> {
    let policy = latest_policy(connection)?;
    let mut statement = connection.prepare(
        "SELECT a.archive_id,a.archive_sequence,a.state,s.state,a.previous_archive_hash,a.record_count,a.table_count,a.first_record_at_utc,a.last_record_at_utc,a.chain_root_hash,a.manifest_sha256,a.package_sha256,a.package_size_bytes,a.file_name,a.created_by_type,a.created_by_id,a.failure_code,a.created_at_utc,a.completed_at_utc,a.verified_at_utc,(SELECT COUNT(*) FROM evidence_archive_exports x WHERE x.archive_id=a.archive_id) FROM evidence_archives a JOIN evidence_archive_storage s ON s.archive_id=a.archive_id ORDER BY a.archive_sequence DESC LIMIT ?1",
    )?;
    let archives = statement
        .query_map(params![MAX_ARCHIVES_SNAPSHOT], map_archive_summary)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut statement = connection.prepare(
        "SELECT event_id,archive_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc FROM evidence_archive_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1",
    )?;
    let events = statement
        .query_map(params![MAX_EVENTS_SNAPSHOT], |row| {
            let metadata = row
                .get::<_, String>(8)
                .ok()
                .and_then(|value| serde_json::from_str(&value).ok())
                .unwrap_or_else(|| json!({"redacted": true}));
            Ok(EvidenceArchiveEventSummary {
                event_id: row.get(0)?,
                archive_id: row.get(1)?,
                policy_id: row.get(2)?,
                event_type: row.get(3)?,
                outcome: row.get(4)?,
                actor_type: row.get(5)?,
                actor_id: row.get(6)?,
                detail_code: row.get(7)?,
                metadata,
                event_hash: row.get(9)?,
                created_at_utc: row.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let shapes = eligible_table_shapes(connection)?;
    let mut unarchived = 0_u64;
    for shape in &shapes {
        let table = quote_identifier(&shape.table_name)?;
        let primary = quote_identifier(&shape.primary_key)?;
        let sql = format!(
            "SELECT COUNT(*) FROM {table} source WHERE NOT EXISTS (SELECT 1 FROM evidence_archive_members member WHERE member.source_table=?1 AND member.source_key=CAST(source.{primary} AS TEXT))"
        );
        let count: i64 = connection.query_row(&sql, params![shape.table_name], |row| row.get(0))?;
        unarchived = unarchived.saturating_add(count.max(0) as u64);
    }
    let next_due = archives
        .iter()
        .find(|archive| archive.state == "verified")
        .and_then(|archive| archive.verified_at_utc.as_deref())
        .map(parse_utc)
        .transpose()?
        .map(|value| (value + Duration::hours(i64::from(policy.interval_hours))).to_rfc3339());
    Ok(EvidenceArchiveSnapshot {
        schema: "homeserver.evidence-archive-snapshot.v1".to_owned(),
        policy: policy.into(),
        archives,
        events,
        eligible_table_count: shapes.len() as u64,
        unarchived_record_count: unarchived,
        next_archive_due_at_utc: next_due,
        source_evidence_deleted: false,
        private_content_exposed: false,
    })
}

fn archive_summary(connection: &Connection, archive_id: &str) -> Result<EvidenceArchiveSummary> {
    connection
        .query_row(
            "SELECT a.archive_id,a.archive_sequence,a.state,s.state,a.previous_archive_hash,a.record_count,a.table_count,a.first_record_at_utc,a.last_record_at_utc,a.chain_root_hash,a.manifest_sha256,a.package_sha256,a.package_size_bytes,a.file_name,a.created_by_type,a.created_by_id,a.failure_code,a.created_at_utc,a.completed_at_utc,a.verified_at_utc,(SELECT COUNT(*) FROM evidence_archive_exports x WHERE x.archive_id=a.archive_id) FROM evidence_archives a JOIN evidence_archive_storage s ON s.archive_id=a.archive_id WHERE a.archive_id=?1",
            params![archive_id],
            map_archive_summary,
        )
        .optional()?
        .context("evidence archive was not found")
}

fn map_archive_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<EvidenceArchiveSummary> {
    Ok(EvidenceArchiveSummary {
        archive_id: row.get(0)?,
        archive_sequence: nonnegative_u64(row.get(1)?),
        state: row.get(2)?,
        storage_state: row.get(3)?,
        previous_archive_hash: row.get(4)?,
        record_count: nonnegative_u64(row.get(5)?),
        table_count: nonnegative_u64(row.get(6)?),
        first_record_at_utc: row.get(7)?,
        last_record_at_utc: row.get(8)?,
        chain_root_hash: row.get(9)?,
        manifest_sha256: row.get(10)?,
        package_sha256: row.get(11)?,
        package_size_bytes: row.get::<_, Option<i64>>(12)?.map(nonnegative_u64),
        file_name: row.get(13)?,
        created_by_type: row.get(14)?,
        created_by_id: row.get(15)?,
        failure_code: row.get(16)?,
        created_at_utc: row.get(17)?,
        completed_at_utc: row.get(18)?,
        verified_at_utc: row.get(19)?,
        export_count: nonnegative_u64(row.get(20)?),
    })
}

fn latest_policy(connection: &Connection) -> Result<PolicyRecord> {
    connection
        .query_row(
            "SELECT policy_id,policy_revision,enabled,interval_hours,max_records_per_archive,retention_count,max_package_bytes,policy_hash,created_by_user_id,reason,created_at_utc FROM evidence_archive_policies ORDER BY policy_revision DESC LIMIT 1",
            [],
            |row| Ok(PolicyRecord {
                policy_id: row.get(0)?,
                policy_revision: nonnegative_u64(row.get(1)?),
                enabled: row.get::<_, i64>(2)? == 1,
                interval_hours: row.get::<_, i64>(3)?.clamp(1,720) as u32,
                max_records_per_archive: row.get::<_, i64>(4)?.clamp(100,50_000) as u32,
                retention_count: row.get::<_, i64>(5)?.clamp(1,365) as u32,
                max_package_bytes: nonnegative_u64(row.get(6)?),
                policy_hash: row.get(7)?,
                created_by_user_id: row.get(8)?,
                reason: row.get(9)?,
                created_at_utc: row.get(10)?,
            }),
        )
        .context("evidence archive policy is unavailable")
}

fn hash_policy(policy: &PolicyRecord) -> Result<String> {
    hash_json(&json!({
        "schema": "homeserver.evidence-archive-policy.v1",
        "policy_revision": policy.policy_revision,
        "enabled": policy.enabled,
        "interval_hours": policy.interval_hours,
        "max_records_per_archive": policy.max_records_per_archive,
        "retention_count": policy.retention_count,
        "max_package_bytes": policy.max_package_bytes,
        "created_by_user_id": policy.created_by_user_id,
        "reason": policy.reason
    }))
}

fn verify_archive_chain(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT archive_id,archive_sequence,previous_archive_id,previous_archive_hash,manifest_sha256 FROM evidence_archives WHERE state='verified' ORDER BY archive_sequence,archive_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let archives = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut previous_id = None::<String>;
    let mut previous_manifest_hash = ZERO_HASH.to_owned();
    let mut previous_sequence = 0_i64;
    for (archive_id, sequence, recorded_previous_id, recorded_previous_hash, manifest_hash) in
        archives
    {
        ensure!(
            sequence > previous_sequence,
            "evidence archive sequence is not strictly increasing"
        );
        ensure!(
            recorded_previous_id == previous_id,
            "evidence archive predecessor identity is invalid"
        );
        ensure!(
            recorded_previous_hash == previous_manifest_hash,
            "evidence archive predecessor hash is invalid"
        );
        ensure!(
            manifest_hash.len() == 64,
            "evidence archive manifest hash is invalid"
        );
        previous_sequence = sequence;
        previous_id = Some(archive_id);
        previous_manifest_hash = manifest_hash;
    }
    Ok(())
}

fn latest_verified_archive(connection: &Connection) -> Result<Option<(String, String)>> {
    let archive = connection
        .query_row(
            "SELECT archive_id,manifest_sha256 FROM evidence_archives WHERE state='verified' ORDER BY archive_sequence DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?,row.get(1)?)),
        )
        .optional()?;
    Ok(archive)
}

fn enforce_retention(
    connection: &Connection,
    config: &AppConfig,
    policy: &PolicyRecord,
    actor_type: &str,
    actor_id: &str,
) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT a.archive_id,a.storage_path FROM evidence_archives a JOIN evidence_archive_storage s ON s.archive_id=a.archive_id WHERE a.state='verified' AND s.state='exported' AND EXISTS (SELECT 1 FROM evidence_archive_exports x WHERE x.archive_id=a.archive_id) AND a.archive_id NOT IN (SELECT retained.archive_id FROM evidence_archives retained JOIN evidence_archive_storage retained_storage ON retained_storage.archive_id=retained.archive_id WHERE retained.state='verified' AND retained_storage.state IN ('present','exported') ORDER BY retained.archive_sequence DESC LIMIT ?1) ORDER BY a.archive_sequence ASC",
    )?;
    let candidates = statement
        .query_map(params![policy.retention_count as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (archive_id, path) in candidates {
        let path = canonical_managed_archive_path(config, Path::new(&path))?;
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let now = now_utc();
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "UPDATE evidence_archive_storage SET state='pruned',pruned_at_utc=?1,updated_at_utc=?1 WHERE archive_id=?2 AND state='exported'",
            params![now,archive_id],
        )?;
        record_event_tx(
            &transaction,
            Some(&archive_id),
            Some(&policy.policy_id),
            "evidence.archive_local_copy_pruned",
            "success",
            actor_type,
            actor_id,
            "export_verified_before_prune",
            json!({"retention_count": policy.retention_count,"source_evidence_deleted": false}),
        )?;
        transaction.commit()?;
    }
    Ok(())
}

fn recover_interrupted_archives(connection: &Connection, config: &AppConfig) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT archive_id,storage_path,policy_id FROM evidence_archives WHERE state='collecting'",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (archive_id, path, policy_id) in rows {
        if let Ok(path) = canonical_managed_archive_path(config, Path::new(&path)) {
            let _ = fs::remove_file(&path);
            let _ = fs::remove_file(path.with_extension("mgha.tmp"));
        }
        mark_archive_failed(
            connection,
            &archive_id,
            &policy_id,
            "system",
            "restart_recovery",
            "restart_interrupted",
        )?;
    }
    Ok(())
}

fn mark_archive_failed(
    connection: &Connection,
    archive_id: &str,
    policy_id: &str,
    actor_type: &str,
    actor_id: &str,
    failure_code: &str,
) -> Result<()> {
    let now = now_utc();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE evidence_archives SET state='failed',failure_code=?1,completed_at_utc=?2 WHERE archive_id=?3 AND state='collecting'",
        params![failure_code,now,archive_id],
    )?;
    transaction.execute(
        "UPDATE evidence_archive_storage SET state='missing',updated_at_utc=?1 WHERE archive_id=?2 AND state='creating'",
        params![now,archive_id],
    )?;
    record_event_tx(
        &transaction,
        Some(archive_id),
        Some(policy_id),
        "evidence.archive_failed",
        "error",
        actor_type,
        actor_id,
        failure_code,
        json!({"private_content_exposed": false,"source_evidence_deleted": false}),
    )?;
    transaction.commit()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_event_tx(
    transaction: &Transaction<'_>,
    archive_id: Option<&str>,
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
    let metadata_json = serde_json::to_string(&metadata)?;
    let event_hash = hash_json(&json!({
        "schema": "homeserver.evidence-archive-event.v1",
        "event_id": &event_id,
        "archive_id": archive_id,
        "policy_id": policy_id,
        "event_type": event_type,
        "outcome": outcome,
        "actor_type": actor_type,
        "actor_id": actor_id,
        "detail_code": detail_code,
        "metadata": metadata,
        "created_at_utc": &created_at
    }))?;
    transaction.execute(
        "INSERT INTO evidence_archive_events (event_id,archive_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![event_id,archive_id,policy_id,event_type,outcome,actor_type,actor_id,detail_code,metadata_json,event_hash,created_at],
    )?;
    Ok(())
}

fn archive_key(config: &AppConfig, connection: &Connection) -> Result<[u8; 32]> {
    let installation_id = database::installation_id(connection)?;
    let backup_key = Zeroizing::new(backup_key::load_or_create(config, &installation_id)?);
    let mut hasher = Sha256::new();
    hasher.update(b"MicrogifterHomeServerEvidenceArchive:v1\0");
    hasher.update(backup_key.as_ref());
    Ok(hasher.finalize().into())
}

fn archive_directory(config: &AppConfig) -> PathBuf {
    config.data_dir.join(ARCHIVE_DIRECTORY)
}

fn canonical_managed_archive_path(config: &AppConfig, path: &Path) -> Result<PathBuf> {
    let directory = archive_directory(config);
    fs::create_dir_all(&directory)?;
    let canonical_directory = directory.canonicalize()?;
    let canonical_path = if path.exists() {
        path.canonicalize()?
    } else {
        let parent = path
            .parent()
            .context("evidence archive path has no parent")?
            .canonicalize()?;
        let file_name = path
            .file_name()
            .context("evidence archive path has no file name")?;
        parent.join(file_name)
    };
    ensure!(
        canonical_path.starts_with(&canonical_directory),
        "evidence archive path escaped the managed directory"
    );
    ensure!(
        canonical_path.extension().and_then(|value| value.to_str()) == Some("mgha"),
        "evidence archive path has an invalid extension"
    );
    Ok(canonical_path)
}

fn write_atomic(temporary: &Path, final_path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = final_path
        .parent()
        .context("evidence archive directory is unavailable")?;
    fs::create_dir_all(directory)?;
    let mut output = File::create(temporary)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);
    fs::rename(temporary, final_path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(final_path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn is_allowed_evidence_table(table: &str) -> bool {
    REVIEWED_EVIDENCE_TABLES.contains(&table)
}

fn quote_identifier(value: &str) -> Result<String> {
    ensure!(
        valid_identifier(value),
        "SQLite evidence identifier is invalid"
    );
    Ok(format!("\"{value}\""))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::Number(value.into()),
        ValueRef::Real(value) if value.is_finite() => serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String("non_finite_real".to_owned())),
        ValueRef::Real(_) => Value::String("non_finite_real".to_owned()),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => Value::String(format!("sha256:{}", sha256_bytes(value))),
    }
}

fn canonical_scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "invalid".to_owned()),
    }
}

fn hash_chain(previous: &str, record: &str) -> String {
    hash_bytes_parts(&[previous.as_bytes(), b"\n", record.as_bytes()])
}

fn hash_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(sha256_bytes(&serde_json::to_vec(value)?))
}

fn hash_text(value: &str) -> String {
    sha256_bytes(value.as_bytes())
}

fn hash_bytes_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    hex::encode(hasher.finalize())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid UTC timestamp: {value}"))
        .map(|value| value.with_timezone(&Utc))
}

fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn validate_uuid(value: &str, label: &str) -> Result<String> {
    let value = value.trim();
    Uuid::parse_str(value).with_context(|| format!("{label} is invalid"))?;
    Ok(value.to_owned())
}

fn validate_hash(value: &str, label: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{label} is invalid"
    );
    Ok(value)
}

fn bounded_text(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    let count = value.chars().count();
    ensure!(
        (minimum..=maximum).contains(&count),
        "{label} length is invalid"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} contains control characters"
    );
    Ok(value.to_owned())
}

fn bounded_failure_code(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .take(120)
        .collect::<String>();
    if normalized.is_empty() {
        "archive_failed".to_owned()
    } else {
        normalized
    }
}

fn safe_archive_file_name(value: &str) -> String {
    let name = value
        .chars()
        .take(200)
        .map(|character| {
            if character.is_ascii_alphanumeric() || ".-_".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if name.ends_with(".mgha") && name.len() > ".mgha".len() {
        name
    } else {
        "Microgifter-HomeServer-Evidence.mgha".to_owned()
    }
}

fn nonnegative_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("evidence_archive_task_failed", error.into())
}

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string().chars().take(500).collect(),
        }),
    )
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string().chars().take(500).collect(),
        }),
    )
}

impl From<PolicyRecord> for EvidenceArchivePolicySummary {
    fn from(value: PolicyRecord) -> Self {
        Self {
            policy_id: value.policy_id,
            policy_revision: value.policy_revision,
            enabled: value.enabled,
            interval_hours: value.interval_hours,
            max_records_per_archive: value.max_records_per_archive,
            retention_count: value.retention_count,
            max_package_bytes: value.max_package_bytes,
            policy_hash: value.policy_hash,
            created_by_user_id: value.created_by_user_id,
            reason: value.reason,
            created_at_utc: value.created_at_utc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_allowlist_is_explicit_and_rejects_future_suffix_matches() {
        assert!(is_allowed_evidence_table("wrapper_authorization_receipts"));
        assert!(is_allowed_evidence_table("agent_runtime_receipts"));
        assert!(is_allowed_evidence_table("model_inference_events"));
        assert!(is_allowed_evidence_table(
            "private_knowledge_access_receipts"
        ));
        assert!(!is_allowed_evidence_table(
            "model_inference_private_results"
        ));
        assert!(!is_allowed_evidence_table("agent_messages"));
        assert!(!is_allowed_evidence_table("future_private_events"));
        assert!(!is_allowed_evidence_table("future_secret_receipts"));
        assert!(!is_allowed_evidence_table("evidence_archive_events"));
    }

    #[test]
    fn archive_chain_is_deterministic() {
        let first = hash_chain(ZERO_HASH, &"a".repeat(64));
        let second = hash_chain(&first, &"b".repeat(64));
        assert_eq!(first.len(), 64);
        assert_eq!(second.len(), 64);
        assert_ne!(first, second);
        assert_eq!(second, hash_chain(&first, &"b".repeat(64)));
    }

    #[test]
    fn archive_file_names_are_bounded() {
        assert_eq!(safe_archive_file_name("evidence.mgha"), "evidence.mgha");
        assert_eq!(
            safe_archive_file_name("../unsafe.exe"),
            "Microgifter-HomeServer-Evidence.mgha"
        );
    }
}
