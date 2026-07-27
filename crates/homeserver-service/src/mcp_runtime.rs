use crate::{model_center, semantic_vault, AppState};
use anyhow::{ensure, Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Arc, time::Instant};
use uuid::Uuid;

const MCP_MIGRATION: &str = include_str!("../../../database/migrations/0009_mcp_runtime.sql");
const MCP_MIGRATION_KEY: &str = "0009_mcp_runtime";
const MCP_ENDPOINT: &str = "http://127.0.0.1:47831/mcp";
const MCP_BRIDGE_FILE_NAME: &str = "microgifter-homeserver-mcp.exe";
const MCP_PROTOCOL_LATEST: &str = "2025-11-25";
const MCP_PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
const MAX_MCP_BODY_BYTES: usize = 128 * 1024;
const MAX_MCP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_MCP_QUERY_CHARS: usize = 200;
const MAX_MCP_DOCUMENT_CHARS: usize = 20_000;
const MAX_MCP_SEARCH_RESULTS: u32 = 20;
const MAX_MCP_CLIENTS: i64 = 100;
const MAX_MCP_REQUESTS_PER_MINUTE: i64 = 120;
const MAX_MCP_AUDIT_RECEIPTS: i64 = 5_000;
const CLIENT_COLUMNS: &str = "client_id,display_name,token_hint,scopes_json,state,expires_at_utc,last_used_at_utc,request_count,created_at_utc,updated_at_utc,revoked_at_utc";
const ALLOWED_SCOPES: &[&str] = &[
    "system.read",
    "cloud.read",
    "models.read",
    "knowledge.search",
    "knowledge.read",
];

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpClientSummary {
    pub client_id: String,
    pub display_name: String,
    pub token_hint: String,
    pub scopes: Vec<String>,
    pub state: String,
    pub expires_at_utc: String,
    pub last_used_at_utc: Option<String>,
    pub request_count: u64,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub revoked_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpRuntimeSnapshot {
    pub state: String,
    pub endpoint: String,
    pub transport: Vec<String>,
    pub protocol_versions: Vec<String>,
    pub bridge_file_name: String,
    pub active_clients: u64,
    pub clients: Vec<McpClientSummary>,
    pub scopes: Vec<String>,
    pub tools: Vec<String>,
    pub requests_per_minute: u64,
    pub read_only: bool,
    pub local_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateMcpClientRequest {
    pub display_name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub expires_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpClientCredential {
    pub client: McpClientSummary,
    pub token: String,
    pub endpoint: String,
    pub bridge_file_name: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct RevokeMcpClientRequest {
    pub client_id: String,
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpActionResult {
    pub client: McpClientSummary,
    pub message: String,
}

#[derive(Debug, Clone)]
struct McpClientContext {
    client_id: String,
    scopes: HashSet<String>,
}

#[derive(Debug)]
enum AuthFailure {
    Unauthorized,
    RateLimited,
    Internal(anyhow::Error),
}

#[derive(Debug)]
struct RpcFailure {
    code: i64,
    message: String,
    capability: &'static str,
    detail_code: &'static str,
}

impl RpcFailure {
    fn invalid_params(message: impl Into<String>, capability: &'static str) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            capability,
            detail_code: "invalid_params",
        }
    }

    fn denied(scope: &'static str) -> Self {
        Self {
            code: -32003,
            message: format!("MCP client is not authorized for scope '{scope}'."),
            capability: scope,
            detail_code: "scope_denied",
        }
    }

    fn internal(error: anyhow::Error, capability: &'static str) -> Self {
        tracing::warn!(?error, capability, "read-only MCP operation failed");
        Self {
            code: -32603,
            message: "HomeServer could not complete the local read-only operation.".to_owned(),
            capability,
            detail_code: "operation_failed",
        }
    }
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MCP_MIGRATION)?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MCP_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "MCP runtime migration is not registered exactly once"
    );
    for table in ["mcp_clients", "mcp_rate_limits", "mcp_audit_receipts"] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    let current_minute = Utc::now().timestamp() / 60;
    connection.execute(
        "DELETE FROM mcp_rate_limits WHERE window_epoch_minute < ?1",
        params![current_minute - 10],
    )?;
    connection.execute(
        "DELETE FROM mcp_audit_receipts WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-90 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM mcp_audit_receipts WHERE receipt_id NOT IN (SELECT receipt_id FROM mcp_audit_receipts ORDER BY created_at_utc DESC,receipt_id DESC LIMIT ?1)",
        params![MAX_MCP_AUDIT_RECEIPTS],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    let control = Router::new()
        .route("/v1/mcp", get(mcp_snapshot))
        .route("/v1/mcp/clients", post(create_mcp_client))
        .route("/v1/mcp/clients/revoke", post(revoke_mcp_client))
        .layer(DefaultBodyLimit::max(64 * 1024));
    let protocol = Router::new()
        .route("/mcp", get(mcp_get).post(mcp_post))
        .layer(DefaultBodyLimit::max(MAX_MCP_BODY_BYTES));
    Router::new()
        .merge(control)
        .merge(protocol)
        .with_state(state)
}

async fn mcp_snapshot(State(state): State<Arc<AppState>>) -> ApiResult<McpRuntimeSnapshot> {
    tokio::task::spawn_blocking(move || runtime_snapshot(&state))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("mcp_snapshot_failed", error))
}

async fn create_mcp_client(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CreateMcpClientRequest>,
) -> ApiResult<McpClientCredential> {
    tokio::task::spawn_blocking(move || create_client(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("mcp_client_create_rejected", error))
}

async fn revoke_mcp_client(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RevokeMcpClientRequest>,
) -> ApiResult<McpActionResult> {
    tokio::task::spawn_blocking(move || revoke_client(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("mcp_client_revoke_rejected", error))
}

async fn mcp_get(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match authorize_request(state, headers).await {
        Ok(_) => {
            let mut response = (
                StatusCode::METHOD_NOT_ALLOWED,
                Json(json!({
                    "ok": false,
                    "error": "mcp_stream_unavailable",
                    "message": "This read-only HomeServer MCP runtime is stateless. Use POST or the packaged stdio bridge."
                })),
            )
                .into_response();
            response
                .headers_mut()
                .insert(header::ALLOW, HeaderValue::from_static("POST"));
            response
        }
        Err(error) => auth_response(error),
    }
}

async fn mcp_post(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    let started = Instant::now();
    let request_bytes = body.len();
    let client = match authorize_request(state.clone(), headers).await {
        Ok(client) => client,
        Err(error) => return auth_response(error),
    };

    let request = match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(request)) => request,
        Ok(_) => {
            let response = rpc_error(
                Value::Null,
                -32600,
                "MCP request must be one JSON-RPC object.",
            );
            let response = json_rpc_http_response(response);
            record_audit_async(
                state,
                Some(client.client_id),
                "invalid",
                "protocol",
                "error",
                "invalid_request",
                request_bytes,
                response_size(&response),
                started,
            );
            return response;
        }
        Err(_) => {
            let response = rpc_error(Value::Null, -32700, "MCP request JSON is invalid.");
            let response = json_rpc_http_response(response);
            record_audit_async(
                state,
                Some(client.client_id),
                "invalid",
                "protocol",
                "error",
                "parse_error",
                request_bytes,
                response_size(&response),
                started,
            );
            return response;
        }
    };

    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let response =
            json_rpc_http_response(rpc_error(id, -32600, "MCP request must use JSON-RPC 2.0."));
        record_audit_async(
            state,
            Some(client.client_id),
            "invalid",
            "protocol",
            "error",
            "invalid_jsonrpc_version",
            request_bytes,
            response_size(&response),
            started,
        );
        return response;
    }

    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let id = request.get("id").cloned();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    if method.is_empty() {
        let response = json_rpc_http_response(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "MCP request method is required.",
        ));
        record_audit_async(
            state,
            Some(client.client_id),
            "invalid",
            "protocol",
            "error",
            "method_missing",
            request_bytes,
            response_size(&response),
            started,
        );
        return response;
    }

    let dispatch = dispatch_request(state.clone(), &client, &method, params).await;
    if id.is_none() {
        let (outcome, detail_code, capability) = match dispatch {
            Ok((_, capability)) => ("success", "notification_accepted", capability),
            Err(error) => ("error", error.detail_code, error.capability),
        };
        record_audit_async(
            state,
            Some(client.client_id),
            method,
            capability,
            outcome,
            detail_code,
            request_bytes,
            0,
            started,
        );
        return StatusCode::ACCEPTED.into_response();
    }

    let id = id.unwrap_or(Value::Null);
    let (response_value, outcome, detail_code, capability) = match dispatch {
        Ok((result, capability)) => (rpc_success(id, result), "success", "completed", capability),
        Err(error) => (
            rpc_error(id, error.code, &error.message),
            if error.code == -32003 {
                "denied"
            } else {
                "error"
            },
            error.detail_code,
            error.capability,
        ),
    };
    let response = json_rpc_http_response(response_value);
    record_audit_async(
        state,
        Some(client.client_id),
        method,
        capability,
        outcome,
        detail_code,
        request_bytes,
        response_size(&response),
        started,
    );
    response
}

async fn authorize_request(
    state: Arc<AppState>,
    headers: HeaderMap,
) -> Result<McpClientContext, AuthFailure> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.starts_with("mghs_mcp_") && value.len() <= 96)
        .ok_or(AuthFailure::Unauthorized)?
        .to_owned();
    tokio::task::spawn_blocking(move || authorize_token(&state, &token))
        .await
        .map_err(|error| AuthFailure::Internal(error.into()))?
}

fn authorize_token(state: &AppState, token: &str) -> Result<McpClientContext, AuthFailure> {
    let token_hash = hash_token(token);
    let mut connection = state.connection().map_err(AuthFailure::Internal)?;
    let record = connection
        .query_row(
            "SELECT client_id,scopes_json FROM mcp_clients WHERE token_hash=?1 AND state='active' AND expires_at_utc > strftime('%Y-%m-%dT%H:%M:%fZ','now')",
            params![token_hash],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| AuthFailure::Internal(error.into()))?
        .ok_or(AuthFailure::Unauthorized)?;

    let scopes = serde_json::from_str::<Vec<String>>(&record.1)
        .map_err(|error| AuthFailure::Internal(error.into()))?
        .into_iter()
        .collect::<HashSet<_>>();
    let current_minute = Utc::now().timestamp() / 60;
    let transaction = connection
        .transaction()
        .map_err(|error| AuthFailure::Internal(error.into()))?;
    transaction
        .execute(
            "INSERT INTO mcp_rate_limits (client_id,window_epoch_minute,request_count) VALUES (?1,?2,1) ON CONFLICT(client_id,window_epoch_minute) DO UPDATE SET request_count=request_count+1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
            params![record.0, current_minute],
        )
        .map_err(|error| AuthFailure::Internal(error.into()))?;
    let request_count: i64 = transaction
        .query_row(
            "SELECT request_count FROM mcp_rate_limits WHERE client_id=?1 AND window_epoch_minute=?2",
            params![record.0, current_minute],
            |row| row.get(0),
        )
        .map_err(|error| AuthFailure::Internal(error.into()))?;
    if request_count > MAX_MCP_REQUESTS_PER_MINUTE {
        transaction
            .rollback()
            .map_err(|error| AuthFailure::Internal(error.into()))?;
        return Err(AuthFailure::RateLimited);
    }
    transaction
        .execute(
            "UPDATE mcp_clients SET last_used_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),request_count=request_count+1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE client_id=?1",
            params![record.0],
        )
        .map_err(|error| AuthFailure::Internal(error.into()))?;
    transaction
        .commit()
        .map_err(|error| AuthFailure::Internal(error.into()))?;

    Ok(McpClientContext {
        client_id: record.0,
        scopes,
    })
}

async fn dispatch_request(
    state: Arc<AppState>,
    client: &McpClientContext,
    method: &str,
    params: Value,
) -> Result<(Value, &'static str), RpcFailure> {
    match method {
        "initialize" => Ok((initialize_result(&params), "protocol")),
        "ping" => Ok((json!({}), "protocol")),
        "notifications/initialized" | "notifications/cancelled" => Ok((Value::Null, "protocol")),
        "tools/list" => Ok((
            json!({ "tools": tool_definitions(&client.scopes) }),
            "protocol",
        )),
        "tools/call" => call_tool(state, client, params).await,
        "resources/list" => Ok((
            json!({ "resources": resource_definitions(&client.scopes) }),
            "protocol",
        )),
        "resources/templates/list" => Ok((
            json!({ "resourceTemplates": resource_templates(&client.scopes) }),
            "protocol",
        )),
        "resources/read" => read_resource(state, client, params).await,
        "prompts/list" => Ok((json!({ "prompts": [] }), "protocol")),
        _ => Err(RpcFailure {
            code: -32601,
            message: format!("MCP method '{method}' is not supported."),
            capability: "protocol",
            detail_code: "method_not_found",
        }),
    }
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(MCP_PROTOCOL_LATEST);
    let negotiated = MCP_PROTOCOL_VERSIONS
        .iter()
        .find(|version| **version == requested)
        .copied()
        .unwrap_or(MCP_PROTOCOL_LATEST);
    json!({
        "protocolVersion": negotiated,
        "capabilities": {
            "resources": { "subscribe": false, "listChanged": false },
            "tools": { "listChanged": false },
            "prompts": { "listChanged": false }
        },
        "serverInfo": {
            "name": "Microgifter HomeServer",
            "title": "Microgifter HomeServer — Read-only Local MCP",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "This server exposes only client-scoped, read-only local HomeServer status, model inventory, cloud status, and cited Knowledge Vault retrieval. It cannot modify files, models, cloud records, commerce, campaigns, rewards, claims, or settings."
    })
}

async fn call_tool(
    state: Arc<AppState>,
    client: &McpClientContext,
    params: Value,
) -> Result<(Value, &'static str), RpcFailure> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcFailure::invalid_params("Tool name is required.", "protocol"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let (payload, capability) = match name {
        "homeserver_status" => {
            require_scope(client, "system.read")?;
            (
                serde_json::to_value(state.snapshot())
                    .map_err(|error| RpcFailure::internal(error.into(), "system.read"))?,
                "system.read",
            )
        }
        "homeserver_cloud_status" => {
            require_scope(client, "cloud.read")?;
            let state_for_cloud = state.clone();
            let snapshot = tokio::task::spawn_blocking(move || state_for_cloud.cloud_snapshot())
                .await
                .map_err(|error| RpcFailure::internal(error.into(), "cloud.read"))?
                .map_err(|error| RpcFailure::internal(error, "cloud.read"))?;
            (
                serde_json::to_value(snapshot)
                    .map_err(|error| RpcFailure::internal(error.into(), "cloud.read"))?,
                "cloud.read",
            )
        }
        "homeserver_model_inventory" => {
            require_scope(client, "models.read")?;
            let snapshot = model_center::snapshot(state.clone())
                .await
                .map_err(|error| RpcFailure::internal(error, "models.read"))?;
            (
                serde_json::to_value(snapshot)
                    .map_err(|error| RpcFailure::internal(error.into(), "models.read"))?,
                "models.read",
            )
        }
        "homeserver_knowledge_search" => {
            require_scope(client, "knowledge.search")?;
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned();
            if query.is_empty() || query.chars().count() > MAX_MCP_QUERY_CHARS {
                return Err(RpcFailure::invalid_params(
                    "Knowledge search query must contain 1 to 200 characters.",
                    "knowledge.search",
                ));
            }
            let mode = arguments
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("hybrid")
                .to_owned();
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(1, u64::from(MAX_MCP_SEARCH_RESULTS)) as u32;
            let result = semantic_vault::semantic_search(
                state.clone(),
                semantic_vault::SemanticSearchRequest {
                    query,
                    limit: Some(limit),
                    mode: Some(mode),
                },
            )
            .await
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "knowledge.search"))?;
            (
                serde_json::to_value(result)
                    .map_err(|error| RpcFailure::internal(error.into(), "knowledge.search"))?,
                "knowledge.search",
            )
        }
        "homeserver_knowledge_document" => {
            require_scope(client, "knowledge.read")?;
            let document_id = arguments
                .get("document_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RpcFailure::invalid_params("document_id is required.", "knowledge.read")
                })?
                .to_owned();
            let state_for_document = state.clone();
            let payload = tokio::task::spawn_blocking(move || {
                knowledge_document(&state_for_document, &document_id)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "knowledge.read"))?
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "knowledge.read"))?;
            (payload, "knowledge.read")
        }
        _ => {
            return Err(RpcFailure {
                code: -32602,
                message: format!("Read-only MCP tool '{name}' is not available."),
                capability: "protocol",
                detail_code: "tool_not_found",
            })
        }
    };
    Ok((tool_result(payload), capability))
}

async fn read_resource(
    state: Arc<AppState>,
    client: &McpClientContext,
    params: Value,
) -> Result<(Value, &'static str), RpcFailure> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| RpcFailure::invalid_params("Resource URI is required.", "protocol"))?;
    let (payload, capability) = match uri {
        "homeserver://status" => {
            require_scope(client, "system.read")?;
            (
                serde_json::to_value(state.snapshot())
                    .map_err(|error| RpcFailure::internal(error.into(), "system.read"))?,
                "system.read",
            )
        }
        "homeserver://cloud" => {
            require_scope(client, "cloud.read")?;
            let state_for_cloud = state.clone();
            let snapshot = tokio::task::spawn_blocking(move || state_for_cloud.cloud_snapshot())
                .await
                .map_err(|error| RpcFailure::internal(error.into(), "cloud.read"))?
                .map_err(|error| RpcFailure::internal(error, "cloud.read"))?;
            (
                serde_json::to_value(snapshot)
                    .map_err(|error| RpcFailure::internal(error.into(), "cloud.read"))?,
                "cloud.read",
            )
        }
        "homeserver://models" => {
            require_scope(client, "models.read")?;
            let snapshot = model_center::snapshot(state.clone())
                .await
                .map_err(|error| RpcFailure::internal(error, "models.read"))?;
            (
                serde_json::to_value(snapshot)
                    .map_err(|error| RpcFailure::internal(error.into(), "models.read"))?,
                "models.read",
            )
        }
        "homeserver://knowledge/documents" => {
            require_scope(client, "knowledge.read")?;
            let state_for_catalog = state.clone();
            let catalog =
                tokio::task::spawn_blocking(move || knowledge_catalog(&state_for_catalog))
                    .await
                    .map_err(|error| RpcFailure::internal(error.into(), "knowledge.read"))?
                    .map_err(|error| RpcFailure::internal(error, "knowledge.read"))?;
            (catalog, "knowledge.read")
        }
        _ if uri.starts_with("homeserver://knowledge/document/") => {
            require_scope(client, "knowledge.read")?;
            let document_id = uri
                .strip_prefix("homeserver://knowledge/document/")
                .unwrap_or_default()
                .to_owned();
            let state_for_document = state.clone();
            let payload = tokio::task::spawn_blocking(move || {
                knowledge_document(&state_for_document, &document_id)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "knowledge.read"))?
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "knowledge.read"))?;
            (payload, "knowledge.read")
        }
        _ => {
            return Err(RpcFailure::invalid_params(
                "Resource URI is not available to this HomeServer MCP client.",
                "protocol",
            ))
        }
    };
    let text = serde_json::to_string(&payload)
        .map_err(|error| RpcFailure::internal(error.into(), capability))?;
    Ok((
        json!({
            "contents": [{
                "uri": uri,
                "mimeType": "application/json",
                "text": text
            }]
        }),
        capability,
    ))
}

fn require_scope(client: &McpClientContext, scope: &'static str) -> Result<(), RpcFailure> {
    if client.scopes.contains(scope) {
        Ok(())
    } else {
        Err(RpcFailure::denied(scope))
    }
}

fn tool_definitions(scopes: &HashSet<String>) -> Vec<Value> {
    let mut tools = Vec::new();
    if scopes.contains("system.read") {
        tools.push(read_only_tool(
            "homeserver_status",
            "Read the local HomeServer health snapshot.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ));
    }
    if scopes.contains("cloud.read") {
        tools.push(read_only_tool(
            "homeserver_cloud_status",
            "Read pairing and synchronization status without accessing credentials.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ));
    }
    if scopes.contains("models.read") {
        tools.push(read_only_tool(
            "homeserver_model_inventory",
            "Read approved local model runtime, inventory, defaults, and bounded hardware guidance.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ));
    }
    if scopes.contains("knowledge.search") {
        tools.push(read_only_tool(
            "homeserver_knowledge_search",
            "Search approved local Knowledge Vault documents with cited keyword, semantic, or hybrid retrieval.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "minLength": 1, "maxLength": MAX_MCP_QUERY_CHARS },
                    "mode": { "type": "string", "enum": ["keyword", "semantic", "hybrid"], "default": "hybrid" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_MCP_SEARCH_RESULTS, "default": 10 }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ));
    }
    if scopes.contains("knowledge.read") {
        tools.push(read_only_tool(
            "homeserver_knowledge_document",
            "Read bounded indexed text and metadata for one approved Knowledge Vault document.",
            json!({
                "type": "object",
                "properties": {
                    "document_id": { "type": "string", "minLength": 1, "maxLength": 80 }
                },
                "required": ["document_id"],
                "additionalProperties": false
            }),
        ));
    }
    tools
}

fn read_only_tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "title": name.replace('_', " "),
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

fn resource_definitions(scopes: &HashSet<String>) -> Vec<Value> {
    let mut resources = Vec::new();
    if scopes.contains("system.read") {
        resources.push(resource_definition(
            "homeserver://status",
            "HomeServer status",
            "Local service health and pending-work summary.",
        ));
    }
    if scopes.contains("cloud.read") {
        resources.push(resource_definition(
            "homeserver://cloud",
            "Cloud synchronization status",
            "Pairing, scopes, queue, and last successful signed synchronization.",
        ));
    }
    if scopes.contains("models.read") {
        resources.push(resource_definition(
            "homeserver://models",
            "Local model inventory",
            "Fixed-loopback Ollama runtime and approved installed-model inventory.",
        ));
    }
    if scopes.contains("knowledge.read") {
        resources.push(resource_definition(
            "homeserver://knowledge/documents",
            "Knowledge Vault document catalog",
            "Metadata for approved HomeServer-managed documents.",
        ));
    }
    resources
}

fn resource_definition(uri: &str, name: &str, description: &str) -> Value {
    json!({
        "uri": uri,
        "name": name,
        "title": name,
        "description": description,
        "mimeType": "application/json"
    })
}

fn resource_templates(scopes: &HashSet<String>) -> Vec<Value> {
    if !scopes.contains("knowledge.read") {
        return Vec::new();
    }
    vec![json!({
        "uriTemplate": "homeserver://knowledge/document/{document_id}",
        "name": "Knowledge Vault document",
        "title": "Knowledge Vault document",
        "description": "Bounded indexed text and metadata for one approved document.",
        "mimeType": "application/json"
    })]
}

fn tool_result(payload: Value) -> Value {
    let text = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_owned());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": payload,
        "isError": false
    })
}

fn runtime_snapshot(state: &AppState) -> Result<McpRuntimeSnapshot> {
    let connection = state.connection()?;
    let clients = list_clients(&connection)?;
    let active_clients = clients
        .iter()
        .filter(|client| client.state == "active")
        .count() as u64;
    Ok(McpRuntimeSnapshot {
        state: if active_clients > 0 {
            "ready".to_owned()
        } else {
            "waiting_for_client".to_owned()
        },
        endpoint: MCP_ENDPOINT.to_owned(),
        transport: vec!["streamable_http".to_owned(), "stdio_bridge".to_owned()],
        protocol_versions: MCP_PROTOCOL_VERSIONS
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        bridge_file_name: MCP_BRIDGE_FILE_NAME.to_owned(),
        active_clients,
        clients,
        scopes: ALLOWED_SCOPES
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        tools: vec![
            "homeserver_status".to_owned(),
            "homeserver_cloud_status".to_owned(),
            "homeserver_model_inventory".to_owned(),
            "homeserver_knowledge_search".to_owned(),
            "homeserver_knowledge_document".to_owned(),
        ],
        requests_per_minute: MAX_MCP_REQUESTS_PER_MINUTE as u64,
        read_only: true,
        local_only: true,
    })
}

fn create_client(state: &AppState, request: CreateMcpClientRequest) -> Result<McpClientCredential> {
    let display_name = sanitize_display_name(&request.display_name)?;
    let scopes = normalize_scopes(&request.scopes)?;
    let expires_days = request.expires_days.unwrap_or(90).clamp(1, 365);
    let connection = state.connection()?;
    let existing_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM mcp_clients WHERE state='active' AND expires_at_utc > strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        existing_count < MAX_MCP_CLIENTS,
        "HomeServer already has the maximum number of active MCP clients"
    );

    let client_id = Uuid::new_v4().to_string();
    let mut secret_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut secret_bytes);
    let token = format!("mghs_mcp_{}", URL_SAFE_NO_PAD.encode(secret_bytes));
    let token_hash = hash_token(&token);
    let token_hint = token_hint(&token);
    let expires_at_utc = (Utc::now() + ChronoDuration::days(i64::from(expires_days)))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    connection.execute(
        "INSERT INTO mcp_clients (client_id,display_name,token_hash,token_hint,scopes_json,state,expires_at_utc) VALUES (?1,?2,?3,?4,?5,'active',?6)",
        params![
            client_id,
            display_name,
            token_hash,
            token_hint,
            serde_json::to_string(&scopes)?,
            expires_at_utc,
        ],
    )?;
    let client = client_by_id(&connection, &client_id)?;
    Ok(McpClientCredential {
        client,
        token,
        endpoint: MCP_ENDPOINT.to_owned(),
        bridge_file_name: MCP_BRIDGE_FILE_NAME.to_owned(),
        message: "MCP client credential created. Copy the token now; HomeServer stores only its SHA-256 hash and cannot reveal it again.".to_owned(),
    })
}

fn revoke_client(state: &AppState, request: RevokeMcpClientRequest) -> Result<McpActionResult> {
    ensure!(
        request.confirmation == "REVOKE",
        "type REVOKE to revoke the MCP client"
    );
    let connection = state.connection()?;
    let affected = connection.execute(
        "UPDATE mcp_clients SET state='revoked',revoked_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE client_id=?1 AND state='active'",
        params![request.client_id],
    )?;
    ensure!(affected == 1, "active MCP client was not found");
    let client = client_by_id(&connection, &request.client_id)?;
    Ok(McpActionResult {
        client,
        message: "MCP client access was revoked immediately.".to_owned(),
    })
}

fn list_clients(connection: &Connection) -> Result<Vec<McpClientSummary>> {
    let sql = format!(
        "SELECT {CLIENT_COLUMNS} FROM mcp_clients ORDER BY updated_at_utc DESC,client_id DESC LIMIT 100"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], map_client)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn client_by_id(connection: &Connection, client_id: &str) -> Result<McpClientSummary> {
    let sql = format!("SELECT {CLIENT_COLUMNS} FROM mcp_clients WHERE client_id=?1");
    connection
        .query_row(&sql, params![client_id], map_client)
        .context("MCP client was not found")
}

fn map_client(row: &Row<'_>) -> rusqlite::Result<McpClientSummary> {
    let scopes_json: String = row.get(3)?;
    let stored_state: String = row.get(4)?;
    let expires_at_utc: String = row.get(5)?;
    let expired = DateTime::parse_from_rfc3339(&expires_at_utc)
        .map(|value| value.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true);
    let effective_state = if stored_state == "active" && expired {
        "expired".to_owned()
    } else {
        stored_state
    };
    Ok(McpClientSummary {
        client_id: row.get(0)?,
        display_name: row.get(1)?,
        token_hint: row.get(2)?,
        scopes: serde_json::from_str(&scopes_json).unwrap_or_default(),
        state: effective_state,
        expires_at_utc,
        last_used_at_utc: row.get(6)?,
        request_count: row.get::<_, i64>(7)?.max(0) as u64,
        created_at_utc: row.get(8)?,
        updated_at_utc: row.get(9)?,
        revoked_at_utc: row.get(10)?,
    })
}

fn normalize_scopes(requested: &[String]) -> Result<Vec<String>> {
    let mut scopes = if requested.is_empty() {
        ALLOWED_SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect::<Vec<_>>()
    } else {
        requested
            .iter()
            .map(|scope| scope.trim().to_ascii_lowercase())
            .collect::<Vec<_>>()
    };
    scopes.sort();
    scopes.dedup();
    ensure!(
        !scopes.is_empty(),
        "at least one read-only MCP scope is required"
    );
    ensure!(
        scopes
            .iter()
            .all(|scope| ALLOWED_SCOPES.contains(&scope.as_str())),
        "MCP client requested an unsupported scope"
    );
    Ok(scopes)
}

fn sanitize_display_name(value: &str) -> Result<String> {
    let cleaned = value
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect::<String>()
        .trim()
        .to_owned();
    ensure!(
        cleaned.chars().count() >= 3,
        "MCP client name must contain at least 3 characters"
    );
    Ok(cleaned)
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn token_hint(token: &str) -> String {
    let prefix = token.chars().take(12).collect::<String>();
    let suffix = token.chars().rev().take(4).collect::<String>();
    format!("{prefix}…{}", suffix.chars().rev().collect::<String>())
}

fn knowledge_catalog(state: &AppState) -> Result<Value> {
    let connection = state.connection()?;
    let mut statement = connection.prepare(
        "SELECT document_id,title,file_name,content_type,size_bytes,state,tags_json,indexed_at_utc,updated_at_utc FROM vault_documents ORDER BY updated_at_utc DESC,document_id DESC LIMIT 200",
    )?;
    let rows = statement.query_map([], |row| {
        let tags_json: String = row.get(6)?;
        Ok(json!({
            "document_id": row.get::<_, String>(0)?,
            "title": row.get::<_, String>(1)?,
            "file_name": row.get::<_, String>(2)?,
            "content_type": row.get::<_, String>(3)?,
            "size_bytes": row.get::<_, i64>(4)?.max(0),
            "state": row.get::<_, String>(5)?,
            "tags": serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default(),
            "indexed_at_utc": row.get::<_, Option<String>>(7)?,
            "updated_at_utc": row.get::<_, String>(8)?,
        }))
    })?;
    let documents = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let count = documents.len();
    Ok(json!({
        "documents": documents,
        "count": count,
        "local_only": true
    }))
}

fn knowledge_document(state: &AppState, document_id: &str) -> Result<Value> {
    ensure!(
        !document_id.is_empty()
            && document_id.len() <= 80
            && document_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-'),
        "document_id is invalid"
    );
    let connection = state.connection()?;
    let row = connection
        .query_row(
            "SELECT document_id,title,file_name,content_type,size_bytes,state,tags_json,indexed_text,indexed_at_utc,updated_at_utc FROM vault_documents WHERE document_id=?1",
            params![document_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()?
        .context("Knowledge Vault document was not found")?;
    let (indexed_text, truncated) = truncate_chars(&row.7, MAX_MCP_DOCUMENT_CHARS);
    Ok(json!({
        "document_id": row.0,
        "title": row.1,
        "file_name": row.2,
        "content_type": row.3,
        "size_bytes": row.4.max(0),
        "state": row.5,
        "tags": serde_json::from_str::<Vec<String>>(&row.6).unwrap_or_default(),
        "indexed_text": indexed_text,
        "truncated": truncated,
        "indexed_at_utc": row.8,
        "updated_at_utc": row.9,
        "local_only": true
    }))
}

fn truncate_chars(value: &str, maximum: usize) -> (String, bool) {
    let mut output = String::new();
    let mut iterator = value.chars();
    for _ in 0..maximum {
        let Some(character) = iterator.next() else {
            return (output, false);
        };
        output.push(character);
    }
    (output, iterator.next().is_some())
}

fn rpc_success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

fn json_rpc_http_response(value: Value) -> Response {
    let bytes = serde_json::to_vec(&value).unwrap_or_else(|_| {
        br#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal JSON-RPC serialization error."}}"#.to_vec()
    });
    if bytes.len() > MAX_MCP_RESPONSE_BYTES {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(rpc_error(
                Value::Null,
                -32603,
                "MCP response exceeded the HomeServer size limit.",
            )),
        )
            .into_response();
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .header("mcp-protocol-version", MCP_PROTOCOL_LATEST)
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn response_size(response: &Response) -> usize {
    response
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
}

fn auth_response(error: AuthFailure) -> Response {
    match error {
        AuthFailure::Unauthorized => {
            let mut response = (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "ok": false,
                    "error": "mcp_authorization_required",
                    "message": "A valid active HomeServer MCP client token is required."
                })),
            )
                .into_response();
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"Microgifter HomeServer MCP\""),
            );
            response
        }
        AuthFailure::RateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "ok": false,
                "error": "mcp_rate_limited",
                "message": "HomeServer MCP client exceeded 120 requests per minute."
            })),
        )
            .into_response(),
        AuthFailure::Internal(error) => {
            tracing::warn!(?error, "MCP authorization check failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "ok": false,
                    "error": "mcp_authorization_failed",
                    "message": "HomeServer could not validate the MCP client."
                })),
            )
                .into_response()
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_audit_async(
    state: Arc<AppState>,
    client_id: Option<String>,
    method: impl Into<String>,
    capability: impl Into<String>,
    outcome: &'static str,
    detail_code: &'static str,
    request_bytes: usize,
    response_bytes: usize,
    started: Instant,
) {
    let method = method.into();
    let capability = capability.into();
    let duration_ms = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
    let _audit_task = tokio::task::spawn_blocking(move || {
        let connection = state.connection()?;
        connection.execute(
            "INSERT INTO mcp_audit_receipts (receipt_id,client_id,method,capability,outcome,detail_code,request_bytes,response_bytes,duration_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                Uuid::new_v4().to_string(),
                client_id,
                method.chars().take(120).collect::<String>(),
                capability.chars().take(120).collect::<String>(),
                outcome,
                detail_code,
                request_bytes.min(i64::MAX as usize) as i64,
                response_bytes.min(i64::MAX as usize) as i64,
                duration_ms,
            ],
        )?;
        Ok::<(), anyhow::Error>(())
    });
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("mcp_task_failed", error.into())
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(?error, code, "HomeServer MCP control operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: "HomeServer could not complete the MCP control operation.".to_owned(),
        }),
    )
}

fn action_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
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
    fn tokens_are_hashed_and_hints_do_not_reveal_secrets() {
        let token = "mghs_mcp_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG";
        assert_eq!(hash_token(token).len(), 64);
        let hint = token_hint(token);
        assert!(hint.starts_with("mghs_mcp_abc"));
        assert!(hint.ends_with("DEFG"));
        assert!(!hint.contains("mnopqrstuvwxyz"));
    }

    #[test]
    fn scopes_are_read_only_and_deduplicated() {
        let scopes = normalize_scopes(&[
            "knowledge.read".to_owned(),
            "system.read".to_owned(),
            "knowledge.read".to_owned(),
        ])
        .expect("read-only scopes should be accepted");
        assert_eq!(scopes, vec!["knowledge.read", "system.read"]);
        assert!(normalize_scopes(&["models.write".to_owned()]).is_err());
    }

    #[test]
    fn document_output_is_bounded() {
        let (value, truncated) = truncate_chars(
            &"x".repeat(MAX_MCP_DOCUMENT_CHARS + 1),
            MAX_MCP_DOCUMENT_CHARS,
        );
        assert_eq!(value.chars().count(), MAX_MCP_DOCUMENT_CHARS);
        assert!(truncated);
    }

    #[test]
    fn tools_are_marked_read_only() {
        let scopes = ALLOWED_SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect::<HashSet<_>>();
        let tools = tool_definitions(&scopes);
        assert_eq!(tools.len(), 5);
        assert!(tools.iter().all(|tool| {
            tool.pointer("/annotations/readOnlyHint") == Some(&Value::Bool(true))
                && tool.pointer("/annotations/destructiveHint") == Some(&Value::Bool(false))
        }));
    }
}
