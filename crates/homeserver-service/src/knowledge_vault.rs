use crate::{config::AppConfig, document_extraction, AppState};
use anyhow::{ensure, Context, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

const VAULT_MIGRATION: &str = include_str!("../../../database/migrations/0005_knowledge_vault.sql");
const VAULT_MIGRATION_KEY: &str = "0005_knowledge_vault";
const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILE_NAME_CHARS: usize = 180;
const MAX_TAGS: usize = 20;
const MAX_TAG_CHARS: usize = 64;
const MAX_SEARCH_QUERY_CHARS: usize = 200;
const MAX_SEARCH_RESULTS: u32 = 50;
const FILE_NAME_HEADER: &str = "x-mg-vault-file-name";
const TAGS_HEADER: &str = "x-mg-vault-tags";
const DOCUMENT_COLUMNS: &str = "document_id,file_name,title,content_type,size_bytes,sha256,state,tags_json,created_at_utc,updated_at_utc,indexed_at_utc,failure_code";

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultDocument {
    pub document_id: String,
    pub file_name: String,
    pub title: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub state: String,
    pub tags: Vec<String>,
    pub created_at_utc: DateTime<Utc>,
    pub updated_at_utc: DateTime<Utc>,
    pub indexed_at_utc: Option<DateTime<Utc>>,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultSnapshot {
    pub documents: Vec<VaultDocument>,
    pub indexed_count: u64,
    pub changed_count: u64,
    pub missing_count: u64,
    pub failed_count: u64,
    pub total_size_bytes: u64,
    pub last_indexed_at_utc: Option<DateTime<Utc>>,
    pub supported_extensions: Vec<String>,
    pub extraction: document_extraction::ExtractionSnapshot,
    pub local_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct VaultSearchRequest {
    pub query: String,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultSearchHit {
    pub document: VaultDocument,
    pub snippet: String,
    pub score: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultSearchResult {
    pub query: String,
    pub hits: Vec<VaultSearchHit>,
}

#[derive(Debug, Deserialize)]
pub struct VaultDeleteRequest {
    pub document_id: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultActionResult {
    pub document: Option<VaultDocument>,
    pub affected: u64,
    pub message: String,
}

pub fn initialize(connection: &Connection, config: &AppConfig) -> Result<()> {
    fs::create_dir_all(documents_dir(config))
        .context("unable to create Knowledge Vault storage")?;
    connection.execute_batch(VAULT_MIGRATION)?;
    health_check(connection)?;
    Ok(())
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![VAULT_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "Knowledge Vault migration is not registered exactly once"
    );
    let _: i64 =
        connection.query_row("SELECT COUNT(*) FROM vault_documents", [], |row| row.get(0))?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/vault", get(vault_snapshot))
        .route("/v1/vault/import", post(import_document))
        .route("/v1/vault/search", post(search_documents))
        .route("/v1/vault/reindex", post(reindex_documents))
        .route("/v1/vault/delete", post(delete_document))
        .layer(DefaultBodyLimit::max(MAX_DOCUMENT_BYTES))
        .with_state(state)
}

impl AppState {
    fn vault_snapshot(&self) -> Result<VaultSnapshot> {
        let connection = self.connection()?;
        snapshot(&connection)
    }

    fn import_vault_document(
        &self,
        file_name: String,
        tags: Vec<String>,
        bytes: Vec<u8>,
    ) -> Result<VaultActionResult> {
        let connection = self.connection()?;
        import(&connection, &self.config, &file_name, tags, &bytes)
    }

    fn search_vault(&self, request: VaultSearchRequest) -> Result<VaultSearchResult> {
        let connection = self.connection()?;
        search(&connection, request)
    }

    fn reindex_vault(&self) -> Result<VaultActionResult> {
        let connection = self.connection()?;
        reindex(&connection, &self.config)
    }

    fn delete_vault_document(&self, request: VaultDeleteRequest) -> Result<VaultActionResult> {
        let connection = self.connection()?;
        delete(&connection, &self.config, request)
    }
}

async fn vault_snapshot(State(state): State<Arc<AppState>>) -> ApiResult<VaultSnapshot> {
    tokio::task::spawn_blocking(move || state.vault_snapshot())
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("vault_snapshot_failed", error))
}

async fn import_document(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<VaultActionResult> {
    let file_name = decode_header(&headers, FILE_NAME_HEADER, 1024, "vault_file_name_invalid")?;
    let tags_json = decode_header(&headers, TAGS_HEADER, 8192, "vault_tags_invalid")?;
    let tags: Vec<String> = serde_json::from_str(&tags_json)
        .map_err(|error| action_error("vault_tags_invalid", error.into()))?;
    let bytes = body.to_vec();
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(action_error(
            "vault_import_too_large",
            anyhow::anyhow!("document exceeds the 32 MB import limit"),
        ));
    }
    if bytes.is_empty() {
        return Err(action_error(
            "vault_import_invalid",
            anyhow::anyhow!("document is empty"),
        ));
    }

    tokio::task::spawn_blocking(move || state.import_vault_document(file_name, tags, bytes))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("vault_import_failed", error))
}

async fn search_documents(
    State(state): State<Arc<AppState>>,
    Json(request): Json<VaultSearchRequest>,
) -> ApiResult<VaultSearchResult> {
    tokio::task::spawn_blocking(move || state.search_vault(request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("vault_search_failed", error))
}

async fn reindex_documents(State(state): State<Arc<AppState>>) -> ApiResult<VaultActionResult> {
    tokio::task::spawn_blocking(move || state.reindex_vault())
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("vault_reindex_failed", error))
}

async fn delete_document(
    State(state): State<Arc<AppState>>,
    Json(request): Json<VaultDeleteRequest>,
) -> ApiResult<VaultActionResult> {
    tokio::task::spawn_blocking(move || state.delete_vault_document(request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("vault_delete_failed", error))
}

fn import(
    connection: &Connection,
    config: &AppConfig,
    file_name: &str,
    tags: Vec<String>,
    bytes: &[u8],
) -> Result<VaultActionResult> {
    ensure!(!bytes.is_empty(), "document is empty");
    ensure!(
        bytes.len() <= MAX_DOCUMENT_BYTES,
        "document exceeds the 32 MB import limit"
    );
    let file_name = validate_file_name(file_name)?;
    let extension = extension(&file_name)?;
    let tags = normalize_tags(tags)?;
    let sha256 = hex::encode(Sha256::digest(bytes));

    if let Some(document) = document_by_sha(connection, &sha256)? {
        return Ok(VaultActionResult {
            document: Some(document),
            affected: 0,
            message: "This document is already in the Knowledge Vault.".to_owned(),
        });
    }

    let document_id = Uuid::new_v4().to_string();
    let managed_name = format!("{}.{}", Uuid::new_v4().simple(), extension);
    let destination = documents_dir(config).join(managed_name);
    validate_managed_path(config, &destination)?;
    let temporary = destination.with_extension(format!("{}.tmp", extension));
    let write_result = (|| -> Result<()> {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        output.write_all(bytes)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error).context("unable to store the managed Knowledge Vault document");
    }

    let operation_id =
        match document_extraction::begin_operation(connection, None, &file_name, "import") {
            Ok(operation_id) => operation_id,
            Err(error) => {
                let _ = fs::remove_file(&destination);
                return Err(error).context("unable to start the document extraction operation");
            }
        };
    let extraction = document_extraction::extract_document(
        config,
        &extension,
        bytes,
        &destination,
        |processed, total| {
            document_extraction::update_operation_progress(
                connection,
                &operation_id,
                processed,
                total,
            )
        },
    );
    let extraction = match extraction {
        Ok(value) => value,
        Err(error) => {
            let _ = document_extraction::finish_operation(
                connection,
                &operation_id,
                "failed",
                "Document extraction failed",
                Some("document_extraction_failed"),
            );
            let _ = fs::remove_file(&destination);
            return Err(error).context("unable to extract managed Knowledge Vault document");
        }
    };

    let title = title_from_file_name(&file_name);
    let content_type = content_type(&extension);
    let tags_json = serde_json::to_string(&tags)?;
    let document_state = if extraction.state == "ocr_required" || extraction.state == "failed" {
        "failed"
    } else {
        "indexed"
    };
    let registration_result = (|| -> Result<()> {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "INSERT INTO vault_documents (document_id,file_name,title,managed_path,content_type,size_bytes,sha256,state,tags_json,indexed_text,failure_code,indexed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,CASE WHEN ?8='indexed' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE NULL END)",
            params![
                document_id,
                file_name,
                title,
                destination.to_string_lossy(),
                content_type,
                bytes.len() as i64,
                sha256,
                document_state,
                tags_json,
                extraction.indexed_text,
                extraction.failure_code,
            ],
        )?;
        document_extraction::store_extraction(&transaction, &document_id, &sha256, &extraction)?;
        transaction.execute(
            "UPDATE vault_extraction_operations SET document_id=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?2",
            params![document_id, operation_id],
        )?;
        transaction.execute(
            "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.document_indexed','A local document was processed in Knowledge Vault',json_object('document_id',?1,'file_name',?2,'size_bytes',?3,'extraction_method',?4,'page_count',?5,'state',?6))",
            params![
                document_id,
                file_name,
                bytes.len() as i64,
                extraction.extraction_method,
                extraction.pages.len() as i64,
                extraction.state,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    })();
    if let Err(error) = registration_result {
        let _ = fs::remove_file(&destination);
        let _ = document_extraction::finish_operation(
            connection,
            &operation_id,
            "failed",
            "Document registration failed",
            Some("document_registration_failed"),
        );
        return Err(error).context("unable to register the Knowledge Vault document");
    }
    document_extraction::finish_operation(
        connection,
        &operation_id,
        "completed",
        if document_state == "indexed" {
            "Document extraction completed"
        } else {
            "Document stored; local OCR runtime is required"
        },
        extraction.failure_code.as_deref(),
    )?;

    Ok(VaultActionResult {
        document: Some(document_by_id(connection, &document_id)?),
        affected: 1,
        message: if extraction.state == "ocr_required" {
            "Document copied into managed storage. Install the local OCR runtime, then use Check Files to extract searchable text.".to_owned()
        } else if extraction.state == "partial" {
            format!(
                "Document indexed with {} pages; {} scanned pages still need local OCR.",
                extraction.pages.len(),
                extraction.ocr_required_page_count
            )
        } else {
            format!(
                "Document copied into managed storage and extracted across {} page(s).",
                extraction.pages.len()
            )
        },
    })
}

fn snapshot(connection: &Connection) -> Result<VaultSnapshot> {
    let mut statement = connection.prepare(&format!(
        "SELECT {DOCUMENT_COLUMNS} FROM vault_documents ORDER BY updated_at_utc DESC,document_id DESC LIMIT 200"
    ))?;
    let documents = statement
        .query_map([], document_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let counts = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(CASE WHEN state='indexed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='changed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='missing' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN state='failed' THEN 1 ELSE 0 END),0),COALESCE(SUM(size_bytes),0),MAX(indexed_at_utc) FROM vault_documents",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        },
    )?;
    let _total = counts.0;
    Ok(VaultSnapshot {
        documents,
        indexed_count: counts.1.max(0) as u64,
        changed_count: counts.2.max(0) as u64,
        missing_count: counts.3.max(0) as u64,
        failed_count: counts.4.max(0) as u64,
        total_size_bytes: counts.5.max(0) as u64,
        last_indexed_at_utc: counts.6.map(parse_utc).transpose()?,
        supported_extensions: document_extraction::supported_extensions()
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        extraction: document_extraction::snapshot(connection)?,
        local_only: true,
    })
}

fn search(connection: &Connection, request: VaultSearchRequest) -> Result<VaultSearchResult> {
    let query = request.query.trim();
    ensure!(!query.is_empty(), "search query is required");
    ensure!(
        query.chars().count() <= MAX_SEARCH_QUERY_CHARS,
        "search query exceeds the 200 character limit"
    );
    let limit = request.limit.unwrap_or(20).clamp(1, MAX_SEARCH_RESULTS);
    let sql = format!(
        "SELECT {DOCUMENT_COLUMNS},indexed_text FROM vault_documents WHERE state='indexed' AND (instr(lower(title),lower(?1)) > 0 OR instr(lower(indexed_text),lower(?1)) > 0) ORDER BY updated_at_utc DESC LIMIT ?2"
    );
    let mut statement = connection.prepare(&sql)?;
    let hits = statement
        .query_map(params![query, limit as i64], |row| {
            let document = document_from_row(row)?;
            let indexed_text = row.get::<_, String>(12)?;
            let score = match_count(&document.title, query)
                .saturating_mul(10)
                .saturating_add(match_count(&indexed_text, query));
            Ok(VaultSearchHit {
                document,
                snippet: search_snippet(&indexed_text, query),
                score,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(VaultSearchResult {
        query: query.to_owned(),
        hits,
    })
}

fn reindex(connection: &Connection, config: &AppConfig) -> Result<VaultActionResult> {
    let mut statement = connection.prepare(
        "SELECT document_id,managed_path,file_name,sha256 FROM vault_documents ORDER BY document_id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    let mut affected = 0_u64;
    for (document_id, path, file_name, expected_sha256) in rows {
        affected += 1;
        if let Err(error) = validate_managed_path(config, &path) {
            mark_document_state(connection, &document_id, "failed", "managed_path_rejected")?;
            tracing::warn!(?error, %document_id, "Knowledge Vault managed path was rejected");
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                mark_document_state(connection, &document_id, "missing", "managed_file_missing")?;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            mark_document_state(
                connection,
                &document_id,
                "failed",
                "managed_file_type_rejected",
            )?;
            continue;
        }
        if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
            mark_document_state(connection, &document_id, "failed", "managed_file_too_large")?;
            continue;
        }
        let bytes = fs::read(&path)?;
        let actual_sha256 = hex::encode(Sha256::digest(&bytes));
        if actual_sha256 != expected_sha256 {
            mark_document_state(
                connection,
                &document_id,
                "changed",
                "content_changed_reimport_required",
            )?;
            continue;
        }
        let extension = extension(&file_name)?;
        let operation_id = document_extraction::begin_operation(
            connection,
            Some(&document_id),
            &file_name,
            "reindex",
        )?;
        match document_extraction::extract_document(
            config,
            &extension,
            &bytes,
            &path,
            |processed, total| {
                document_extraction::update_operation_progress(
                    connection,
                    &operation_id,
                    processed,
                    total,
                )
            },
        ) {
            Ok(extraction) => {
                let document_state =
                    if extraction.state == "ocr_required" || extraction.state == "failed" {
                        "failed"
                    } else {
                        "indexed"
                    };
                let transaction = connection.unchecked_transaction()?;
                transaction.execute(
                    "UPDATE vault_documents SET state=?1,indexed_text=?2,failure_code=?3,indexed_at_utc=CASE WHEN ?1='indexed' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE NULL END,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE document_id=?4",
                    params![
                        document_state,
                        extraction.indexed_text,
                        extraction.failure_code,
                        document_id,
                    ],
                )?;
                document_extraction::store_extraction(
                    &transaction,
                    &document_id,
                    &expected_sha256,
                    &extraction,
                )?;
                transaction.execute(
                    "UPDATE vault_semantic_documents SET state='stale',failure_code='extracted_text_changed',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE document_id=?1 AND state='ready'",
                    params![document_id],
                )?;
                transaction.commit()?;
                document_extraction::finish_operation(
                    connection,
                    &operation_id,
                    "completed",
                    if document_state == "indexed" {
                        "Document re-extraction completed"
                    } else {
                        "Document still requires local OCR"
                    },
                    extraction.failure_code.as_deref(),
                )?;
            }
            Err(error) => {
                mark_document_state(connection, &document_id, "failed", "text_extraction_failed")?;
                document_extraction::finish_operation(
                    connection,
                    &operation_id,
                    "failed",
                    "Document re-extraction failed",
                    Some("text_extraction_failed"),
                )?;
                tracing::warn!(?error, %document_id, "Knowledge Vault reindex failed");
            }
        }
    }
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.reindex_completed','Knowledge Vault managed documents were checked and re-extracted',json_object('affected',?1))",
        params![affected as i64],
    )?;
    Ok(VaultActionResult {
        document: None,
        affected,
        message: format!("Checked and re-extracted {affected} managed documents."),
    })
}

fn delete(
    connection: &Connection,
    config: &AppConfig,
    request: VaultDeleteRequest,
) -> Result<VaultActionResult> {
    ensure!(
        request.confirmation == "DELETE",
        "type DELETE to remove this managed Knowledge Vault copy"
    );
    let (document, path) = document_with_path(connection, &request.document_id)?;
    validate_managed_path(config, &path)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            ensure!(
                !metadata.file_type().is_symlink(),
                "managed document is a symlink"
            );
            ensure!(metadata.is_file(), "managed document is not a regular file");
            fs::remove_file(&path)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM vault_documents WHERE document_id=?1",
        params![request.document_id],
    )?;
    transaction.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.document_deleted','A managed Knowledge Vault document copy was deleted',json_object('document_id',?1,'file_name',?2))",
        params![document.document_id, document.file_name],
    )?;
    transaction.commit()?;
    Ok(VaultActionResult {
        document: None,
        affected: 1,
        message:
            "Managed Knowledge Vault copy and index were deleted. The source file was not changed."
                .to_owned(),
    })
}

fn document_by_id(connection: &Connection, document_id: &str) -> Result<VaultDocument> {
    connection
        .query_row(
            &format!("SELECT {DOCUMENT_COLUMNS} FROM vault_documents WHERE document_id=?1"),
            params![document_id],
            document_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("Knowledge Vault document was not found"))
}

fn document_by_sha(connection: &Connection, sha256: &str) -> Result<Option<VaultDocument>> {
    connection
        .query_row(
            &format!("SELECT {DOCUMENT_COLUMNS} FROM vault_documents WHERE sha256=?1"),
            params![sha256],
            document_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn document_with_path(
    connection: &Connection,
    document_id: &str,
) -> Result<(VaultDocument, PathBuf)> {
    let sql =
        format!("SELECT {DOCUMENT_COLUMNS},managed_path FROM vault_documents WHERE document_id=?1");
    connection
        .query_row(&sql, params![document_id], |row| {
            Ok((
                document_from_row(row)?,
                PathBuf::from(row.get::<_, String>(12)?),
            ))
        })
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("Knowledge Vault document was not found"))
}

fn document_from_row(row: &Row<'_>) -> rusqlite::Result<VaultDocument> {
    let tags_json = row.get::<_, String>(7)?;
    let tags = serde_json::from_str(&tags_json).map_err(|error| to_sql_error(error.into()))?;
    let created = parse_utc(row.get::<_, String>(8)?).map_err(to_sql_error)?;
    let updated = parse_utc(row.get::<_, String>(9)?).map_err(to_sql_error)?;
    let indexed = row
        .get::<_, Option<String>>(10)?
        .map(parse_utc)
        .transpose()
        .map_err(to_sql_error)?;
    Ok(VaultDocument {
        document_id: row.get(0)?,
        file_name: row.get(1)?,
        title: row.get(2)?,
        content_type: row.get(3)?,
        size_bytes: row.get::<_, i64>(4)?.max(0) as u64,
        sha256: row.get(5)?,
        state: row.get(6)?,
        tags,
        created_at_utc: created,
        updated_at_utc: updated,
        indexed_at_utc: indexed,
        failure_code: row.get(11)?,
    })
}

fn mark_document_state(
    connection: &Connection,
    document_id: &str,
    state: &str,
    failure_code: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE vault_documents SET state=?1,failure_code=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE document_id=?3",
        params![state, failure_code, document_id],
    )?;
    Ok(())
}

fn validate_file_name(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(!value.is_empty(), "document file name is required");
    ensure!(
        value.chars().count() <= MAX_FILE_NAME_CHARS,
        "document file name exceeds the length limit"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "document file name contains control characters"
    );
    let mut components = Path::new(value).components();
    let component = components.next();
    ensure!(
        matches!(component, Some(Component::Normal(_))) && components.next().is_none(),
        "document file name must not contain a path"
    );
    extension(value)?;
    Ok(value.to_owned())
}

fn extension(file_name: &str) -> Result<String> {
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .context("document must have a supported file extension")?;
    ensure!(
        document_extraction::supported_extensions().contains(&extension.as_str()),
        "unsupported Knowledge Vault document type"
    );
    Ok(extension)
}

fn title_from_file_name(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Untitled document")
        .chars()
        .take(MAX_FILE_NAME_CHARS)
        .collect()
}

fn content_type(extension: &str) -> &'static str {
    match extension {
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "log" => "text/plain",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "tif" | "tiff" => "image/tiff",
        _ => "text/plain",
    }
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>> {
    ensure!(tags.len() <= MAX_TAGS, "too many Knowledge Vault tags");
    let mut normalized = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        ensure!(
            tag.chars().count() <= MAX_TAG_CHARS,
            "Knowledge Vault tag exceeds the length limit"
        );
        ensure!(
            !tag.chars().any(char::is_control),
            "Knowledge Vault tag contains control characters"
        );
        let tag = tag.to_owned();
        if !normalized.contains(&tag) {
            normalized.push(tag);
        }
    }
    Ok(normalized)
}

fn documents_dir(config: &AppConfig) -> PathBuf {
    config.data_dir.join("vault").join("documents")
}

fn validate_managed_path(config: &AppConfig, path: &Path) -> Result<()> {
    let expected = documents_dir(config).canonicalize()?;
    let parent = path
        .parent()
        .context("managed Knowledge Vault path has no parent")?
        .canonicalize()?;
    ensure!(
        parent == expected,
        "document path is outside managed Knowledge Vault storage"
    );
    ensure!(
        path.file_name().is_some(),
        "managed Knowledge Vault path has no file name"
    );
    Ok(())
}

fn match_count(text: &str, query: &str) -> u32 {
    let text = text.to_lowercase();
    let query = query.to_lowercase();
    if query.is_empty() {
        return 0;
    }
    text.match_indices(query.as_str())
        .count()
        .min(u32::MAX as usize) as u32
}

fn search_snippet(text: &str, query: &str) -> String {
    let lower = text.to_lowercase();
    let query = query.to_lowercase();
    let byte_position = lower.find(query.as_str()).unwrap_or(0);
    let char_position = lower[..byte_position].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let start = char_position.saturating_sub(90).min(chars.len());
    let end = (char_position + query.chars().count() + 180).min(chars.len());
    let mut snippet: String = chars[start..end].iter().collect();
    snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet
}

fn decode_header(
    headers: &HeaderMap,
    name: &'static str,
    maximum: usize,
    code: &'static str,
) -> Result<String, (StatusCode, Json<ApiError>)> {
    let encoded = headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            action_error(
                code,
                anyhow::anyhow!("required Knowledge Vault header is missing"),
            )
        })?;
    if encoded.len() > maximum {
        return Err(action_error(
            code,
            anyhow::anyhow!("Knowledge Vault header exceeds its size limit"),
        ));
    }
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| {
        action_error(
            code,
            anyhow::anyhow!("Knowledge Vault header encoding is invalid"),
        )
    })?;
    String::from_utf8(bytes).map_err(|_| {
        action_error(
            code,
            anyhow::anyhow!("Knowledge Vault header must be UTF-8"),
        )
    })
}

fn parse_utc(value: String) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("invalid UTC timestamp '{value}'"))
}

fn to_sql_error(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "vault_task_failed",
        anyhow::anyhow!(error),
    )
}

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    let text = error.to_string().to_lowercase();
    let status = if text.contains("unsupported")
        || text.contains("required")
        || text.contains("invalid")
        || text.contains("empty")
        || text.contains("limit")
        || text.contains("not found")
        || text.contains("outside managed")
        || text.contains("type delete")
        || text.contains("path")
        || text.contains("utf-8")
        || text.contains("already")
    {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    api_error(status, code, error)
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::INTERNAL_SERVER_ERROR, code, error)
}

fn api_error(
    status: StatusCode,
    code: &'static str,
    error: anyhow::Error,
) -> (StatusCode, Json<ApiError>) {
    (
        status,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string().chars().take(500).collect(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_basenames_and_supported_document_types_only() {
        assert_eq!(
            validate_file_name("operations.md").unwrap(),
            "operations.md"
        );
        assert!(validate_file_name("../operations.md").is_err());
        assert_eq!(
            validate_file_name("operations.pdf").unwrap(),
            "operations.pdf"
        );
        assert_eq!(
            validate_file_name("handbook.docx").unwrap(),
            "handbook.docx"
        );
        assert!(validate_file_name("macro.docm").is_err());
        assert!(validate_file_name("folder/operations.txt").is_err());
    }

    #[test]
    fn tags_are_bounded_deduplicated_and_trimmed() {
        let tags = normalize_tags(vec![
            " policy ".to_owned(),
            "policy".to_owned(),
            "training".to_owned(),
        ])
        .unwrap();
        assert_eq!(tags, vec!["policy", "training"]);
    }

    #[test]
    fn snippets_are_bounded_and_readable() {
        let text = format!(
            "{} private roadmap {}",
            "before ".repeat(100),
            "after ".repeat(100)
        );
        let snippet = search_snippet(&text, "private roadmap");
        assert!(snippet.contains("private roadmap"));
        assert!(snippet.len() < text.len());
    }
}
