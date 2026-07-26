from __future__ import annotations

import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parents[1]


def write(path: str, content: str) -> None:
    target = ROOT / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(content.rstrip() + "\n", encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    if content.count(old) != 1:
        raise RuntimeError(f"expected one match in {path}: {old[:80]!r}")
    target.write_text(content.replace(old, new, 1), encoding="utf-8")


def replace_regex(path: str, pattern: str, replacement: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    updated, count = re.subn(pattern, replacement, content, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"expected one regex match in {path}: {pattern[:80]!r}")
    target.write_text(updated, encoding="utf-8")


write(
    "database/migrations/0005_knowledge_vault.sql",
    r'''
CREATE TABLE IF NOT EXISTS vault_documents (
    document_id TEXT PRIMARY KEY,
    file_name TEXT NOT NULL,
    title TEXT NOT NULL,
    managed_path TEXT NOT NULL UNIQUE,
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    sha256 TEXT NOT NULL UNIQUE CHECK (length(sha256) = 64),
    state TEXT NOT NULL CHECK (state IN ('indexed','changed','missing','failed')),
    tags_json TEXT NOT NULL DEFAULT '[]',
    indexed_text TEXT NOT NULL DEFAULT '',
    failure_code TEXT,
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    indexed_at_utc TEXT
);

CREATE INDEX IF NOT EXISTS idx_vault_documents_state_updated
    ON vault_documents(state, updated_at_utc DESC);
CREATE INDEX IF NOT EXISTS idx_vault_documents_title
    ON vault_documents(title);

CREATE TABLE IF NOT EXISTS vault_access_rules (
    access_rule_id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    document_id TEXT NOT NULL,
    access_level TEXT NOT NULL CHECK (access_level IN ('read','search')),
    created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(agent_id, document_id, access_level),
    FOREIGN KEY(document_id) REFERENCES vault_documents(document_id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0005_knowledge_vault');
''',
)

write(
    "crates/homeserver-service/src/knowledge_vault.rs",
    r'''
use crate::{config::AppConfig, AppState};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

const VAULT_MIGRATION: &str =
    include_str!("../../../database/migrations/0005_knowledge_vault.sql");
const VAULT_MIGRATION_KEY: &str = "0005_knowledge_vault";
const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_INDEXED_CHARS: usize = 2_000_000;
const MAX_FILE_NAME_CHARS: usize = 180;
const MAX_TAGS: usize = 20;
const MAX_TAG_CHARS: usize = 64;
const MAX_SEARCH_QUERY_CHARS: usize = 200;
const MAX_SEARCH_RESULTS: u32 = 50;
const FILE_NAME_HEADER: &str = "x-mg-vault-file-name";
const TAGS_HEADER: &str = "x-mg-vault-tags";
const SUPPORTED_EXTENSIONS: [&str; 5] = ["txt", "md", "csv", "json", "log"];
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
    fs::create_dir_all(documents_dir(config)).context("unable to create Knowledge Vault storage")?;
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
    let _: i64 = connection.query_row("SELECT COUNT(*) FROM vault_documents", [], |row| {
        row.get(0)
    })?;
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
        snapshot(&self.connection()?)
    }

    fn import_vault_document(
        &self,
        file_name: String,
        tags: Vec<String>,
        bytes: Vec<u8>,
    ) -> Result<VaultActionResult> {
        import(&self.connection()?, &self.config, &file_name, tags, &bytes)
    }

    fn search_vault(&self, request: VaultSearchRequest) -> Result<VaultSearchResult> {
        search(&self.connection()?, request)
    }

    fn reindex_vault(&self) -> Result<VaultActionResult> {
        reindex(&self.connection()?, &self.config)
    }

    fn delete_vault_document(&self, request: VaultDeleteRequest) -> Result<VaultActionResult> {
        delete(&self.connection()?, &self.config, request)
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
    body: Body,
) -> ApiResult<VaultActionResult> {
    let file_name = decode_header(&headers, FILE_NAME_HEADER, 1024, "vault_file_name_invalid")?;
    let tags_json = decode_header(&headers, TAGS_HEADER, 8192, "vault_tags_invalid")?;
    let tags: Vec<String> = serde_json::from_str(&tags_json)
        .map_err(|error| action_error("vault_tags_invalid", error.into()))?;
    let mut stream = body.into_data_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| action_error("vault_import_stream_failed", error.into()))?;
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| action_error("vault_import_too_large", anyhow::anyhow!("document size overflow")))?;
        if next_len > MAX_DOCUMENT_BYTES {
            return Err(action_error(
                "vault_import_too_large",
                anyhow::anyhow!("document exceeds the 16 MB import limit"),
            ));
        }
        bytes.extend_from_slice(&chunk);
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
        "document exceeds the 16 MB import limit"
    );
    let file_name = validate_file_name(file_name)?;
    let extension = extension(&file_name)?;
    let tags = normalize_tags(tags)?;
    let indexed_text = extract_text(&extension, bytes)?;
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

    let title = title_from_file_name(&file_name);
    let content_type = content_type(&extension);
    let tags_json = serde_json::to_string(&tags)?;
    let transaction = connection.unchecked_transaction()?;
    let insert_result = transaction.execute(
        "INSERT INTO vault_documents (document_id,file_name,title,managed_path,content_type,size_bytes,sha256,state,tags_json,indexed_text,indexed_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,'indexed',?8,?9,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            document_id,
            file_name,
            title,
            destination.to_string_lossy(),
            content_type,
            bytes.len() as i64,
            sha256,
            tags_json,
            indexed_text,
        ],
    );
    if let Err(error) = insert_result {
        let _ = fs::remove_file(&destination);
        return Err(error).context("unable to register the Knowledge Vault document");
    }
    transaction.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.document_indexed','A local document was indexed in Knowledge Vault',json_object('document_id',?1,'file_name',?2,'size_bytes',?3))",
        params![document_id, file_name, bytes.len() as i64],
    )?;
    transaction.commit()?;

    Ok(VaultActionResult {
        document: Some(document_by_id(connection, &document_id)?),
        affected: 1,
        message: "Document copied into managed local storage and indexed.".to_owned(),
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
        "SELECT COUNT(*),SUM(CASE WHEN state='indexed' THEN 1 ELSE 0 END),SUM(CASE WHEN state='changed' THEN 1 ELSE 0 END),SUM(CASE WHEN state='missing' THEN 1 ELSE 0 END),SUM(CASE WHEN state='failed' THEN 1 ELSE 0 END),COALESCE(SUM(size_bytes),0),MAX(indexed_at_utc) FROM vault_documents",
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
        supported_extensions: SUPPORTED_EXTENSIONS
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
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
            mark_document_state(connection, &document_id, "failed", "managed_file_type_rejected")?;
            continue;
        }
        if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
            mark_document_state(connection, &document_id, "failed", "managed_file_too_large")?;
            continue;
        }
        let bytes = fs::read(&path)?;
        let actual_sha256 = hex::encode(Sha256::digest(&bytes));
        if actual_sha256 != expected_sha256 {
            mark_document_state(connection, &document_id, "changed", "content_changed_reimport_required")?;
            continue;
        }
        let extension = extension(&file_name)?;
        match extract_text(&extension, &bytes) {
            Ok(indexed_text) => {
                connection.execute(
                    "UPDATE vault_documents SET state='indexed',indexed_text=?1,failure_code=NULL,indexed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE document_id=?2",
                    params![indexed_text, document_id],
                )?;
            }
            Err(error) => {
                mark_document_state(connection, &document_id, "failed", "text_extraction_failed")?;
                tracing::warn!(?error, %document_id, "Knowledge Vault reindex failed");
            }
        }
    }
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.reindex_completed','Knowledge Vault managed documents were checked and reindexed',json_object('affected',?1))",
        params![affected as i64],
    )?;
    Ok(VaultActionResult {
        document: None,
        affected,
        message: format!("Checked and reindexed {affected} managed documents."),
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
            ensure!(!metadata.file_type().is_symlink(), "managed document is a symlink");
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
        message: "Managed Knowledge Vault copy and index were deleted. The source file was not changed.".to_owned(),
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

fn document_with_path(connection: &Connection, document_id: &str) -> Result<(VaultDocument, PathBuf)> {
    let sql = format!(
        "SELECT {DOCUMENT_COLUMNS},managed_path FROM vault_documents WHERE document_id=?1"
    );
    connection
        .query_row(&sql, params![document_id], |row| {
            Ok((document_from_row(row)?, PathBuf::from(row.get::<_, String>(12)?)))
        })
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("Knowledge Vault document was not found"))
}

fn document_from_row(row: &Row<'_>) -> rusqlite::Result<VaultDocument> {
    let tags_json = row.get::<_, String>(7)?;
    let tags = serde_json::from_str(&tags_json).map_err(to_sql_error)?;
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
        SUPPORTED_EXTENSIONS.contains(&extension.as_str()),
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
        _ => "text/plain",
    }
}

fn extract_text(extension: &str, bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).context("document must be UTF-8 text")?;
    ensure!(!text.contains('\0'), "document contains unsupported null bytes");
    if extension == "json" {
        let _: serde_json::Value = serde_json::from_str(text).context("JSON document is invalid")?;
    }
    Ok(text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .take(MAX_INDEXED_CHARS)
        .collect())
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
    ensure!(parent == expected, "document path is outside managed Knowledge Vault storage");
    ensure!(path.file_name().is_some(), "managed Knowledge Vault path has no file name");
    Ok(())
}

fn match_count(text: &str, query: &str) -> u32 {
    let text = text.to_lowercase();
    let query = query.to_lowercase();
    if query.is_empty() {
        return 0;
    }
    text.match_indices(&query).count().min(u32::MAX as usize) as u32
}

fn search_snippet(text: &str, query: &str) -> String {
    let lower = text.to_lowercase();
    let query = query.to_lowercase();
    let byte_position = lower.find(&query).unwrap_or(0);
    let char_position = lower[..byte_position].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let start = char_position.saturating_sub(90).min(chars.len());
    let end = (char_position + query.chars().count() + 180).min(chars.len());
    let mut snippet: String = chars[start..end].iter().collect();
    snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        snippet.insert_str(0, "…");
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
        .ok_or_else(|| action_error(code, anyhow::anyhow!("required Knowledge Vault header is missing")))?;
    if encoded.len() > maximum {
        return Err(action_error(
            code,
            anyhow::anyhow!("Knowledge Vault header exceeds its size limit"),
        ));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| action_error(code, anyhow::anyhow!("Knowledge Vault header encoding is invalid")))?;
    String::from_utf8(bytes)
        .map_err(|_| action_error(code, anyhow::anyhow!("Knowledge Vault header must be UTF-8")))
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
    fn file_names_are_basenames_and_supported_text_types_only() {
        assert_eq!(validate_file_name("operations.md").unwrap(), "operations.md");
        assert!(validate_file_name("../operations.md").is_err());
        assert!(validate_file_name("operations.pdf").is_err());
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
        let text = format!("{} private roadmap {}", "before ".repeat(100), "after ".repeat(100));
        let snippet = search_snippet(&text, "private roadmap");
        assert!(snippet.contains("private roadmap"));
        assert!(snippet.len() < text.len());
    }
}
''',
)

write(
    "src-tauri/src/vault.rs",
    r'''
use super::{client, decode_json, get_json, post_json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rfd::AsyncFileDialog;
use serde_json::{json, Value};
use std::path::Path;
use tokio_util::io::ReaderStream;

const MAX_VAULT_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;
const FILE_NAME_HEADER: &str = "x-mg-vault-file-name";
const TAGS_HEADER: &str = "x-mg-vault-tags";

#[tauri::command]
pub(crate) async fn homeserver_vault() -> Result<Value, String> {
    get_json("/v1/vault").await
}

#[tauri::command]
pub(crate) async fn homeserver_search_vault(query: String) -> Result<Value, String> {
    post_json(
        "/v1/vault/search",
        &json!({ "query": query, "limit": 20 }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_reindex_vault() -> Result<Value, String> {
    post_json("/v1/vault/reindex", &json!({})).await
}

#[tauri::command]
pub(crate) async fn homeserver_delete_vault_document(
    document_id: String,
    confirmation: String,
) -> Result<Value, String> {
    post_json(
        "/v1/vault/delete",
        &json!({ "document_id": document_id, "confirmation": confirmation }),
    )
    .await
}

#[tauri::command]
pub(crate) async fn homeserver_import_vault_document(
    tags: Vec<String>,
) -> Result<Option<Value>, String> {
    let Some(source) = AsyncFileDialog::new()
        .add_filter("Knowledge Vault text documents", &["txt", "md", "csv", "json", "log"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let path = source.path();
    reject_unsafe_source(path)?;
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| error.to_string())?;
    if metadata.len() == 0 || metadata.len() > MAX_VAULT_DOCUMENT_BYTES {
        return Err("Knowledge Vault documents must be between 1 byte and 16 MB.".to_owned());
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Selected document file name is invalid.".to_owned())?;
    let input = tokio::fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let encoded_name = URL_SAFE_NO_PAD.encode(file_name.as_bytes());
    let tags_json = serde_json::to_vec(&tags).map_err(|error| error.to_string())?;
    let encoded_tags = URL_SAFE_NO_PAD.encode(tags_json);
    let response = client()?
        .post(format!(
            "{}/v1/vault/import",
            microgifter_homeserver_core::api_base_url()
        ))
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .header(reqwest::header::CONTENT_LENGTH, metadata.len())
        .header(FILE_NAME_HEADER, encoded_name)
        .header(TAGS_HEADER, encoded_tags)
        .body(reqwest::Body::wrap_stream(ReaderStream::new(input)))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    decode_json(response).await.map(Some)
}

fn reject_unsafe_source(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("Knowledge Vault does not import symbolic links or reparse-point files.".to_owned());
    }
    if !metadata.is_file() {
        return Err("Knowledge Vault can only import regular files.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maximum_document_size_is_bounded() {
        assert_eq!(MAX_VAULT_DOCUMENT_BYTES, 16 * 1024 * 1024);
    }
}
''',
)

write(
    "docs/phase-4a-knowledge-vault.md",
    r'''
# Phase 4A — Knowledge Vault foundation

## Scope delivered by this build

- Managed local copies of approved UTF-8 text, Markdown, CSV, JSON, and log files.
- Native Tauri file selection; local API callers never provide source filesystem paths.
- 16 MB per-document limit and a two-million-character indexing ceiling.
- SHA-256 duplicate prevention.
- SQLite document metadata, tags, state, extracted text, timestamps, and future agent access-rule schema.
- Local phrase search with bounded result counts and snippets.
- Reindex checks for missing, changed, oversized, symlinked, or invalid managed files.
- Explicit `DELETE` confirmation for removing a managed copy and its index.
- Audit events for imports, reindex runs, and deletion.
- Loopback-only API and trusted Control Center header enforcement inherited from the HomeServer local API.
- Live Knowledge Vault metrics, document list, import, search, reindex, and delete controls.

## Security boundary

- Imported documents are copied into `%ProgramData%\\Microgifter\\HomeServer\\vault\\documents`.
- The selected source file is not modified or deleted.
- No source path is sent to the HomeServer service.
- No Vault content is synchronized to Microgifter Cloud.
- No LAN, browser, public HTTP, model, OCR, PDF extraction, or MCP access is enabled.
- Managed paths are constrained to the canonical Vault document directory.
- Symbolic links and non-regular files are rejected.

## Deferred Phase 4 work

- PDF and office-document parsing.
- OCR.
- Embedding generation and semantic/vector search.
- Folder watching and automatic change ingestion.
- Backup package inclusion for managed document binaries.
- Agent runtime enforcement of the access-rule schema.
- Model Center and Ollama integration.

## Acceptance targets for this foundation

- Rust formatting, service tests, strict Clippy, frontend syntax, and production frontend build.
- Idempotent migration registration.
- Duplicate, unsafe-name, unsupported-type, oversized-file, missing-file, changed-file, and explicit-delete behavior.
- Existing backup, update, cloud connector, installer, signed-update, and rollback workflows must remain green before merge.
''',
)

replace_once(
    "crates/homeserver-service/src/main.rs",
    "mod http;\n",
    "mod http;\nmod knowledge_vault;\n",
)

replace_once(
    "crates/homeserver-service/src/app.rs",
    "use crate::{backup, config::AppConfig, database, http, update, update_store, AppState};",
    "use crate::{backup, config::AppConfig, database, http, knowledge_vault, update, update_store, AppState};",
)
replace_once(
    "crates/homeserver-service/src/app.rs",
    "    cloud_connector::initialize(&connection)?;\n",
    "    cloud_connector::initialize(&connection)?;\n    knowledge_vault::initialize(&connection, &config)?;\n",
)
replace_once(
    "crates/homeserver-service/src/app.rs",
    "    let router = http::secure(http::router(state.clone()).merge(cloud_connector::router(state)));",
    "    let router = http::secure(\n        http::router(state.clone())\n            .merge(cloud_connector::router(state.clone()))\n            .merge(knowledge_vault::router(state)),\n    );",
)

replace_once("src-tauri/src/lib.rs", "mod cloud;\n", "mod cloud;\nmod vault;\n")
replace_once(
    "src-tauri/src/lib.rs",
    "            homeserver_export_recovery_package\n",
    "            homeserver_export_recovery_package,\n            vault::homeserver_vault,\n            vault::homeserver_import_vault_document,\n            vault::homeserver_search_vault,\n            vault::homeserver_reindex_vault,\n            vault::homeserver_delete_vault_document\n",
)

replace_once(
    "src/main.js",
    "let updateStatus = null;\n",
    "let updateStatus = null;\nlet vaultSnapshot = null;\nlet vaultSearchResult = null;\n",
)
replace_once(
    "src/main.js",
    "  document.querySelectorAll(\"[data-save-setting]\").forEach((button) => button.addEventListener(\"click\", savePreferences));\n",
    "  document.querySelectorAll(\"[data-save-setting]\").forEach((button) => button.addEventListener(\"click\", savePreferences));\n  document.querySelector(\"#vault-import\")?.addEventListener(\"click\", importVaultDocument);\n  document.querySelector(\"#vault-search-form\")?.addEventListener(\"submit\", searchVault);\n  document.querySelector(\"#vault-reindex\")?.addEventListener(\"click\", reindexVault);\n  document.querySelectorAll(\"[data-vault-delete]\").forEach((button) => button.addEventListener(\"click\", deleteVaultDocument));\n",
)

knowledge_ui = r'''function renderKnowledge() {
  const documents = vaultSnapshot?.documents || [];
  const indexed = Number(vaultSnapshot?.indexed_count || 0);
  const attention = Number(vaultSnapshot?.changed_count || 0) + Number(vaultSnapshot?.missing_count || 0) + Number(vaultSnapshot?.failed_count || 0);
  const lastIndexed = vaultSnapshot?.last_indexed_at_utc;
  return `${pageHeader("Knowledge Vault", "Your private, searchable knowledge workspace. Imported content stays on this HomeServer.", `<button id="vault-reindex" class="button secondary" type="button" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>${icon("refresh", 16)}Reindex</button><button id="vault-import" class="button primary" type="button" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>${icon("upload", 16)}Import Document</button>`)}
    <form id="vault-search-form" class="toolbar-row"><label class="filter-search wide">${icon("search", 17)}<input id="vault-search-query" type="search" maxlength="200" placeholder="Search your knowledge vault..." value="${escapeHtml(vaultSearchResult?.query || "")}" required></label><button class="button secondary" type="submit" ${busy || !indexed ? "disabled" : ""}>Search</button><span class="planned-label">Local only · TXT MD CSV JSON LOG</span></form>
    <section class="metrics six-up">
      ${metricCard("vault", "Indexed Items", String(indexed), attention ? `${attention} need attention` : "Managed local documents", attention ? "amber" : "blue")}
      ${metricCard("file", "Managed Documents", String(documents.length), "Copied into protected storage", "purple")}
      ${metricCard("storage", "Storage Used", formatBytes(vaultSnapshot?.total_size_bytes || 0), "Vault document copies", "blue")}
      ${metricCard("activity", "Search Index", indexed ? "Ready" : "Empty", "Bounded local text index", indexed ? "green" : "gray")}
      ${metricCard("backup", "Last Indexing", lastIndexed ? relativeDate(lastIndexed) : "Not yet", lastIndexed ? formatDate(lastIndexed) : "Import a supported document", "gray")}
      ${metricCard("shield", "Privacy", vaultSnapshot?.local_only === false ? "Review" : "Local", "No cloud content sync", "green")}
    </section>
    <section class="knowledge-grid">
      <article class="panel vault-summary"><div class="panel-title"><div><h2>Vault Summary</h2></div></div><div class="storage-layout">${donut(documents.length ? Math.min(100, indexed / documents.length * 100) : 0, `${indexed}/${documents.length}`, "indexed", attention ? "amber" : "blue")}<ul class="legend"><li><i class="blue"></i><span>Indexed</span><strong>${indexed}</strong></li><li><i class="amber"></i><span>Changed</span><strong>${Number(vaultSnapshot?.changed_count || 0)}</strong></li><li><i class="red"></i><span>Missing / failed</span><strong>${Number(vaultSnapshot?.missing_count || 0) + Number(vaultSnapshot?.failed_count || 0)}</strong></li><li><i class="purple"></i><span>Supported types</span><strong>${vaultSnapshot?.supported_extensions?.length || 5}</strong></li></ul></div><button id="vault-reindex" class="text-button" type="button" ${busy || !documents.length ? "disabled" : ""}>Check managed files ${icon("arrow", 13)}</button></article>
      <article class="panel indexed-content"><div class="panel-title"><div><h2>Indexed Content</h2></div></div><div class="content-type-list">${contentType("file", "Text & logs", String(documents.filter((document) => ["text/plain", "text/markdown"].includes(document.content_type)).length), "blue")}${contentType("file", "CSV Files", String(documents.filter((document) => document.content_type === "text/csv").length), "green")}${contentType("storage", "JSON Data", String(documents.filter((document) => document.content_type === "application/json").length), "purple")}${contentType("activity", "Needs Attention", String(attention), attention ? "amber" : "gray")}</div><span class="planned-banner">PDF, OCR, embeddings, and semantic search are deferred to later Phase 4 scopes.</span></article>
      <article class="panel search-preview"><div class="panel-title"><div><h2>Search Results</h2></div><span>${vaultSearchResult ? `${vaultSearchResult.hits?.length || 0} matches` : "Run a local search"}</span></div>${renderVaultSearchResults()}</article>
      <article class="panel recent-sources"><div class="panel-title"><div><h2>Managed Documents</h2></div><span>${documents.length} records</span></div><div class="source-list">${documents.length ? documents.slice(0, 20).map(vaultDocumentRow).join("") : `<div class="empty-search compact">${icon("vault", 30)}<strong>No documents imported</strong><p>Import an approved UTF-8 text document to create the first local index.</p></div>`}</div></article>
      <article class="panel indexing-status"><div class="panel-title"><div><h2>Indexing Status</h2></div></div><div class="indexing-layout">${donut(documents.length ? indexed / documents.length * 100 : 0, documents.length ? `${Math.round(indexed / documents.length * 100)}%` : "0%", "indexed", attention ? "amber" : "blue")}<dl class="detail-list"><div><dt>Indexed</dt><dd>${indexed}</dd></div><div><dt>Changed</dt><dd>${Number(vaultSnapshot?.changed_count || 0)}</dd></div><div><dt>Missing</dt><dd>${Number(vaultSnapshot?.missing_count || 0)}</dd></div><div><dt>Failed</dt><dd>${Number(vaultSnapshot?.failed_count || 0)}</dd></div></dl></div></article>
      <article class="panel processing-queue"><div class="panel-title"><div><h2>Processing Boundary</h2></div></div><div class="empty-search compact">${icon("shield", 30)}<strong>Local managed storage</strong><p>Imports are copied into HomeServer storage. Source files remain unchanged and no content is synchronized to cloud services.</p></div></article>
    </section>
    <div class="privacy-banner">${icon("shield", 20)}<div><strong>Your document content remains local</strong><span>The service accepts document bytes from the trusted Control Center, never caller-supplied source paths.</span></div><button class="text-button" data-page="system">Review security ${icon("arrow", 13)}</button></div>`;
}

function renderVaultSearchResults() {
  if (!vaultSearchResult) return `<div class="empty-search">${icon("search", 34)}<strong>Search managed documents</strong><p>Search the bounded local text index without sending content to a cloud service.</p></div>`;
  const hits = vaultSearchResult.hits || [];
  if (!hits.length) return `<div class="empty-search">${icon("search", 34)}<strong>No matching documents</strong><p>Try another phrase or reindex the managed files.</p></div>`;
  return `<div class="source-list">${hits.map((hit) => `<div><div class="app-icon tone-blue">${icon("file", 18)}</div><span><strong>${escapeHtml(hit.document.title)}</strong><small>${escapeHtml(hit.snippet)}</small></span><em>Score ${Number(hit.score || 0)}</em>${badge(humanize(hit.document.state), hit.document.state === "indexed" ? "healthy" : "warning")}</div>`).join("")}</div>`;
}

function vaultDocumentRow(document) {
  return `<div><div class="app-icon tone-${document.state === "indexed" ? "blue" : "amber"}">${icon("file", 18)}</div><span><strong>${escapeHtml(document.title)}</strong><small>${escapeHtml(document.file_name)} · ${formatBytes(document.size_bytes)} · ${relativeDate(document.indexed_at_utc)}</small></span><em>${escapeHtml(document.content_type)}</em>${badge(humanize(document.state), document.state === "indexed" ? "healthy" : "warning")}<button class="icon-button danger" type="button" data-vault-delete="${escapeHtml(document.document_id)}" title="Delete managed copy">${icon("trash", 16)}</button></div>`;
}'''
replace_regex(
    "src/main.js",
    r"function renderKnowledge\(\) \{.*?\n\}\n\nfunction contentType",
    knowledge_ui + "\n\nfunction contentType",
)

vault_actions = r'''
async function importVaultDocument() {
  await withBusy(async () => {
    const result = await invoke("homeserver_import_vault_document", { tags: [] });
    if (!result) return null;
    vaultSearchResult = null;
    return { kind: result.affected ? "success" : "info", message: result.message };
  });
}

async function searchVault(event) {
  event.preventDefault();
  const query = document.querySelector("#vault-search-query")?.value?.trim() || "";
  if (!query) return;
  busy = true;
  notice = null;
  render();
  try {
    vaultSearchResult = await invoke("homeserver_search_vault", { query });
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    render();
  }
}

async function reindexVault() {
  await withBusy(async () => {
    const result = await invoke("homeserver_reindex_vault");
    vaultSearchResult = null;
    return { kind: "success", message: result.message };
  });
}

async function deleteVaultDocument(event) {
  const documentId = event.currentTarget.dataset.vaultDelete;
  const confirmation = window.prompt("Type DELETE to remove the HomeServer-managed copy and its local index. The source file will not be changed:");
  if (confirmation !== "DELETE") return;
  await withBusy(async () => {
    const result = await invoke("homeserver_delete_vault_document", { documentId, confirmation });
    vaultSearchResult = null;
    return { kind: "success", message: result.message };
  });
}

'''
replace_once("src/main.js", "async function loadAll(clearNotice = true) {\n", vault_actions + "async function loadAll(clearNotice = true) {\n")
replace_once(
    "src/main.js",
    "  const results = await Promise.allSettled([invoke(\"homeserver_status\"), invoke(\"homeserver_cloud_status\"), invoke(\"homeserver_backups\"), invoke(\"homeserver_updates\")]);",
    "  const results = await Promise.allSettled([invoke(\"homeserver_status\"), invoke(\"homeserver_cloud_status\"), invoke(\"homeserver_backups\"), invoke(\"homeserver_updates\"), invoke(\"homeserver_vault\")]);",
)
replace_once(
    "src/main.js",
    "    updateStatus = null;\n",
    "    updateStatus = null;\n    vaultSnapshot = null;\n",
)
replace_once(
    "src/main.js",
    "  updateStatus = results[3].status === \"fulfilled\" ? results[3].value : null;\n",
    "  updateStatus = results[3].status === \"fulfilled\" ? results[3].value : null;\n  vaultSnapshot = results[4].status === \"fulfilled\" ? results[4].value : null;\n",
)

replace_once(
    "README.md",
    "- Phase 3B foundation: signed update verification, application, health validation, and rollback.\n",
    "- Phase 3B foundation: signed update verification, application, health validation, and rollback.\n- Phase 4A foundation: managed local text-document import, indexing, search, change detection, and deletion.\n",
)
replace_once(
    "README.md",
    "Knowledge Vault, local model management, MCP runtime, and broader Linux/NAS deployment remain future phases and are not represented as complete.",
    "PDF/OCR and semantic Knowledge Vault indexing, local model management, MCP runtime, and broader Linux/NAS deployment remain future phases and are not represented as complete.",
)

print("Phase 4A Knowledge Vault source changes applied.")
