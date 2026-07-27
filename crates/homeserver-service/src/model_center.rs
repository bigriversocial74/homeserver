use crate::AppState;
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest::{redirect::Policy, Client, Response};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashSet,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

const MODEL_MIGRATION: &str = include_str!("../../../database/migrations/0006_model_center.sql");
const MODEL_MIGRATION_KEY: &str = "0006_model_center";
const OLLAMA_API_BASE: &str = "http://127.0.0.1:11434";
const MAX_OLLAMA_JSON_BYTES: usize = 4 * 1024 * 1024;
const MAX_PULL_STREAM_BYTES: usize = 64 * 1024 * 1024;
const MAX_PULL_LINE_BYTES: usize = 128 * 1024;
const MAX_TEST_PROMPT_CHARS: usize = 500;
const MAX_TEST_RESPONSE_CHARS: usize = 4_000;
const MAX_EMBED_INPUTS: usize = 8;
const MAX_EMBED_INPUT_CHARS: usize = 2_000;
const MAX_EMBED_TOTAL_CHARS: usize = 12_000;
const MAX_EMBEDDING_DIMENSIONS: usize = 4_096;
const MAX_OPERATION_HISTORY: i64 = 200;
const OPERATION_COLUMNS: &str = "operation_id,model_name,operation_type,state,status_message,completed_bytes,total_bytes,failure_code,created_at_utc,updated_at_utc,completed_at_utc";

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRuntime {
    pub provider: String,
    pub api_url: String,
    pub state: String,
    pub version: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwareSnapshot {
    pub logical_cpu_count: u32,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub free_disk_bytes: u64,
    pub gpu_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalModel {
    pub name: String,
    pub size_bytes: u64,
    pub digest: Option<String>,
    pub modified_at_utc: Option<String>,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
    pub running: bool,
    pub size_vram_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogModel {
    pub model: String,
    pub display_name: String,
    pub purpose: String,
    pub estimated_size_bytes: u64,
    pub minimum_memory_bytes: u64,
    pub supports_chat: bool,
    pub supports_embeddings: bool,
    pub installed: bool,
    pub running: bool,
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelOperation {
    pub operation_id: String,
    pub model_name: String,
    pub operation_type: String,
    pub state: String,
    pub status_message: String,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub failure_code: Option<String>,
    pub created_at_utc: DateTime<Utc>,
    pub updated_at_utc: DateTime<Utc>,
    pub completed_at_utc: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSettings {
    pub default_chat_model: Option<String>,
    pub default_embedding_model: Option<String>,
    pub context_size: u32,
    pub test_timeout_seconds: u64,
    pub max_download_gb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCenterSnapshot {
    pub runtime: ModelRuntime,
    pub hardware: HardwareSnapshot,
    pub installed_models: Vec<LocalModel>,
    pub catalog: Vec<CatalogModel>,
    pub operations: Vec<ModelOperation>,
    pub settings: ModelSettings,
    pub local_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct ModelNameRequest {
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteModelRequest {
    pub model: String,
    pub confirmation: String,
}

#[derive(Debug, Deserialize)]
pub struct TestModelRequest {
    pub model: String,
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateModelSettingsRequest {
    pub default_chat_model: Option<String>,
    pub default_embedding_model: Option<String>,
    pub context_size: u32,
    pub test_timeout_seconds: u64,
    pub max_download_gb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelActionResult {
    pub accepted: bool,
    pub operation: Option<ModelOperation>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelTestResult {
    pub model: String,
    pub kind: String,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize)]
struct OllamaVersionResponse {
    version: String,
}

#[derive(Debug, Default, Deserialize)]
struct OllamaModelList {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Debug, Default, Deserialize)]
struct OllamaModel {
    #[serde(default, alias = "model")]
    name: String,
    size: Option<u64>,
    digest: Option<String>,
    modified_at: Option<String>,
    expires_at: Option<String>,
    size_vram: Option<u64>,
    #[serde(default)]
    details: OllamaModelDetails,
}

#[derive(Debug, Default, Deserialize)]
struct OllamaModelDetails {
    family: Option<String>,
    parameter_size: Option<String>,
    quantization_level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    #[serde(default)]
    response: String,
}

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
struct OllamaPullProgress {
    #[serde(default)]
    status: String,
    completed: Option<u64>,
    total: Option<u64>,
    error: Option<String>,
}

#[derive(Debug)]
struct LocalSnapshot {
    hardware: HardwareSnapshot,
    operations: Vec<ModelOperation>,
    settings: ModelSettings,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MODEL_MIGRATION)?;
    connection.execute(
        "UPDATE model_operations SET state='interrupted',status_message='Interrupted by HomeServer restart',failure_code='service_restarted',completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state IN ('pending','running')",
        [],
    )?;
    health_check(connection)?;
    maintain_history(connection)?;
    Ok(())
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MODEL_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "Model Center migration is not registered exactly once"
    );
    let _: i64 = connection.query_row("SELECT COUNT(*) FROM model_operations", [], |row| {
        row.get(0)
    })?;
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM model_operations WHERE operation_id NOT IN (SELECT operation_id FROM model_operations ORDER BY updated_at_utc DESC,operation_id DESC LIMIT ?1) AND state NOT IN ('pending','running')",
        params![MAX_OPERATION_HISTORY],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/models", get(model_snapshot))
        .route("/v1/models/pull", post(pull_model))
        .route("/v1/models/delete", post(delete_model))
        .route("/v1/models/unload", post(unload_model))
        .route("/v1/models/test", post(test_model))
        .route("/v1/models/settings", post(update_settings))
        .with_state(state)
}

async fn model_snapshot(State(state): State<Arc<AppState>>) -> ApiResult<ModelCenterSnapshot> {
    snapshot(state)
        .await
        .map(Json)
        .map_err(|error| internal_error("model_snapshot_failed", error))
}

async fn pull_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ModelNameRequest>,
) -> ApiResult<ModelActionResult> {
    let model = approved_model(&request.model)
        .map_err(|error| action_error("model_not_approved", error))?
        .model
        .to_owned();
    ensure_runtime_available()
        .await
        .map_err(|error| conflict_error("model_runtime_unavailable", error))?;

    let state_for_begin = state.clone();
    let model_for_begin = model.clone();
    let (operation, started) = tokio::task::spawn_blocking(move || {
        begin_pull_operation(&state_for_begin, &model_for_begin)
    })
    .await
    .map_err(task_error)?
    .map_err(|error| action_error("model_pull_rejected", error))?;

    if started {
        let task_state = state.clone();
        let task_operation = operation.clone();
        tokio::spawn(async move {
            if let Err(error) = run_pull(task_state.clone(), task_operation.clone()).await {
                let failure_code = public_failure_code(&error);
                let _ = tokio::task::spawn_blocking(move || {
                    finish_operation(
                        &task_state,
                        &task_operation.operation_id,
                        "failed",
                        "Model download failed",
                        Some(&failure_code),
                    )
                })
                .await;
                tracing::warn!(?error, model = %task_operation.model_name, "local model pull failed");
            }
        });
    }

    Ok(Json(ModelActionResult {
        accepted: started,
        operation: Some(operation),
        message: if started {
            format!("Started the approved local model download for {model}.")
        } else {
            format!("A download for {model} is already active.")
        },
    }))
}

async fn delete_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DeleteModelRequest>,
) -> ApiResult<ModelActionResult> {
    if request.confirmation != "DELETE" {
        return Err(action_error(
            "model_delete_confirmation_required",
            anyhow::anyhow!("type DELETE to remove this local model"),
        ));
    }
    let model = approved_model(&request.model)
        .map_err(|error| action_error("model_not_approved", error))?
        .model
        .to_owned();
    let client = ollama_client(60).map_err(|error| internal_error("model_client_failed", error))?;
    let response = client
        .delete(format!("{OLLAMA_API_BASE}/api/delete"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .map_err(|error| conflict_error("model_runtime_unavailable", error.into()))?;
    require_success(response, MAX_OLLAMA_JSON_BYTES)
        .await
        .map_err(|error| action_error("model_delete_failed", error))?;

    let state_for_db = state.clone();
    let model_for_db = model.clone();
    tokio::task::spawn_blocking(move || clear_deleted_model(&state_for_db, &model_for_db))
        .await
        .map_err(task_error)?
        .map_err(|error| internal_error("model_delete_state_failed", error))?;

    Ok(Json(ModelActionResult {
        accepted: true,
        operation: None,
        message: format!("Removed {model} from the local Ollama model store."),
    }))
}

async fn unload_model(Json(request): Json<ModelNameRequest>) -> ApiResult<ModelActionResult> {
    let model = approved_model(&request.model)
        .map_err(|error| action_error("model_not_approved", error))?
        .model
        .to_owned();
    let client = ollama_client(30).map_err(|error| internal_error("model_client_failed", error))?;
    let response = client
        .post(format!("{OLLAMA_API_BASE}/api/generate"))
        .json(&serde_json::json!({
            "model": model,
            "prompt": "",
            "stream": false,
            "keep_alive": 0
        }))
        .send()
        .await
        .map_err(|error| conflict_error("model_runtime_unavailable", error.into()))?;
    require_success(response, MAX_OLLAMA_JSON_BYTES)
        .await
        .map_err(|error| action_error("model_unload_failed", error))?;
    Ok(Json(ModelActionResult {
        accepted: true,
        operation: None,
        message: format!("Unloaded {model} from local memory."),
    }))
}

async fn test_model(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TestModelRequest>,
) -> ApiResult<ModelTestResult> {
    let catalog = approved_model(&request.model)
        .map_err(|error| action_error("model_not_approved", error))?;
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err(action_error(
            "model_test_prompt_invalid",
            anyhow::anyhow!("test prompt is required"),
        ));
    }
    if prompt.chars().count() > MAX_TEST_PROMPT_CHARS {
        return Err(action_error(
            "model_test_prompt_invalid",
            anyhow::anyhow!("test prompt exceeds the 500 character limit"),
        ));
    }

    let state_for_settings = state.clone();
    let settings = tokio::task::spawn_blocking(move || read_settings(&state_for_settings))
        .await
        .map_err(task_error)?
        .map_err(|error| internal_error("model_settings_failed", error))?;
    let client = ollama_client(settings.test_timeout_seconds)
        .map_err(|error| internal_error("model_client_failed", error))?;
    let started = Instant::now();

    let (kind, output) = if catalog.supports_embeddings && !catalog.supports_chat {
        let response = client
            .post(format!("{OLLAMA_API_BASE}/api/embed"))
            .json(&serde_json::json!({
                "model": catalog.model,
                "input": prompt,
                "truncate": true
            }))
            .send()
            .await
            .map_err(|error| action_error("model_test_failed", error.into()))?;
        let payload: OllamaEmbedResponse = decode_json(response, MAX_OLLAMA_JSON_BYTES)
            .await
            .map_err(|error| action_error("model_test_failed", error))?;
        let dimensions = payload.embeddings.first().map(Vec::len).unwrap_or(0);
        if dimensions == 0 {
            return Err(action_error(
                "model_test_failed",
                anyhow::anyhow!("embedding model returned no vector"),
            ));
        }
        (
            "embedding".to_owned(),
            format!("Generated a local embedding with {dimensions} dimensions."),
        )
    } else {
        let response = client
            .post(format!("{OLLAMA_API_BASE}/api/generate"))
            .json(&serde_json::json!({
                "model": catalog.model,
                "prompt": prompt,
                "stream": false,
                "keep_alive": 0,
                "options": {
                    "num_ctx": settings.context_size,
                    "num_predict": 128,
                    "temperature": 0.2
                }
            }))
            .send()
            .await
            .map_err(|error| action_error("model_test_failed", error.into()))?;
        let payload: OllamaGenerateResponse = decode_json(response, MAX_OLLAMA_JSON_BYTES)
            .await
            .map_err(|error| action_error("model_test_failed", error))?;
        if payload.response.trim().is_empty() {
            return Err(action_error(
                "model_test_failed",
                anyhow::anyhow!("model returned an empty response"),
            ));
        }
        (
            "chat".to_owned(),
            payload
                .response
                .trim()
                .chars()
                .take(MAX_TEST_RESPONSE_CHARS)
                .collect(),
        )
    };

    Ok(Json(ModelTestResult {
        model: catalog.model.to_owned(),
        kind,
        output,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }))
}

async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdateModelSettingsRequest>,
) -> ApiResult<ModelSettings> {
    if !(512..=32_768).contains(&request.context_size) {
        return Err(action_error(
            "model_settings_invalid",
            anyhow::anyhow!("context size must be between 512 and 32768"),
        ));
    }
    if !(10..=120).contains(&request.test_timeout_seconds) {
        return Err(action_error(
            "model_settings_invalid",
            anyhow::anyhow!("test timeout must be between 10 and 120 seconds"),
        ));
    }
    if !(1..=100).contains(&request.max_download_gb) {
        return Err(action_error(
            "model_settings_invalid",
            anyhow::anyhow!("download limit must be between 1 and 100 GB"),
        ));
    }

    for model in request
        .default_chat_model
        .iter()
        .chain(request.default_embedding_model.iter())
        .filter(|value| !value.trim().is_empty())
    {
        approved_model(model).map_err(|error| action_error("model_not_approved", error))?;
    }

    let installed = fetch_model_list("/api/tags")
        .await
        .map_err(|error| conflict_error("model_runtime_unavailable", error))?;
    let installed_names: HashSet<String> = installed
        .models
        .into_iter()
        .map(|model| model.name)
        .collect();
    validate_default_assignment(
        request.default_chat_model.as_deref(),
        true,
        &installed_names,
    )
    .map_err(|error| action_error("model_settings_invalid", error))?;
    validate_default_assignment(
        request.default_embedding_model.as_deref(),
        false,
        &installed_names,
    )
    .map_err(|error| action_error("model_settings_invalid", error))?;

    let state_for_db = state.clone();
    tokio::task::spawn_blocking(move || write_settings(&state_for_db, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("model_settings_failed", error))
}

async fn snapshot(state: Arc<AppState>) -> Result<ModelCenterSnapshot> {
    let state_for_local = state.clone();
    let local = tokio::task::spawn_blocking(move || local_snapshot(&state_for_local))
        .await
        .context("Model Center local snapshot task failed")??;

    let client = ollama_client(10)?;
    let version_response = match client
        .get(format!("{OLLAMA_API_BASE}/api/version"))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => {
            let catalog = catalog_with_state(&local.hardware, &[], &[]);
            return Ok(ModelCenterSnapshot {
                runtime: ModelRuntime {
                    provider: "ollama".to_owned(),
                    api_url: OLLAMA_API_BASE.to_owned(),
                    state: "not_running".to_owned(),
                    version: None,
                    last_error: Some("ollama_unavailable".to_owned()),
                },
                hardware: local.hardware,
                installed_models: Vec::new(),
                catalog,
                operations: local.operations,
                settings: local.settings,
                local_only: true,
            });
        }
    };
    let version: OllamaVersionResponse =
        decode_json(version_response, MAX_OLLAMA_JSON_BYTES).await?;
    let installed = fetch_model_list("/api/tags").await?;
    let running = fetch_model_list("/api/ps").await.unwrap_or_default();
    let running_names: HashSet<String> = running
        .models
        .iter()
        .map(|model| model.name.clone())
        .collect();
    let running_details = running
        .models
        .iter()
        .map(|model| (model.name.clone(), model.size_vram.unwrap_or(0)))
        .collect::<std::collections::HashMap<_, _>>();

    let installed_models = installed
        .models
        .into_iter()
        .map(|model| LocalModel {
            running: running_names
                .iter()
                .any(|running| model_names_match(running, &model.name)),
            size_vram_bytes: running_details
                .iter()
                .find(|(name, _)| model_names_match(name, &model.name))
                .map(|(_, size)| *size)
                .unwrap_or(0),
            name: model.name,
            size_bytes: model.size.unwrap_or(0),
            digest: model.digest,
            modified_at_utc: model.modified_at.or(model.expires_at),
            family: model.details.family,
            parameter_size: model.details.parameter_size,
            quantization_level: model.details.quantization_level,
        })
        .collect::<Vec<_>>();
    let installed_names = installed_models
        .iter()
        .map(|model| model.name.clone())
        .collect::<Vec<_>>();
    let running_names = running_names.into_iter().collect::<Vec<_>>();
    let catalog = catalog_with_state(&local.hardware, &installed_names, &running_names);

    Ok(ModelCenterSnapshot {
        runtime: ModelRuntime {
            provider: "ollama".to_owned(),
            api_url: OLLAMA_API_BASE.to_owned(),
            state: "running".to_owned(),
            version: Some(version.version),
            last_error: None,
        },
        hardware: local.hardware,
        installed_models,
        catalog,
        operations: local.operations,
        settings: local.settings,
        local_only: true,
    })
}

fn local_snapshot(state: &AppState) -> Result<LocalSnapshot> {
    let connection = state.connection()?;
    Ok(LocalSnapshot {
        hardware: hardware_snapshot(&state.config.data_dir),
        operations: list_operations(&connection)?,
        settings: read_settings_from_connection(&connection)?,
    })
}

fn read_settings(state: &AppState) -> Result<ModelSettings> {
    let connection = state.connection()?;
    read_settings_from_connection(&connection)
}

pub(crate) fn configured_embedding_model_from_connection(
    connection: &Connection,
) -> Result<Option<String>> {
    Ok(read_settings_from_connection(connection)?.default_embedding_model)
}

pub(crate) fn validate_embedding_model(model: &str) -> Result<()> {
    let definition = approved_model(model)?;
    ensure!(
        definition.supports_embeddings,
        "configured model does not support embeddings"
    );
    Ok(())
}

pub(crate) async fn embed_texts(
    state: Arc<AppState>,
    model: String,
    inputs: Vec<String>,
) -> Result<Vec<Vec<f32>>> {
    validate_embedding_model(&model)?;
    ensure!(!inputs.is_empty(), "embedding input is required");
    ensure!(
        inputs.len() <= MAX_EMBED_INPUTS,
        "embedding batch exceeds the 8 input limit"
    );
    let mut total_chars = 0_usize;
    for input in &inputs {
        let count = input.chars().count();
        ensure!(count > 0, "embedding input is empty");
        ensure!(
            count <= MAX_EMBED_INPUT_CHARS,
            "embedding input exceeds the 2000 character limit"
        );
        total_chars = total_chars.saturating_add(count);
    }
    ensure!(
        total_chars <= MAX_EMBED_TOTAL_CHARS,
        "embedding batch exceeds the total character limit"
    );

    let state_for_settings = state.clone();
    let settings = tokio::task::spawn_blocking(move || read_settings(&state_for_settings))
        .await
        .context("embedding settings task failed")??;
    ensure!(
        settings.default_embedding_model.as_deref() == Some(model.as_str()),
        "embedding model no longer matches the configured default"
    );
    let client = ollama_client(settings.test_timeout_seconds)?;
    let response = client
        .post(format!("{OLLAMA_API_BASE}/api/embed"))
        .json(&serde_json::json!({
            "model": model,
            "input": inputs,
            "truncate": true
        }))
        .send()
        .await
        .context("unable to reach the local Ollama embedding runtime")?;
    let payload: OllamaEmbedResponse = decode_json(response, MAX_OLLAMA_JSON_BYTES).await?;
    ensure!(
        payload.embeddings.len() <= MAX_EMBED_INPUTS,
        "embedding runtime returned too many vectors"
    );
    ensure!(
        payload.embeddings.iter().all(|vector| {
            !vector.is_empty()
                && vector.len() <= MAX_EMBEDDING_DIMENSIONS
                && vector.iter().all(|value| value.is_finite())
        }),
        "embedding runtime returned an invalid vector"
    );
    Ok(payload.embeddings)
}

fn write_settings(state: &AppState, request: UpdateModelSettingsRequest) -> Result<ModelSettings> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    set_setting(
        &transaction,
        "model_default_chat",
        request.default_chat_model.as_deref().unwrap_or("").trim(),
    )?;
    set_setting(
        &transaction,
        "model_default_embedding",
        request
            .default_embedding_model
            .as_deref()
            .unwrap_or("")
            .trim(),
    )?;
    set_setting(
        &transaction,
        "model_context_size",
        &request.context_size.to_string(),
    )?;
    set_setting(
        &transaction,
        "model_test_timeout_seconds",
        &request.test_timeout_seconds.to_string(),
    )?;
    set_setting(
        &transaction,
        "model_max_download_gb",
        &request.max_download_gb.to_string(),
    )?;
    transaction.execute(
        "INSERT INTO service_events (event_type,message) VALUES ('model.settings_updated','Local Model Center settings were updated')",
        [],
    )?;
    transaction.commit()?;
    read_settings_from_connection(&connection)
}

fn clear_deleted_model(state: &AppState, model: &str) -> Result<()> {
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;
    for key in ["model_default_chat", "model_default_embedding"] {
        let current = setting_string(&transaction, key, "")?;
        if model_names_match(&current, model) {
            set_setting(&transaction, key, "")?;
        }
    }
    transaction.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('model.deleted','An approved local model was deleted',json_object('model',?1))",
        params![model],
    )?;
    transaction.commit()?;
    Ok(())
}

fn read_settings_from_connection(connection: &Connection) -> Result<ModelSettings> {
    let chat = setting_string(connection, "model_default_chat", "")?;
    let embedding = setting_string(connection, "model_default_embedding", "")?;
    Ok(ModelSettings {
        default_chat_model: non_empty(chat),
        default_embedding_model: non_empty(embedding),
        context_size: setting_u32(connection, "model_context_size", 4_096, 512, 32_768)?,
        test_timeout_seconds: u64::from(setting_u32(
            connection,
            "model_test_timeout_seconds",
            60,
            10,
            120,
        )?),
        max_download_gb: setting_u32(connection, "model_max_download_gb", 20, 1, 100)?,
    })
}

fn begin_pull_operation(state: &AppState, model: &str) -> Result<(ModelOperation, bool)> {
    let catalog = approved_model(model)?;
    let hardware = hardware_snapshot(&state.config.data_dir);
    let settings = read_settings(state)?;
    let max_bytes = u64::from(settings.max_download_gb) * 1024 * 1024 * 1024;
    ensure!(
        catalog.estimated_size_bytes <= max_bytes,
        "model exceeds the configured download limit"
    );
    if hardware.free_disk_bytes > 0 {
        let required = catalog
            .estimated_size_bytes
            .saturating_add(2 * 1024 * 1024 * 1024);
        ensure!(
            hardware.free_disk_bytes >= required,
            "insufficient free disk space for this model"
        );
    }

    let connection = state.connection()?;
    if let Some(operation) = active_pull(&connection, model)? {
        return Ok((operation, false));
    }
    let operation_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO model_operations (operation_id,model_name,operation_type,state,status_message,total_bytes) VALUES (?1,?2,'pull','pending','Queued for local download',?3)",
        params![operation_id, model, catalog.estimated_size_bytes as i64],
    )?;
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('model.pull_started','An approved local model download was started',json_object('model',?1,'operation_id',?2))",
        params![model, operation_id],
    )?;
    Ok((operation_by_id(&connection, &operation_id)?, true))
}

async fn run_pull(state: Arc<AppState>, operation: ModelOperation) -> Result<()> {
    update_operation(
        &state,
        &operation.operation_id,
        "running",
        "Connecting to the local Ollama model registry",
        0,
        operation.total_bytes,
        None,
    )?;
    let client = ollama_client(2 * 60 * 60)?;
    let response = client
        .post(format!("{OLLAMA_API_BASE}/api/pull"))
        .json(&serde_json::json!({ "model": operation.model_name, "stream": true }))
        .send()
        .await
        .context("unable to start Ollama model pull")?;
    ensure!(
        response.status().is_success(),
        "Ollama rejected the model pull"
    );

    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut received = 0_usize;
    let mut last_completed = 0_u64;
    let mut last_status = String::new();
    let mut last_update = Instant::now() - Duration::from_secs(1);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Ollama model pull stream failed")?;
        received = received
            .checked_add(chunk.len())
            .context("Ollama model pull stream size overflow")?;
        ensure!(
            received <= MAX_PULL_STREAM_BYTES,
            "Ollama model pull progress exceeded the response limit"
        );
        buffer.extend_from_slice(&chunk);
        ensure!(
            buffer.len() <= MAX_PULL_LINE_BYTES,
            "Ollama model pull progress line exceeded the limit"
        );

        while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.drain(..=position).collect::<Vec<_>>();
            let line = line.strip_suffix(b"\n").unwrap_or(&line);
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            let progress: OllamaPullProgress =
                serde_json::from_slice(line).context("invalid Ollama model pull progress")?;
            if let Some(error) = progress.error {
                bail!(
                    "Ollama model pull failed: {}",
                    public_ollama_message(&error)
                );
            }
            let completed = progress.completed.unwrap_or(last_completed);
            let total = progress.total.unwrap_or(operation.total_bytes);
            let status = if progress.status.trim().is_empty() {
                "Downloading approved local model".to_owned()
            } else {
                public_ollama_message(&progress.status)
            };
            let should_update = status != last_status
                || completed.saturating_sub(last_completed) >= 1024 * 1024
                || last_update.elapsed() >= Duration::from_secs(1);
            if should_update {
                update_operation(
                    &state,
                    &operation.operation_id,
                    "running",
                    &status,
                    completed,
                    total,
                    None,
                )?;
                last_completed = completed;
                last_status = status;
                last_update = Instant::now();
            }
        }
    }

    if !buffer.is_empty() {
        let progress: OllamaPullProgress =
            serde_json::from_slice(&buffer).context("invalid final Ollama model pull progress")?;
        if let Some(error) = progress.error {
            bail!(
                "Ollama model pull failed: {}",
                public_ollama_message(&error)
            );
        }
        last_completed = progress.completed.unwrap_or(last_completed);
    }

    finish_operation(
        &state,
        &operation.operation_id,
        "succeeded",
        "Local model download completed and was verified by Ollama",
        None,
    )?;
    let connection = state.connection()?;
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('model.pull_completed','An approved local model download completed',json_object('model',?1,'operation_id',?2,'completed_bytes',?3))",
        params![operation.model_name, operation.operation_id, last_completed as i64],
    )?;
    Ok(())
}

fn update_operation(
    state: &AppState,
    operation_id: &str,
    operation_state: &str,
    status_message: &str,
    completed_bytes: u64,
    total_bytes: u64,
    failure_code: Option<&str>,
) -> Result<()> {
    state.connection()?.execute(
        "UPDATE model_operations SET state=?1,status_message=?2,completed_bytes=?3,total_bytes=?4,failure_code=?5,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?6",
        params![
            operation_state,
            status_message.chars().take(300).collect::<String>(),
            completed_bytes as i64,
            total_bytes as i64,
            failure_code,
            operation_id,
        ],
    )?;
    Ok(())
}

fn finish_operation(
    state: &AppState,
    operation_id: &str,
    operation_state: &str,
    status_message: &str,
    failure_code: Option<&str>,
) -> Result<()> {
    state.connection()?.execute(
        "UPDATE model_operations SET state=?1,status_message=?2,failure_code=?3,completed_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE operation_id=?4",
        params![operation_state, status_message, failure_code, operation_id],
    )?;
    Ok(())
}

fn list_operations(connection: &Connection) -> Result<Vec<ModelOperation>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {OPERATION_COLUMNS} FROM model_operations ORDER BY updated_at_utc DESC,operation_id DESC LIMIT 50"
    ))?;
    let operations = statement
        .query_map([], operation_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(operations)
}

fn active_pull(connection: &Connection, model: &str) -> Result<Option<ModelOperation>> {
    connection
        .query_row(
            &format!("SELECT {OPERATION_COLUMNS} FROM model_operations WHERE model_name=?1 AND operation_type='pull' AND state IN ('pending','running') ORDER BY created_at_utc DESC LIMIT 1"),
            params![model],
            operation_from_row,
        )
        .optional()
        .map_err(Into::into)
}

fn operation_by_id(connection: &Connection, operation_id: &str) -> Result<ModelOperation> {
    connection
        .query_row(
            &format!("SELECT {OPERATION_COLUMNS} FROM model_operations WHERE operation_id=?1"),
            params![operation_id],
            operation_from_row,
        )
        .context("Model Center operation was not found")
}

fn operation_from_row(row: &Row<'_>) -> rusqlite::Result<ModelOperation> {
    Ok(ModelOperation {
        operation_id: row.get(0)?,
        model_name: row.get(1)?,
        operation_type: row.get(2)?,
        state: row.get(3)?,
        status_message: row.get(4)?,
        completed_bytes: row.get::<_, i64>(5)?.max(0) as u64,
        total_bytes: row.get::<_, i64>(6)?.max(0) as u64,
        failure_code: row.get(7)?,
        created_at_utc: parse_utc(row.get(8)?).map_err(to_sql_error)?,
        updated_at_utc: parse_utc(row.get(9)?).map_err(to_sql_error)?,
        completed_at_utc: row
            .get::<_, Option<String>>(10)?
            .map(parse_utc)
            .transpose()
            .map_err(to_sql_error)?,
    })
}

fn approved_model(model: &str) -> Result<&'static CatalogDefinition> {
    let model = model.trim();
    catalog_definitions()
        .iter()
        .find(|candidate| candidate.model == model)
        .ok_or_else(|| anyhow::anyhow!("model is not in the approved HomeServer starter catalog"))
}

#[derive(Debug)]
struct CatalogDefinition {
    model: &'static str,
    display_name: &'static str,
    purpose: &'static str,
    estimated_size_bytes: u64,
    minimum_memory_bytes: u64,
    supports_chat: bool,
    supports_embeddings: bool,
}

fn catalog_definitions() -> &'static [CatalogDefinition] {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    static CATALOG: [CatalogDefinition; 5] = [
        CatalogDefinition {
            model: "gemma3:1b",
            display_name: "Gemma 3 1B",
            purpose: "Lightweight local chat and summarization",
            estimated_size_bytes: 815 * MIB,
            minimum_memory_bytes: 4 * GIB,
            supports_chat: true,
            supports_embeddings: false,
        },
        CatalogDefinition {
            model: "llama3.2:1b",
            display_name: "Llama 3.2 1B",
            purpose: "Fast local assistance and rewriting",
            estimated_size_bytes: 1_300 * MIB,
            minimum_memory_bytes: 4 * GIB,
            supports_chat: true,
            supports_embeddings: false,
        },
        CatalogDefinition {
            model: "llama3.2:3b",
            display_name: "Llama 3.2 3B",
            purpose: "General local reasoning and retrieval",
            estimated_size_bytes: 2_000 * MIB,
            minimum_memory_bytes: 8 * GIB,
            supports_chat: true,
            supports_embeddings: false,
        },
        CatalogDefinition {
            model: "gemma3:4b",
            display_name: "Gemma 3 4B",
            purpose: "Higher-quality local chat on capable systems",
            estimated_size_bytes: 3_300 * MIB,
            minimum_memory_bytes: 12 * GIB,
            supports_chat: true,
            supports_embeddings: false,
        },
        CatalogDefinition {
            model: "nomic-embed-text:latest",
            display_name: "Nomic Embed Text",
            purpose: "Semantic Knowledge Vault indexing and retrieval",
            estimated_size_bytes: 274 * MIB,
            minimum_memory_bytes: 4 * GIB,
            supports_chat: false,
            supports_embeddings: true,
        },
    ];
    &CATALOG
}

fn catalog_with_state(
    hardware: &HardwareSnapshot,
    installed: &[String],
    running: &[String],
) -> Vec<CatalogModel> {
    catalog_definitions()
        .iter()
        .map(|definition| {
            let is_installed = installed
                .iter()
                .any(|model| model_names_match(model, definition.model));
            let is_running = running
                .iter()
                .any(|model| model_names_match(model, definition.model));
            let disk_ready = hardware.free_disk_bytes == 0
                || hardware.free_disk_bytes
                    >= definition
                        .estimated_size_bytes
                        .saturating_add(2 * 1024 * 1024 * 1024);
            CatalogModel {
                model: definition.model.to_owned(),
                display_name: definition.display_name.to_owned(),
                purpose: definition.purpose.to_owned(),
                estimated_size_bytes: definition.estimated_size_bytes,
                minimum_memory_bytes: definition.minimum_memory_bytes,
                supports_chat: definition.supports_chat,
                supports_embeddings: definition.supports_embeddings,
                installed: is_installed,
                running: is_running,
                recommended: hardware.total_memory_bytes >= definition.minimum_memory_bytes
                    && disk_ready,
            }
        })
        .collect()
}

fn validate_default_assignment(
    model: Option<&str>,
    chat: bool,
    installed: &HashSet<String>,
) -> Result<()> {
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let definition = approved_model(model)?;
    if chat {
        ensure!(
            definition.supports_chat,
            "default chat model must support chat"
        );
    } else {
        ensure!(
            definition.supports_embeddings,
            "default embedding model must support embeddings"
        );
    }
    ensure!(
        installed
            .iter()
            .any(|installed_model| model_names_match(installed_model, model)),
        "default model must be installed locally"
    );
    Ok(())
}

fn model_names_match(left: &str, right: &str) -> bool {
    left == right || strip_latest(left) == strip_latest(right)
}

fn strip_latest(value: &str) -> &str {
    value.strip_suffix(":latest").unwrap_or(value)
}

async fn ensure_runtime_available() -> Result<()> {
    let response = ollama_client(5)?
        .get(format!("{OLLAMA_API_BASE}/api/version"))
        .send()
        .await
        .context("Ollama is not running on the approved loopback endpoint")?;
    ensure!(
        response.status().is_success(),
        "Ollama runtime is unavailable"
    );
    Ok(())
}

async fn fetch_model_list(path: &str) -> Result<OllamaModelList> {
    let response = ollama_client(10)?
        .get(format!("{OLLAMA_API_BASE}{path}"))
        .send()
        .await
        .context("unable to reach the local Ollama runtime")?;
    decode_json(response, MAX_OLLAMA_JSON_BYTES).await
}

fn ollama_client(timeout_seconds: u64) -> Result<Client> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(timeout_seconds.clamp(5, 2 * 60 * 60)))
        .build()
        .context("unable to create the fixed-loopback Ollama client")
}

async fn decode_json<T: DeserializeOwned>(response: Response, maximum: usize) -> Result<T> {
    let status = response.status();
    let bytes = read_bounded(response, maximum).await?;
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
        bail!("Ollama request failed: {}", public_ollama_message(&message));
    }
    serde_json::from_slice(&bytes).context("invalid JSON from the local Ollama runtime")
}

async fn require_success(response: Response, maximum: usize) -> Result<()> {
    let status = response.status();
    let bytes = read_bounded(response, maximum).await?;
    if status.is_success() {
        return Ok(());
    }
    let message = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(&bytes).chars().take(300).collect());
    bail!("Ollama request failed: {}", public_ollama_message(&message));
}

async fn read_bounded(response: Response, maximum: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        bail!("Ollama response exceeds the local response limit");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("unable to read the local Ollama response")?;
        let next = bytes
            .len()
            .checked_add(chunk.len())
            .context("Ollama response size overflow")?;
        ensure!(
            next <= maximum,
            "Ollama response exceeds the local response limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn setting_string(connection: &Connection, key: &str, default: &str) -> Result<String> {
    Ok(connection
        .query_row(
            "SELECT setting_value FROM homeserver_settings WHERE setting_key=?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| default.to_owned()))
}

fn setting_u32(
    connection: &Connection,
    key: &str,
    default: u32,
    minimum: u32,
    maximum: u32,
) -> Result<u32> {
    let value = setting_string(connection, key, &default.to_string())?
        .parse::<u32>()
        .unwrap_or(default);
    Ok(value.clamp(minimum, maximum))
}

fn set_setting(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO homeserver_settings (setting_key,setting_value,updated_at_utc) VALUES (?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(setting_key) DO UPDATE SET setting_value=excluded.setting_value,updated_at_utc=excluded.updated_at_utc",
        params![key, value],
    )?;
    Ok(())
}

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn public_ollama_message(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(300)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn public_failure_code(error: &anyhow::Error) -> String {
    let text = error.to_string().to_ascii_lowercase();
    if text.contains("disk") {
        "insufficient_disk_space"
    } else if text.contains("timeout") {
        "ollama_timeout"
    } else if text.contains("response limit") || text.contains("size overflow") {
        "ollama_response_rejected"
    } else if text.contains("not running")
        || text.contains("connect")
        || text.contains("unavailable")
    {
        "ollama_unavailable"
    } else {
        "model_operation_failed"
    }
    .to_owned()
}

fn hardware_snapshot(data_dir: &Path) -> HardwareSnapshot {
    let logical_cpu_count = std::thread::available_parallelism()
        .map(|value| value.get() as u32)
        .unwrap_or(1);
    let (total_memory_bytes, available_memory_bytes) = memory_status();
    HardwareSnapshot {
        logical_cpu_count,
        total_memory_bytes,
        available_memory_bytes,
        free_disk_bytes: free_disk_space(data_dir),
        gpu_name: std::env::var("MG_HOMESERVER_GPU_NAME")
            .ok()
            .map(|value| value.trim().chars().take(160).collect::<String>())
            .filter(|value| !value.is_empty()),
    }
}

#[cfg(windows)]
fn memory_status() -> (u64, u64) {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { zeroed() };
    status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
        (status.ullTotalPhys, status.ullAvailPhys)
    } else {
        (0, 0)
    }
}

#[cfg(not(windows))]
fn memory_status() -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let mut total = 0_u64;
    let mut available = 0_u64;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total = parse_kib(value);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available = parse_kib(value);
        }
    }
    (total, available)
}

#[cfg(not(windows))]
fn parse_kib(value: &str) -> u64 {
    value
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(1024)
}

#[cfg(windows)]
fn free_disk_space(path: &Path) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut available = 0_u64;
    let result = unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result != 0 {
        available
    } else {
        0
    }
}

#[cfg(not(windows))]
fn free_disk_space(_path: &Path) -> u64 {
    0
}

fn parse_utc(value: String) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|date| date.with_timezone(&Utc))
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

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}

fn conflict_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::CONFLICT,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::error!(?error, error_code = code, "Model Center request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: "The local Model Center operation failed.".to_owned(),
        }),
    )
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("model_task_failed", error.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch(include_str!(
                "../../../database/migrations/0001_initial.sql"
            ))
            .expect("initial migration");
        initialize(&connection).expect("model migration");
        connection
    }

    #[test]
    fn starter_catalog_is_bounded_and_approved() {
        let catalog = catalog_definitions();
        assert_eq!(catalog.len(), 5);
        assert!(catalog.iter().all(|model| model.model.contains(':')));
        assert!(catalog.iter().any(|model| model.supports_embeddings));
        assert!(catalog.iter().all(|model| model.estimated_size_bytes > 0));
    }

    #[test]
    fn unknown_model_is_rejected() {
        assert!(approved_model("unapproved/model:latest").is_err());
    }

    #[test]
    fn restart_marks_active_operations_interrupted() {
        let connection = connection();
        connection
            .execute(
                "INSERT INTO model_operations (operation_id,model_name,operation_type,state) VALUES ('op-1','llama3.2:1b','pull','running')",
                [],
            )
            .expect("operation insert");
        initialize(&connection).expect("restart initialization");
        let state: String = connection
            .query_row(
                "SELECT state FROM model_operations WHERE operation_id='op-1'",
                [],
                |row| row.get(0),
            )
            .expect("operation state");
        assert_eq!(state, "interrupted");
    }

    #[test]
    fn recommendations_respect_memory_thresholds() {
        let hardware = HardwareSnapshot {
            logical_cpu_count: 4,
            total_memory_bytes: 6 * 1024 * 1024 * 1024,
            available_memory_bytes: 4 * 1024 * 1024 * 1024,
            free_disk_bytes: 30 * 1024 * 1024 * 1024,
            gpu_name: None,
        };
        let catalog = catalog_with_state(&hardware, &[], &[]);
        assert!(
            catalog
                .iter()
                .find(|model| model.model == "llama3.2:1b")
                .expect("1b model")
                .recommended
        );
        assert!(
            !catalog
                .iter()
                .find(|model| model.model == "gemma3:4b")
                .expect("4b model")
                .recommended
        );
    }

    #[test]
    fn settings_round_trip_and_clamp() {
        let connection = connection();
        let settings = read_settings_from_connection(&connection).expect("settings");
        assert_eq!(settings.context_size, 4_096);
        assert_eq!(settings.test_timeout_seconds, 60);
        assert_eq!(settings.max_download_gb, 20);
    }

    #[test]
    fn operation_history_is_bounded() {
        let connection = connection();
        for index in 0..250 {
            connection
                .execute(
                    "INSERT INTO model_operations (operation_id,model_name,operation_type,state) VALUES (?1,'llama3.2:1b','test','succeeded')",
                    params![format!("op-{index:03}")],
                )
                .expect("operation insert");
        }
        maintain_history(&connection).expect("history maintenance");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM model_operations", [], |row| {
                row.get(0)
            })
            .expect("operation count");
        assert_eq!(count, MAX_OPERATION_HISTORY);
    }

    #[test]
    fn fixed_runtime_is_loopback_only() {
        assert_eq!(OLLAMA_API_BASE, "http://127.0.0.1:11434");
    }

    #[test]
    fn local_database_initializes_in_temporary_directory() {
        let directory = tempdir().expect("directory");
        let path = directory.path().join("models.sqlite3");
        let connection = Connection::open(path).expect("database");
        connection
            .execute_batch(include_str!(
                "../../../database/migrations/0001_initial.sql"
            ))
            .expect("initial migration");
        initialize(&connection).expect("model migration");
        health_check(&connection).expect("health check");
    }
}
