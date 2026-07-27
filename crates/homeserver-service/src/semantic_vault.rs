use crate::{model_center, AppState};
use anyhow::{ensure, Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, collections::HashMap, sync::Arc};
use uuid::Uuid;

const SEMANTIC_MIGRATION: &str =
    include_str!("../../../database/migrations/0007_semantic_vault.sql");
const SEMANTIC_MIGRATION_KEY: &str = "0007_semantic_vault";
const MAX_QUERY_CHARS: usize = 200;
const MAX_SEARCH_RESULTS: u32 = 50;
const MAX_DOCUMENTS_PER_REBUILD: usize = 200;
const MAX_SEMANTIC_CHARS_PER_DOCUMENT: usize = 512_000;
const CHUNK_TARGET_CHARS: usize = 1_200;
const CHUNK_OVERLAP_CHARS: usize = 160;
const MAX_CHUNKS_PER_DOCUMENT: usize = 512;
const EMBEDDING_BATCH_SIZE: usize = 8;
const MAX_SEARCH_CHUNKS: usize = 5_000;
const MAX_EMBEDDING_DIMENSIONS: usize = 4_096;
const OPERATION_COLUMNS: &str = "operation_id,operation_type,state,embedding_model,status_message,processed_documents,total_documents,processed_chunks,failed_documents,failure_code,created_at_utc,updated_at_utc,completed_at_utc";

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticOperation {
    pub operation_id: String,
    pub operation_type: String,
    pub state: String,
    pub embedding_model: String,
    pub status_message: String,
    pub processed_documents: u64,
    pub total_documents: u64,
    pub processed_chunks: u64,
    pub failed_documents: u64,
    pub failure_code: Option<String>,
    pub created_at_utc: DateTime<Utc>,
    pub updated_at_utc: DateTime<Utc>,
    pub completed_at_utc: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticVaultSnapshot {
    pub default_embedding_model: Option<String>,
    pub state: String,
    pub ready_documents: u64,
    pub stale_documents: u64,
    pub failed_documents: u64,
    pub chunk_count: u64,
    pub embedding_dimensions: u32,
    pub last_embedded_at_utc: Option<DateTime<Utc>>,
    pub latest_operation: Option<SemanticOperation>,
    pub local_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct SemanticRebuildRequest {
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SemanticActionResult {
    pub accepted: bool,
    pub operation: SemanticOperation,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct SemanticSearchRequest {
    pub query: String,
    pub limit: Option<u32>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticSearchHit {
    pub document_id: String,
    pub title: String,
    pub file_name: String,
    pub content_type: String,
    pub snippet: String,
    pub citation: String,
    pub chunk_ordinal: Option<u32>,
    pub page_number: Option<u32>,
    pub keyword_score: u32,
    pub semantic_score: f32,
    pub combined_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SemanticSearchResult {
    pub query: String,
    pub mode: String,
    pub embedding_model: Option<String>,
    pub semantic_available: bool,
    pub hits: Vec<SemanticSearchHit>,
}

#[derive(Debug, Clone)]
struct SourceDocument {
    document_id: String,
    title: String,
    file_name: String,
    content_type: String,
    source_sha256: String,
    indexed_text: String,
}

#[derive(Debug, Clone)]
struct StoredChunk {
    document_id: String,
    title: String,
    file_name: String,
    content_type: String,
    chunk_ordinal: u32,
    page_number: Option<u32>,
    chunk_text: String,
    embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
struct RankedDocument {
    document_id: String,
    title: String,
    file_name: String,
    content_type: String,
    snippet: String,
    citation: String,
    chunk_ordinal: Option<u32>,
    page_number: Option<u32>,
    keyword_score: u32,
    semantic_score: f32,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(SEMANTIC_MIGRATION)?;
    connection.execute(
        "UPDATE vault_semantic_operations SET state='interrupted',status_message='Interrupted by HomeServer restart',failure_code='service_restarted',completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state IN ('pending','running')",
        [],
    )?;
    connection.execute(
        "DELETE FROM vault_semantic_operations WHERE operation_id NOT IN (SELECT operation_id FROM vault_semantic_operations ORDER BY updated_at_utc DESC,operation_id DESC LIMIT 200) AND state NOT IN ('pending','running')",
        [],
    )?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![SEMANTIC_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "semantic Knowledge Vault migration is not registered exactly once"
    );
    for table in [
        "vault_semantic_documents",
        "vault_semantic_chunks",
        "vault_semantic_operations",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/vault/semantic", get(semantic_snapshot))
        .route("/v1/vault/semantic/rebuild", post(rebuild_semantic_index))
        .route("/v1/vault/semantic/search", post(search_semantic_index))
        .with_state(state)
}

async fn semantic_snapshot(State(state): State<Arc<AppState>>) -> ApiResult<SemanticVaultSnapshot> {
    tokio::task::spawn_blocking(move || snapshot(&state))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("semantic_snapshot_failed", error))
}

async fn rebuild_semantic_index(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SemanticRebuildRequest>,
) -> ApiResult<SemanticActionResult> {
    let force = request.force;
    let state_for_begin = state.clone();
    let (operation, started) = tokio::task::spawn_blocking(move || {
        begin_rebuild(&state_for_begin, force)
    })
    .await
    .map_err(task_error)?
    .map_err(|error| action_error("semantic_rebuild_rejected", error))?;

    if started {
        let task_state = state.clone();
        let task_operation = operation.clone();
        tokio::spawn(async move {
            if let Err(error) = run_rebuild(task_state.clone(), task_operation.clone(), force).await {
                let failure_code = public_failure_code(&error);
                let operation_id = task_operation.operation_id.clone();
                let state_for_finish = task_state.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    finish_operation(
                        &state_for_finish,
                        &operation_id,
                        "failed",
                        "Semantic indexing failed",
                        Some(&failure_code),
                    )
                })
                .await;
                tracing::warn!(?error, "local semantic Knowledge Vault rebuild failed");
            }
        });
    }

    Ok(Json(SemanticActionResult {
        accepted: started,
        operation,
        message: if started {
            "Started local semantic Knowledge Vault indexing.".to_owned()
        } else {
            "A semantic Knowledge Vault rebuild is already active.".to_owned()
        },
    }))
}

async fn search_semantic_index(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SemanticSearchRequest>,
) -> ApiResult<SemanticSearchResult> {
    semantic_search(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("semantic_search_failed", error))
}

fn snapshot(state: &AppState) -> Result<SemanticVaultSnapshot> {
    let connection = state.connection()?;
    let default_embedding_model = model_center::configured_embedding_model_from_connection(&connection)?;
    refresh_stale_states(&connection, default_embedding_model.as_deref())?;
    let counts = connection.query_row(
        "SELECT COALESCE(SUM(CASE WHEN d.state='indexed' AND s.state='ready' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN d.state='indexed' AND (s.document_id IS NULL OR s.state IN ('pending','stale')) THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN s.state='failed' THEN 1 ELSE 0 END),0),COALESCE(SUM(CASE WHEN s.state='ready' THEN s.chunk_count ELSE 0 END),0),COALESCE(MAX(CASE WHEN s.state='ready' THEN s.dimensions ELSE 0 END),0),MAX(CASE WHEN s.state='ready' THEN s.embedded_at_utc END) FROM vault_documents d LEFT JOIN vault_semantic_documents s ON s.document_id=d.document_id",
        [],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        },
    )?;
    let latest_operation = latest_operation(&connection)?;
    let state_label = if latest_operation
        .as_ref()
        .is_some_and(|operation| matches!(operation.state.as_str(), "pending" | "running"))
    {
        "indexing"
    } else if default_embedding_model.is_none() {
        "not_configured"
    } else if counts.1 > 0 || counts.2 > 0 {
        "attention"
    } else if counts.0 > 0 {
        "ready"
    } else {
        "empty"
    };
    Ok(SemanticVaultSnapshot {
        default_embedding_model,
        state: state_label.to_owned(),
        ready_documents: counts.0.max(0) as u64,
        stale_documents: counts.1.max(0) as u64,
        failed_documents: counts.2.max(0) as u64,
        chunk_count: counts.3.max(0) as u64,
        embedding_dimensions: counts.4.clamp(0, i64::from(u32::MAX)) as u32,
        last_embedded_at_utc: counts.5.map(parse_utc).transpose()?,
        latest_operation,
        local_only: true,
    })
}

fn refresh_stale_states(connection: &Connection, model: Option<&str>) -> Result<()> {
    connection.execute(
        "UPDATE vault_semantic_documents SET state='stale',failure_code='source_changed',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE document_id IN (SELECT s.document_id FROM vault_semantic_documents s JOIN vault_documents d ON d.document_id=s.document_id WHERE d.state <> 'indexed' OR d.sha256 <> s.source_sha256)",
        [],
    )?;
    if let Some(model) = model {
        connection.execute(
            "UPDATE vault_semantic_documents SET state='stale',failure_code='embedding_model_changed',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE embedding_model IS NOT NULL AND embedding_model <> ?1 AND state='ready'",
            params![model],
        )?;
    }
    Ok(())
}

fn begin_rebuild(state: &AppState, force: bool) -> Result<(SemanticOperation, bool)> {
    let connection = state.connection()?;
    let model = model_center::configured_embedding_model_from_connection(&connection)?
        .context("assign an installed default embedding model in Model Center first")?;
    model_center::validate_embedding_model(&model)?;
    if let Some(operation) = active_operation(&connection)? {
        return Ok((operation, false));
    }
    let document_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM vault_documents WHERE state='indexed' AND length(indexed_text) > 0",
        [],
        |row| row.get(0),
    )?;
    ensure!(document_count > 0, "import and index a document before building semantic search");
    ensure!(
        document_count as usize <= MAX_DOCUMENTS_PER_REBUILD,
        "semantic rebuild exceeds the 200 document safety limit"
    );
    let operation_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO vault_semantic_operations (operation_id,operation_type,state,embedding_model,status_message,total_documents) VALUES (?1,'rebuild','pending',?2,?3,?4)",
        params![
            operation_id,
            model,
            if force { "Full semantic rebuild queued" } else { "Semantic indexing queued" },
            document_count,
        ],
    )?;
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.semantic_rebuild_started','Local semantic Knowledge Vault indexing started',json_object('operation_id',?1,'documents',?2,'force',?3))",
        params![operation_id, document_count, if force { 1 } else { 0 }],
    )?;
    Ok((operation_by_id(&connection, &operation_id)?, true))
}

async fn run_rebuild(
    state: Arc<AppState>,
    operation: SemanticOperation,
    force: bool,
) -> Result<()> {
    let operation_id = operation.operation_id.clone();
    let model = operation.embedding_model.clone();
    let state_for_documents = state.clone();
    let documents = tokio::task::spawn_blocking(move || source_documents(&state_for_documents))
        .await
        .context("semantic document inventory task failed")??;
    let state_for_running = state.clone();
    let running_operation_id = operation_id.clone();
    tokio::task::spawn_blocking(move || {
        set_operation_running(&state_for_running, &running_operation_id)
    })
    .await
    .context("semantic operation start task failed")??;

    let mut processed_documents = 0_u64;
    let mut processed_chunks = 0_u64;
    let mut failed_documents = 0_u64;

    for document in documents {
        let state_for_current = state.clone();
        let document_id_for_current = document.document_id.clone();
        let model_for_current = model.clone();
        let source_sha_for_current = document.source_sha256.clone();
        let already_ready = tokio::task::spawn_blocking(move || {
            semantic_document_is_current(
                &state_for_current,
                &document_id_for_current,
                &model_for_current,
                &source_sha_for_current,
            )
        })
        .await
        .context("semantic current-state task failed")??;

        let result = if already_ready && !force {
            Ok(0_u64)
        } else {
            index_document(state.clone(), &model, document.clone()).await
        };
        match result {
            Ok(chunk_count) => {
                processed_chunks = processed_chunks.saturating_add(chunk_count);
            }
            Err(error) => {
                failed_documents = failed_documents.saturating_add(1);
                let failure_code = public_failure_code(&error);
                let state_for_failure = state.clone();
                let document_id = document.document_id.clone();
                let source_sha256 = document.source_sha256.clone();
                let model_name = model.clone();
                tokio::task::spawn_blocking(move || {
                    mark_semantic_document_failed(
                        &state_for_failure,
                        &document_id,
                        &source_sha256,
                        &model_name,
                        &failure_code,
                    )
                })
                .await
                .context("semantic document failure task failed")??;
                tracing::warn!(document_id = %document.document_id, ?error, "semantic document indexing failed");
            }
        }
        processed_documents = processed_documents.saturating_add(1);
        let state_for_progress = state.clone();
        let operation_id_for_progress = operation_id.clone();
        tokio::task::spawn_blocking(move || {
            update_operation_progress(
                &state_for_progress,
                &operation_id_for_progress,
                processed_documents,
                processed_chunks,
                failed_documents,
            )
        })
        .await
        .context("semantic operation progress task failed")??;
    }

    let message = if failed_documents == 0 {
        "Semantic Knowledge Vault index is ready"
    } else {
        "Semantic indexing completed with document errors"
    };
    let state_for_finish = state.clone();
    let operation_id_for_finish = operation_id.clone();
    tokio::task::spawn_blocking(move || {
        finish_operation(
            &state_for_finish,
            &operation_id_for_finish,
            "completed",
            message,
            None,
        )
    })
    .await
    .context("semantic operation completion task failed")??;
    Ok(())
}

async fn index_document(
    state: Arc<AppState>,
    model: &str,
    document: SourceDocument,
) -> Result<u64> {
    let chunks = chunk_text(&document.indexed_text);
    ensure!(!chunks.is_empty(), "document produced no semantic chunks");
    ensure!(
        chunks.len() <= MAX_CHUNKS_PER_DOCUMENT,
        "document exceeds the semantic chunk safety limit"
    );

    let state_for_mark = state.clone();
    let document_id_for_mark = document.document_id.clone();
    let source_sha_for_mark = document.source_sha256.clone();
    let model_for_mark = model.to_owned();
    tokio::task::spawn_blocking(move || {
        mark_semantic_document_indexing(
            &state_for_mark,
            &document_id_for_mark,
            &source_sha_for_mark,
            &model_for_mark,
        )
    })
    .await
    .context("semantic document state task failed")??;

    let mut embeddings = Vec::with_capacity(chunks.len());
    for batch in chunks.chunks(EMBEDDING_BATCH_SIZE) {
        let values = model_center::embed_texts(
            state.clone(),
            model.to_owned(),
            batch.to_vec(),
        )
        .await?;
        ensure!(
            values.len() == batch.len(),
            "embedding runtime returned an unexpected batch size"
        );
        embeddings.extend(values);
    }
    let dimensions = embeddings.first().map(Vec::len).unwrap_or(0);
    ensure!(dimensions > 0, "embedding runtime returned an empty vector");
    ensure!(
        dimensions <= MAX_EMBEDDING_DIMENSIONS,
        "embedding vector exceeds the dimension safety limit"
    );
    ensure!(
        embeddings.iter().all(|vector| {
            vector.len() == dimensions && vector.iter().all(|value| value.is_finite())
        }),
        "embedding vectors are inconsistent or non-finite"
    );

    let state_for_store = state.clone();
    let model_for_store = model.to_owned();
    let chunk_count = chunks.len() as u64;
    tokio::task::spawn_blocking(move || {
        store_document_embeddings(
            &state_for_store,
            &document,
            &model_for_store,
            chunks,
            embeddings,
            dimensions,
        )
    })
    .await
    .context("semantic embedding storage task failed")??;
    Ok(chunk_count)
}

async fn semantic_search(
    state: Arc<AppState>,
    request: SemanticSearchRequest,
) -> Result<SemanticSearchResult> {
    let query = request.query.trim().to_owned();
    ensure!(!query.is_empty(), "search query is required");
    ensure!(
        query.chars().count() <= MAX_QUERY_CHARS,
        "search query exceeds the 200 character limit"
    );
    let mode = request.mode.as_deref().unwrap_or("hybrid").trim().to_ascii_lowercase();
    ensure!(
        matches!(mode.as_str(), "keyword" | "semantic" | "hybrid"),
        "search mode must be keyword, semantic, or hybrid"
    );
    let limit = request.limit.unwrap_or(20).clamp(1, MAX_SEARCH_RESULTS) as usize;

    let state_for_inventory = state.clone();
    let query_for_inventory = query.clone();
    let (model, keyword_documents, chunks) = tokio::task::spawn_blocking(move || {
        search_inventory(&state_for_inventory, &query_for_inventory)
    })
    .await
    .context("semantic search inventory task failed")??;

    let semantic_available = model.is_some() && !chunks.is_empty();
    if mode == "semantic" {
        ensure!(
            semantic_available,
            "semantic index is not ready; assign an embedding model and build the index"
        );
    }

    let query_embedding = if mode != "keyword" && semantic_available {
        let model_name = model.clone().context("semantic embedding model is missing")?;
        let mut result = model_center::embed_texts(
            state.clone(),
            model_name,
            vec![query.clone()],
        )
        .await?;
        Some(result.pop().context("embedding runtime returned no query vector")?)
    } else {
        None
    };

    let hits = rank_results(
        &query,
        &mode,
        keyword_documents,
        chunks,
        query_embedding.as_deref(),
        limit,
    )?;
    Ok(SemanticSearchResult {
        query,
        mode,
        embedding_model: model,
        semantic_available,
        hits,
    })
}

fn search_inventory(
    state: &AppState,
    query: &str,
) -> Result<(Option<String>, Vec<SourceDocument>, Vec<StoredChunk>)> {
    let connection = state.connection()?;
    let model = model_center::configured_embedding_model_from_connection(&connection)?;
    let mut statement = connection.prepare(
        "SELECT document_id,title,file_name,content_type,sha256,indexed_text FROM vault_documents WHERE state='indexed' ORDER BY updated_at_utc DESC LIMIT 200",
    )?;
    let keyword_documents = statement
        .query_map([], source_document_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    let Some(model_name) = model.as_deref() else {
        return Ok((model, keyword_documents, Vec::new()));
    };
    let mut chunk_statement = connection.prepare(
        "SELECT c.document_id,d.title,d.file_name,d.content_type,c.chunk_ordinal,c.page_number,c.chunk_text,c.embedding_json,c.dimensions FROM vault_semantic_chunks c JOIN vault_semantic_documents s ON s.document_id=c.document_id JOIN vault_documents d ON d.document_id=c.document_id WHERE s.state='ready' AND s.embedding_model=?1 AND c.embedding_model=?1 AND d.state='indexed' ORDER BY c.document_id,c.chunk_ordinal LIMIT ?2",
    )?;
    let chunks = chunk_statement
        .query_map(params![model_name, MAX_SEARCH_CHUNKS as i64], stored_chunk_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let _ = query;
    Ok((model, keyword_documents, chunks))
}

fn rank_results(
    query: &str,
    mode: &str,
    keyword_documents: Vec<SourceDocument>,
    chunks: Vec<StoredChunk>,
    query_embedding: Option<&[f32]>,
    limit: usize,
) -> Result<Vec<SemanticSearchHit>> {
    let semantic_used = query_embedding.is_some();
    let mut ranked: HashMap<String, RankedDocument> = HashMap::new();
    if mode != "semantic" {
        for document in keyword_documents {
            let keyword_score = match_count(&document.title, query)
                .saturating_mul(10)
                .saturating_add(match_count(&document.indexed_text, query));
            if keyword_score == 0 {
                continue;
            }
            ranked.insert(
                document.document_id.clone(),
                RankedDocument {
                    document_id: document.document_id,
                    title: document.title,
                    file_name: document.file_name.clone(),
                    content_type: document.content_type,
                    snippet: search_snippet(&document.indexed_text, query),
                    citation: document.file_name,
                    chunk_ordinal: None,
                    page_number: None,
                    keyword_score,
                    semantic_score: 0.0,
                },
            );
        }
    }

    if let Some(query_vector) = query_embedding {
        ensure!(!query_vector.is_empty(), "query embedding is empty");
        for chunk in chunks {
            if chunk.embedding.len() != query_vector.len() {
                continue;
            }
            let similarity = cosine_similarity(query_vector, &chunk.embedding);
            let entry = ranked.entry(chunk.document_id.clone()).or_insert_with(|| RankedDocument {
                document_id: chunk.document_id.clone(),
                title: chunk.title.clone(),
                file_name: chunk.file_name.clone(),
                content_type: chunk.content_type.clone(),
                snippet: semantic_snippet(&chunk.chunk_text),
                citation: citation(&chunk.file_name, chunk.page_number, chunk.chunk_ordinal),
                chunk_ordinal: Some(chunk.chunk_ordinal),
                page_number: chunk.page_number,
                keyword_score: 0,
                semantic_score: similarity,
            });
            if similarity > entry.semantic_score || entry.chunk_ordinal.is_none() {
                entry.semantic_score = similarity;
                entry.snippet = semantic_snippet(&chunk.chunk_text);
                entry.citation = citation(&chunk.file_name, chunk.page_number, chunk.chunk_ordinal);
                entry.chunk_ordinal = Some(chunk.chunk_ordinal);
                entry.page_number = chunk.page_number;
            }
        }
    }

    let mut values = ranked
        .into_values()
        .filter_map(|item| {
            let keyword = (item.keyword_score.min(50) as f32 / 50.0).clamp(0.0, 1.0);
            let semantic = if item.chunk_ordinal.is_some() {
                ((item.semantic_score + 1.0) / 2.0).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let combined = match mode {
                "keyword" => keyword,
                "semantic" => semantic,
                _ if semantic_used => semantic.mul_add(0.7, keyword * 0.3),
                _ => keyword,
            };
            if combined <= 0.0 {
                return None;
            }
            Some(SemanticSearchHit {
                document_id: item.document_id,
                title: item.title,
                file_name: item.file_name,
                content_type: item.content_type,
                snippet: item.snippet,
                citation: item.citation,
                chunk_ordinal: item.chunk_ordinal,
                page_number: item.page_number,
                keyword_score: item.keyword_score,
                semantic_score: item.semantic_score,
                combined_score: combined,
            })
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| {
        right
            .combined_score
            .partial_cmp(&left.combined_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.title.cmp(&right.title))
    });
    values.truncate(limit);
    Ok(values)
}

fn source_documents(state: &AppState) -> Result<Vec<SourceDocument>> {
    let connection = state.connection()?;
    let mut statement = connection.prepare(
        "SELECT document_id,title,file_name,content_type,sha256,indexed_text FROM vault_documents WHERE state='indexed' AND length(indexed_text) > 0 ORDER BY document_id LIMIT ?1",
    )?;
    let documents = statement
        .query_map(params![MAX_DOCUMENTS_PER_REBUILD as i64], source_document_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(documents)
}

fn source_document_from_row(row: &Row<'_>) -> rusqlite::Result<SourceDocument> {
    Ok(SourceDocument {
        document_id: row.get(0)?,
        title: row.get(1)?,
        file_name: row.get(2)?,
        content_type: row.get(3)?,
        source_sha256: row.get(4)?,
        indexed_text: row.get(5)?,
    })
}

fn stored_chunk_from_row(row: &Row<'_>) -> rusqlite::Result<StoredChunk> {
    let embedding_json = row.get::<_, String>(7)?;
    let embedding: Vec<f32> = serde_json::from_str(&embedding_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;
    let dimensions = row.get::<_, i64>(8)?.max(0) as usize;
    if embedding.len() != dimensions || embedding.len() > MAX_EMBEDDING_DIMENSIONS {
        return Err(rusqlite::Error::InvalidColumnType(
            8,
            "dimensions".to_owned(),
            rusqlite::types::Type::Integer,
        ));
    }
    Ok(StoredChunk {
        document_id: row.get(0)?,
        title: row.get(1)?,
        file_name: row.get(2)?,
        content_type: row.get(3)?,
        chunk_ordinal: row.get::<_, i64>(4)?.max(0) as u32,
        page_number: row
            .get::<_, Option<i64>>(5)?
            .map(|value| value.max(1) as u32),
        chunk_text: row.get(6)?,
        embedding,
    })
}

fn semantic_document_is_current(
    state: &AppState,
    document_id: &str,
    model: &str,
    source_sha256: &str,
) -> Result<bool> {
    let connection = state.connection()?;
    let current = connection
        .query_row(
            "SELECT state,embedding_model,source_sha256 FROM vault_semantic_documents WHERE document_id=?1",
            params![document_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(current.is_some_and(|(status, current_model, current_sha)| {
        status == "ready"
            && current_model.as_deref() == Some(model)
            && current_sha == source_sha256
    }))
}

fn mark_semantic_document_indexing(
    state: &AppState,
    document_id: &str,
    source_sha256: &str,
    model: &str,
) -> Result<()> {
    let connection = state.connection()?;
    connection.execute(
        "INSERT INTO vault_semantic_documents (document_id,state,embedding_model,source_sha256,chunk_count,dimensions,failure_code,embedded_at_utc,updated_at_utc) VALUES (?1,'indexing',?2,?3,0,0,NULL,NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(document_id) DO UPDATE SET state='indexing',embedding_model=excluded.embedding_model,source_sha256=excluded.source_sha256,chunk_count=0,dimensions=0,failure_code=NULL,embedded_at_utc=NULL,updated_at_utc=excluded.updated_at_utc",
        params![document_id, model, source_sha256],
    )?;
    Ok(())
}

fn mark_semantic_document_failed(
    state: &AppState,
    document_id: &str,
    source_sha256: &str,
    model: &str,
    failure_code: &str,
) -> Result<()> {
    let connection = state.connection()?;
    connection.execute(
        "INSERT INTO vault_semantic_documents (document_id,state,embedding_model,source_sha256,chunk_count,dimensions,failure_code,embedded_at_utc,updated_at_utc) VALUES (?1,'failed',?2,?3,0,0,?4,NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(document_id) DO UPDATE SET state='failed',embedding_model=excluded.embedding_model,source_sha256=excluded.source_sha256,chunk_count=0,dimensions=0,failure_code=excluded.failure_code,embedded_at_utc=NULL,updated_at_utc=excluded.updated_at_utc",
        params![document_id, model, source_sha256, failure_code],
    )?;
    Ok(())
}

fn store_document_embeddings(
    state: &AppState,
    document: &SourceDocument,
    model: &str,
    chunks: Vec<String>,
    embeddings: Vec<Vec<f32>>,
    dimensions: usize,
) -> Result<()> {
    ensure!(chunks.len() == embeddings.len(), "chunk and embedding counts differ");
    let mut connection = state.connection()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "DELETE FROM vault_semantic_chunks WHERE document_id=?1",
        params![document.document_id],
    )?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO vault_semantic_chunks (chunk_id,document_id,chunk_ordinal,page_number,chunk_text,chunk_sha256,embedding_model,dimensions,embedding_json) VALUES (?1,?2,?3,NULL,?4,?5,?6,?7,?8)",
        )?;
        for (ordinal, (chunk, embedding)) in chunks.into_iter().zip(embeddings).enumerate() {
            let chunk_sha256 = hex::encode(Sha256::digest(chunk.as_bytes()));
            let embedding_json = serde_json::to_string(&embedding)?;
            statement.execute(params![
                Uuid::new_v4().to_string(),
                document.document_id,
                ordinal as i64,
                chunk,
                chunk_sha256,
                model,
                dimensions as i64,
                embedding_json,
            ])?;
        }
    }
    transaction.execute(
        "INSERT INTO vault_semantic_documents (document_id,state,embedding_model,source_sha256,chunk_count,dimensions,failure_code,embedded_at_utc,updated_at_utc) VALUES (?1,'ready',?2,?3,?4,?5,NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(document_id) DO UPDATE SET state='ready',embedding_model=excluded.embedding_model,source_sha256=excluded.source_sha256,chunk_count=excluded.chunk_count,dimensions=excluded.dimensions,failure_code=NULL,embedded_at_utc=excluded.embedded_at_utc,updated_at_utc=excluded.updated_at_utc",
        params![
            document.document_id,
            model,
            document.source_sha256,
            embeddings_count(&transaction, &document.document_id)?,
            dimensions as i64,
        ],
    )?;
    transaction.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.semantic_document_ready','A local Knowledge Vault document semantic index was updated',json_object('document_id',?1,'model',?2,'dimensions',?3))",
        params![document.document_id, model, dimensions as i64],
    )?;
    transaction.commit()?;
    Ok(())
}

fn embeddings_count(connection: &Connection, document_id: &str) -> Result<i64> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM vault_semantic_chunks WHERE document_id=?1",
            params![document_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn active_operation(connection: &Connection) -> Result<Option<SemanticOperation>> {
    connection
        .query_row(
            &format!("SELECT {OPERATION_COLUMNS} FROM vault_semantic_operations WHERE state IN ('pending','running') ORDER BY created_at_utc DESC LIMIT 1"),
            [],
            operation_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn latest_operation(connection: &Connection) -> Result<Option<SemanticOperation>> {
    connection
        .query_row(
            &format!("SELECT {OPERATION_COLUMNS} FROM vault_semantic_operations ORDER BY updated_at_utc DESC,operation_id DESC LIMIT 1"),
            [],
            operation_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn operation_by_id(connection: &Connection, operation_id: &str) -> Result<SemanticOperation> {
    connection
        .query_row(
            &format!("SELECT {OPERATION_COLUMNS} FROM vault_semantic_operations WHERE operation_id=?1"),
            params![operation_id],
            operation_from_row,
        )
        .context("semantic Knowledge Vault operation was not found")
}

fn operation_from_row(row: &Row<'_>) -> rusqlite::Result<SemanticOperation> {
    Ok(SemanticOperation {
        operation_id: row.get(0)?,
        operation_type: row.get(1)?,
        state: row.get(2)?,
        embedding_model: row.get(3)?,
        status_message: row.get(4)?,
        processed_documents: row.get::<_, i64>(5)?.max(0) as u64,
        total_documents: row.get::<_, i64>(6)?.max(0) as u64,
        processed_chunks: row.get::<_, i64>(7)?.max(0) as u64,
        failed_documents: row.get::<_, i64>(8)?.max(0) as u64,
        failure_code: row.get(9)?,
        created_at_utc: parse_utc(row.get(10)?).map_err(to_sql_error)?,
        updated_at_utc: parse_utc(row.get(11)?).map_err(to_sql_error)?,
        completed_at_utc: row
            .get::<_, Option<String>>(12)?
            .map(parse_utc)
            .transpose()
            .map_err(to_sql_error)?,
    })
}

fn set_operation_running(state: &AppState, operation_id: &str) -> Result<()> {
    let connection = state.connection()?;
    connection.execute(
        "UPDATE vault_semantic_operations SET state='running',status_message='Embedding local Knowledge Vault documents',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?1",
        params![operation_id],
    )?;
    Ok(())
}

fn update_operation_progress(
    state: &AppState,
    operation_id: &str,
    processed_documents: u64,
    processed_chunks: u64,
    failed_documents: u64,
) -> Result<()> {
    let connection = state.connection()?;
    connection.execute(
        "UPDATE vault_semantic_operations SET processed_documents=?1,processed_chunks=?2,failed_documents=?3,status_message=?4,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?5",
        params![
            processed_documents as i64,
            processed_chunks as i64,
            failed_documents as i64,
            format!("Indexed {processed_documents} documents and {processed_chunks} chunks"),
            operation_id,
        ],
    )?;
    Ok(())
}

fn finish_operation(
    state: &AppState,
    operation_id: &str,
    state_value: &str,
    status_message: &str,
    failure_code: Option<&str>,
) -> Result<()> {
    let connection = state.connection()?;
    connection.execute(
        "UPDATE vault_semantic_operations SET state=?1,status_message=?2,failure_code=?3,completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?4",
        params![state_value, status_message, failure_code, operation_id],
    )?;
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('vault.semantic_rebuild_finished','Local semantic Knowledge Vault indexing finished',json_object('operation_id',?1,'state',?2))",
        params![operation_id, state_value],
    )?;
    Ok(())
}

fn chunk_text(text: &str) -> Vec<String> {
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .take(MAX_SEMANTIC_CHARS_PER_DOCUMENT)
        .collect::<String>();
    let chars = normalized.chars().collect::<Vec<_>>();
    let mut chunks = Vec::new();
    let mut start = 0_usize;
    while start < chars.len() && chunks.len() < MAX_CHUNKS_PER_DOCUMENT {
        let target_end = (start + CHUNK_TARGET_CHARS).min(chars.len());
        let minimum_end = (start + CHUNK_TARGET_CHARS / 2).min(target_end);
        let mut end = target_end;
        if target_end < chars.len() {
            if let Some(offset) = chars[minimum_end..target_end]
                .iter()
                .rposition(|value| matches!(value, '\n' | '.' | '!' | '?' | ';' | ' '))
            {
                end = minimum_end + offset + 1;
            }
        }
        if end <= start {
            end = target_end.max(start + 1).min(chars.len());
        }
        let chunk = chars[start..end]
            .iter()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        if end >= chars.len() {
            break;
        }
        let next = end.saturating_sub(CHUNK_OVERLAP_CHARS);
        start = if next > start { next } else { end };
    }
    chunks
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
    let chars = text.chars().collect::<Vec<_>>();
    let start = char_position.saturating_sub(90).min(chars.len());
    let end = (char_position + query.chars().count() + 180).min(chars.len());
    let mut snippet = chars[start..end]
        .iter()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if start > 0 {
        snippet.insert(0, '…');
    }
    if end < chars.len() {
        snippet.push('…');
    }
    snippet
}

fn semantic_snippet(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut snippet = compact.chars().take(360).collect::<String>();
    if compact.chars().count() > 360 {
        snippet.push('…');
    }
    snippet
}

fn citation(file_name: &str, page_number: Option<u32>, chunk_ordinal: u32) -> String {
    match page_number {
        Some(page) => format!("{file_name} · page {page}"),
        None => format!("{file_name} · section {}", chunk_ordinal + 1),
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return -1.0;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (&a, &b) in left.iter().zip(right) {
        let a = f64::from(a);
        let b = f64::from(b);
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm <= f64::EPSILON || right_norm <= f64::EPSILON {
        return -1.0;
    }
    (dot / (left_norm.sqrt() * right_norm.sqrt())).clamp(-1.0, 1.0) as f32
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

fn public_failure_code(error: &anyhow::Error) -> String {
    let text = error.to_string().to_lowercase();
    if text.contains("ollama") || text.contains("embedding runtime") {
        "embedding_runtime_unavailable"
    } else if text.contains("model") {
        "embedding_model_invalid"
    } else if text.contains("chunk") || text.contains("dimension") || text.contains("vector") {
        "embedding_payload_invalid"
    } else if text.contains("limit") {
        "semantic_limit_exceeded"
    } else {
        "semantic_index_failed"
    }
    .to_owned()
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "semantic_task_failed",
        anyhow::anyhow!(error),
    )
}

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    let text = error.to_string().to_lowercase();
    let status = if text.contains("required")
        || text.contains("limit")
        || text.contains("must be")
        || text.contains("not ready")
        || text.contains("assign")
        || text.contains("import")
        || text.contains("unexpected")
        || text.contains("empty")
    {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if text.contains("ollama") || text.contains("runtime") {
        StatusCode::CONFLICT
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
    fn chunking_is_bounded_overlapping_and_deterministic() {
        let text = "alpha beta gamma delta. ".repeat(1_000);
        let chunks = chunk_text(&text);
        assert!(!chunks.is_empty());
        assert!(chunks.len() <= MAX_CHUNKS_PER_DOCUMENT);
        assert!(chunks.iter().all(|chunk| chunk.chars().count() <= CHUNK_TARGET_CHARS + 1));
        assert_eq!(chunks, chunk_text(&text));
    }

    #[test]
    fn cosine_similarity_ranks_identical_vectors_first() {
        let identical = cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]);
        let orthogonal = cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]);
        assert!(identical > orthogonal);
        assert!((identical - 1.0).abs() < 0.0001);
    }

    #[test]
    fn citations_support_future_page_level_extraction() {
        assert_eq!(citation("policy.pdf", Some(3), 4), "policy.pdf · page 3");
        assert_eq!(citation("policy.md", None, 1), "policy.md · section 2");
    }
}
