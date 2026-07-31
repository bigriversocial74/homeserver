use crate::{app::cloud_registry, model_center, operational_data, semantic_vault, AppState};
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use keyring::Entry;
use rand::{rngs::OsRng, RngCore};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroize;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0029_unified_agent_orchestration.sql");
const MIGRATION_KEY: &str = "0029_unified_agent_orchestration";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServerAgentMcp";
const CALLBACK_PATH: &str = "/oauth/microgifter/callback";
const CALLBACK_URI: &str = "http://127.0.0.1:47831/oauth/microgifter/callback";
const LOCAL_API_HOST: &str = "127.0.0.1:47831";
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const MAX_CONTROL_BODY_BYTES: usize = 256 * 1024;
const MAX_MCP_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MCP_TOOLS: usize = 250;
const MAX_MCP_AUTOMATIC_CALLS: usize = 3;
const MAX_CONTEXT_RECEIPTS: i64 = 100_000;
const MAX_MCP_RECEIPTS: i64 = 100_000;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolSummary {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub annotations: Value,
    pub operation_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SiteIntegrationSummary {
    pub connection_id: String,
    pub provider_key: String,
    pub resource_uri: String,
    pub authorization_server: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub state: String,
    pub token_expires_at_utc: Option<String>,
    pub tools: Vec<McpToolSummary>,
    pub last_tool_sync_utc: Option<String>,
    pub last_success_utc: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentGuidanceItem {
    pub key: String,
    pub title: String,
    pub message: String,
    pub action_label: String,
    pub action_target: String,
    pub priority: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentIntegrationSnapshot {
    pub schema: String,
    pub system: Value,
    pub knowledge: Value,
    pub models: Value,
    pub operational_data: Value,
    pub cloud_connections: Value,
    pub backups: Value,
    pub site_integrations: Vec<SiteIntegrationSummary>,
    pub guidance: Vec<AgentGuidanceItem>,
    pub active_prompt: Option<AgentGuidanceItem>,
    pub complete_control_is_user_authorized: bool,
    pub read_tools_may_run_automatically: bool,
    pub state_changing_tools_require_authority: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpGroundingRecord {
    pub connection_id: String,
    pub tool_name: String,
    pub operation_class: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnifiedMcpGrounding {
    pub records: Vec<McpGroundingRecord>,
    pub available_tools: Vec<McpToolSummary>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigureSiteIntegrationRequest {
    pub connection_id: String,
    pub resource_uri: Option<String>,
    pub authorization_server: Option<String>,
    pub client_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionReferenceRequest {
    pub connection_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorizationStartResult {
    pub connection_id: String,
    pub authorization_url: String,
    pub redirect_uri: String,
    pub expires_at_utc: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CallMcpToolRequest {
    pub connection_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: Value,
    pub confirmation: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DismissGuidanceRequest {
    pub prompt_key: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMcpCredentials {
    access_token: Option<String>,
    refresh_token: Option<String>,
    pending_verifier: Option<String>,
}

impl Drop for StoredMcpCredentials {
    fn drop(&mut self) {
        if let Some(value) = self.access_token.as_mut() {
            value.zeroize();
        }
        if let Some(value) = self.refresh_token.as_mut() {
            value.zeroize();
        }
        if let Some(value) = self.pending_verifier.as_mut() {
            value.zeroize();
        }
    }
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
    refresh_token: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RpcEnvelope {
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

#[derive(Debug, Clone)]
struct IntegrationRecord {
    summary: SiteIntegrationSummary,
    credential_key: String,
    pending_expires_at_utc: Option<String>,
}

#[derive(Clone)]
struct McpHttpClient {
    client: reqwest::Client,
}

impl McpHttpClient {
    fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(format!(
                    "Microgifter-HomeServer-Agent/{}",
                    env!("CARGO_PKG_VERSION")
                ))
                .build()?,
        })
    }

    async fn token_exchange(
        &self,
        record: &IntegrationRecord,
        code: &str,
        verifier: &str,
    ) -> Result<OAuthTokenResponse> {
        let token_url = oauth_endpoint(&record.summary.authorization_server, "/oauth/token.php")?;
        let response = self
            .client
            .post(token_url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("client_id", record.summary.client_id.as_str()),
                ("redirect_uri", record.summary.redirect_uri.as_str()),
                ("code_verifier", verifier),
                ("resource", record.summary.resource_uri.as_str()),
            ])
            .send()
            .await
            .context("unable to exchange the Microgifter MCP authorization code")?;
        decode_oauth_response(response).await
    }

    async fn refresh_token(
        &self,
        record: &IntegrationRecord,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse> {
        let token_url = oauth_endpoint(&record.summary.authorization_server, "/oauth/token.php")?;
        let response = self
            .client
            .post(token_url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", record.summary.client_id.as_str()),
                ("resource", record.summary.resource_uri.as_str()),
            ])
            .send()
            .await
            .context("unable to refresh the Microgifter MCP authorization")?;
        decode_oauth_response(response).await
    }

    async fn rpc(
        &self,
        record: &IntegrationRecord,
        access_token: &str,
        method: &str,
        params: Value,
        notification: bool,
    ) -> Result<Value> {
        let body = if notification {
            json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            })
        } else {
            json!({
                "jsonrpc": "2.0",
                "id": Uuid::new_v4().to_string(),
                "method": method,
                "params": params,
            })
        };
        let response = self
            .client
            .post(&record.summary.resource_uri)
            .bearer_auth(access_token)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION)
            .json(&body)
            .send()
            .await
            .context("Microgifter MCP request failed")?;
        if notification && response.status().is_success() {
            return Ok(json!({}));
        }
        decode_rpc_response(response).await
    }

    async fn initialize(&self, record: &IntegrationRecord, access_token: &str) -> Result<()> {
        let _ = self
            .rpc(
                record,
                access_token,
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "Microgifter HomeServer Agent",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
                false,
            )
            .await?;
        let _ = self
            .rpc(
                record,
                access_token,
                "notifications/initialized",
                json!({}),
                true,
            )
            .await?;
        Ok(())
    }

    async fn list_tools(
        &self,
        record: &IntegrationRecord,
        access_token: &str,
    ) -> Result<Vec<McpToolSummary>> {
        self.initialize(record, access_token).await?;
        let result = self
            .rpc(record, access_token, "tools/list", json!({}), false)
            .await?;
        let values = result
            .get("tools")
            .and_then(Value::as_array)
            .context("Microgifter MCP tool discovery did not return tools")?;
        ensure!(
            values.len() <= MAX_MCP_TOOLS,
            "Microgifter MCP returned too many tools"
        );
        values.iter().map(parse_tool).collect()
    }

    async fn call_tool(
        &self,
        record: &IntegrationRecord,
        access_token: &str,
        tool_name: &str,
        arguments: &Value,
    ) -> Result<Value> {
        self.initialize(record, access_token).await?;
        self.rpc(
            record,
            access_token,
            "tools/call",
            json!({ "name": tool_name, "arguments": arguments }),
            false,
        )
        .await
    }
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        count == 1,
        "unified Agent migration is not registered exactly once"
    );
    health_check(connection)?;
    maintain_history(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    for table in [
        "agent_site_integrations",
        "agent_engagement_state",
        "agent_context_receipts",
        "agent_mcp_invocation_receipts",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    let context_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM agent_context_receipts", [], |row| {
            row.get(0)
        })?;
    ensure!(
        context_count <= MAX_CONTEXT_RECEIPTS,
        "Agent context receipt retention requires archival"
    );
    let mcp_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_mcp_invocation_receipts",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        mcp_count <= MAX_MCP_RECEIPTS,
        "Agent MCP receipt retention requires archival"
    );
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/agent/integrations", get(snapshot_handler))
        .route("/v1/agent/integrations/configure", post(configure_handler))
        .route("/v1/agent/integrations/authorize", post(authorize_handler))
        .route("/v1/agent/integrations/tools", post(refresh_tools_handler))
        .route("/v1/agent/integrations/call", post(call_tool_handler))
        .route("/v1/agent/guidance/dismiss", post(dismiss_guidance_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub fn oauth_callback_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(CALLBACK_PATH, get(oauth_callback))
        .with_state(state)
}

async fn snapshot_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<AgentIntegrationSnapshot> {
    integration_snapshot(state)
        .await
        .map(Json)
        .map_err(|error| internal_error("agent_integrations_snapshot_failed", error))
}

async fn configure_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConfigureSiteIntegrationRequest>,
) -> ApiResult<AgentIntegrationSnapshot> {
    let task_state = state.clone();
    tokio::task::spawn_blocking(move || configure_site_integration(&task_state, request))
        .await
        .map_err(task_error)?
        .map_err(|error| action_error("agent_integration_configure_rejected", error))?;
    integration_snapshot(state)
        .await
        .map(Json)
        .map_err(|error| internal_error("agent_integrations_snapshot_failed", error))
}

async fn authorize_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionReferenceRequest>,
) -> ApiResult<AuthorizationStartResult> {
    tokio::task::spawn_blocking(move || begin_authorization(&state, &request.connection_id))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("agent_mcp_authorization_rejected", error))
}

async fn refresh_tools_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ConnectionReferenceRequest>,
) -> ApiResult<SiteIntegrationSummary> {
    refresh_tools(state, &request.connection_id)
        .await
        .map(Json)
        .map_err(|error| action_error("agent_mcp_tool_discovery_failed", error))
}

async fn call_tool_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CallMcpToolRequest>,
) -> ApiResult<Value> {
    call_tool_with_authority(state, request)
        .await
        .map(Json)
        .map_err(|error| action_error("agent_mcp_tool_call_rejected", error))
}

async fn dismiss_guidance_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DismissGuidanceRequest>,
) -> ApiResult<AgentIntegrationSnapshot> {
    let task_state = state.clone();
    tokio::task::spawn_blocking(move || dismiss_guidance(&task_state, &request.prompt_key))
        .await
        .map_err(task_error)?
        .map_err(|error| action_error("agent_guidance_dismiss_rejected", error))?;
    integration_snapshot(state)
        .await
        .map(Json)
        .map_err(|error| internal_error("agent_integrations_snapshot_failed", error))
}

async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<OAuthCallbackQuery>,
) -> Html<String> {
    let result = complete_authorization(state, &headers, query).await;
    match result {
        Ok(connection_name) => Html(callback_page(
            "Microgifter connected",
            &format!(
                "HomeServer can now discover and use authorized Microgifter MCP tools for {connection_name}. You may close this window."
            ),
            true,
        )),
        Err(error) => Html(callback_page(
            "Connection not completed",
            &format!("HomeServer could not complete MCP authorization: {error}"),
            false,
        )),
    }
}

async fn complete_authorization(
    state: Arc<AppState>,
    headers: &HeaderMap,
    query: OAuthCallbackQuery,
) -> Result<String> {
    let host_valid = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case(LOCAL_API_HOST));
    ensure!(host_valid, "OAuth callback host is invalid");
    if let Some(error) = query.error {
        bail!(
            "{}: {}",
            error,
            query
                .error_description
                .unwrap_or_else(|| "authorization was denied".to_owned())
        );
    }
    let code = bounded_text(
        query
            .code
            .as_deref()
            .context("authorization code is missing")?,
        1,
        2_000,
        "authorization code",
    )?;
    let state_value = bounded_text(
        query
            .state
            .as_deref()
            .context("authorization state is missing")?,
        32,
        500,
        "authorization state",
    )?;
    let state_hash = sha256_hex(state_value.as_bytes());
    let record = {
        let connection = state.connection()?;
        integration_by_pending_state(&connection, &state_hash)?
    };
    let pending_expiry = record
        .pending_expires_at_utc
        .as_deref()
        .context("authorization request has no expiration")?;
    ensure!(
        parse_time(pending_expiry)? > Utc::now(),
        "authorization request expired"
    );
    let mut credentials = load_credentials(&record.credential_key)?;
    let verifier = credentials
        .pending_verifier
        .take()
        .context("PKCE verifier is unavailable")?;
    let client = McpHttpClient::new()?;
    let token = client.token_exchange(&record, &code, &verifier).await?;
    validate_token_response(&token)?;
    credentials.access_token = Some(token.access_token);
    if let Some(refresh_token) = token.refresh_token {
        credentials.refresh_token = Some(refresh_token);
    }
    save_credentials(&record.credential_key, &credentials)?;
    let expires_at = timestamp(Utc::now() + ChronoDuration::seconds(token.expires_in as i64));
    {
        let connection = state.connection()?;
        connection.execute(
            "UPDATE agent_site_integrations SET state='connected',token_expires_at_utc=?1,pending_state_hash=NULL,pending_expires_at_utc=NULL,last_error=NULL,last_success_utc=?2,updated_at_utc=?2 WHERE connection_id=?3 AND pending_state_hash=?4",
            params![expires_at,now_string(),record.summary.connection_id,state_hash],
        )?;
    }
    let refreshed = refresh_tools(state.clone(), &record.summary.connection_id).await?;
    Ok(refreshed.connection_id)
}

pub async fn integration_snapshot(state: Arc<AppState>) -> Result<AgentIntegrationSnapshot> {
    let health = serde_json::to_value(state.snapshot())?;
    let knowledge = match semantic_vault::snapshot(&state) {
        Ok(snapshot) => serde_json::to_value(snapshot)?,
        Err(error) => json!({
            "state": "unavailable",
            "error": truncate_chars(&error.to_string(), 300)
        }),
    };
    let models = match model_center::snapshot(state.clone()).await {
        Ok(snapshot) => serde_json::to_value(snapshot)?,
        Err(error) => json!({
            "runtime": { "state": "unavailable" },
            "error": truncate_chars(&error.to_string(), 300)
        }),
    };
    let operational = {
        let task_state = state.clone();
        tokio::task::spawn_blocking(move || operational_data::snapshot_for_state(&task_state))
            .await
            .context("operational data snapshot task failed")??
    };
    let clouds = {
        let task_state = state.clone();
        tokio::task::spawn_blocking(move || task_state.cloud_connections_snapshot())
            .await
            .context("cloud connection snapshot task failed")??
    };
    let backups = {
        let task_state = state.clone();
        tokio::task::spawn_blocking(move || task_state.backup_catalog())
            .await
            .context("backup snapshot task failed")??
    };
    let integrations = {
        let task_state = state.clone();
        tokio::task::spawn_blocking(move || read_integrations(&task_state.connection()?))
            .await
            .context("site integration snapshot task failed")??
    };
    let dismissed = {
        let connection = state.connection()?;
        dismissed_prompt_keys(&connection)?
    };
    let guidance = build_guidance(
        &health,
        &knowledge,
        &models,
        &operational,
        &clouds,
        &backups,
        &integrations,
    );
    let active_prompt = guidance
        .iter()
        .find(|item| !dismissed.contains(&item.key))
        .cloned();
    Ok(AgentIntegrationSnapshot {
        schema: "homeserver.unified-agent-integrations.v1".to_owned(),
        system: health,
        knowledge,
        models,
        operational_data: serde_json::to_value(operational)?,
        cloud_connections: serde_json::to_value(clouds)?,
        backups: serde_json::to_value(backups)?,
        site_integrations: integrations,
        guidance,
        active_prompt,
        complete_control_is_user_authorized: true,
        read_tools_may_run_automatically: true,
        state_changing_tools_require_authority: true,
    })
}

pub async fn collect_mcp_grounding(
    state: Arc<AppState>,
    prompt: &str,
    connection_ids: &[String],
) -> UnifiedMcpGrounding {
    let integrations = match state
        .connection()
        .and_then(|connection| read_integrations(&connection))
    {
        Ok(values) => values
            .into_iter()
            .filter(|integration| {
                integration.state == "connected"
                    && (connection_ids.is_empty()
                        || connection_ids.contains(&integration.connection_id))
            })
            .collect::<Vec<_>>(),
        Err(error) => {
            return UnifiedMcpGrounding {
                records: Vec::new(),
                available_tools: Vec::new(),
                errors: vec![format!("MCP integration registry unavailable: {error}")],
            };
        }
    };
    let mut records = Vec::new();
    let mut available_tools = Vec::new();
    let mut errors = Vec::new();
    for integration in integrations {
        available_tools.extend(integration.tools.clone());
        let selected = select_automatic_tools(prompt, &integration.tools);
        for (tool, arguments) in selected.into_iter().take(MAX_MCP_AUTOMATIC_CALLS) {
            let request = CallMcpToolRequest {
                connection_id: integration.connection_id.clone(),
                tool_name: tool.name.clone(),
                arguments,
                confirmation: None,
            };
            match call_tool_with_authority(state.clone(), request).await {
                Ok(result) => records.push(McpGroundingRecord {
                    connection_id: integration.connection_id.clone(),
                    tool_name: tool.name,
                    operation_class: tool.operation_class,
                    result,
                }),
                Err(error) => errors.push(format!(
                    "{} on {} failed: {}",
                    tool.name, integration.connection_id, error
                )),
            }
        }
    }
    available_tools.sort_by(|left, right| left.name.cmp(&right.name));
    available_tools.dedup_by(|left, right| left.name == right.name);
    UnifiedMcpGrounding {
        records,
        available_tools,
        errors,
    }
}

pub fn record_context_receipt(
    state: &AppState,
    thread_id: Option<&str>,
    prompt: &str,
    source_keys: &[String],
    knowledge_hits: usize,
    operational_records: usize,
    mcp_tools: &[String],
    context_hash: &str,
    inference_state: &str,
    failure_code: Option<&str>,
) -> Result<String> {
    ensure!(
        ["not_started", "completed", "unavailable", "failed"].contains(&inference_state),
        "Agent context inference state is invalid"
    );
    ensure!(context_hash.len() == 64, "Agent context hash is invalid");
    let receipt_id = Uuid::new_v4().to_string();
    state.connection()?.execute(
        "INSERT INTO agent_context_receipts (receipt_id,thread_id,prompt_hash,source_keys_json,knowledge_hit_count,operational_record_count,mcp_tool_names_json,context_hash,inference_state,failure_code) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            receipt_id,
            thread_id,
            sha256_hex(prompt.as_bytes()),
            serde_json::to_string(source_keys)?,
            knowledge_hits as i64,
            operational_records as i64,
            serde_json::to_string(mcp_tools)?,
            context_hash,
            inference_state,
            failure_code,
        ],
    )?;
    Ok(receipt_id)
}

fn configure_site_integration(
    state: &AppState,
    request: ConfigureSiteIntegrationRequest,
) -> Result<()> {
    validate_uuid(&request.connection_id, "connection ID")?;
    let cloud = state
        .cloud_connections_snapshot()?
        .connections
        .into_iter()
        .find(|connection| connection.connection_id == request.connection_id)
        .context("paired cloud connection was not found")?;
    ensure!(
        cloud.provider_key == "microgifter",
        "MCP provider is not installed"
    );
    let resource_uri = normalize_resource_uri(
        request
            .resource_uri
            .as_deref()
            .unwrap_or("https://mcp.microgifter.com/mcp"),
    )?;
    let authorization_server = normalize_authorization_server(
        request
            .authorization_server
            .as_deref()
            .unwrap_or("https://microgifter.com"),
    )?;
    let client_id = bounded_text(&request.client_id, 1, 240, "OAuth client ID")?;
    let scopes = normalize_scopes(&request.scopes)?;
    let installation_id = crate::database::installation_id(&state.connection()?)?;
    let credential_key = format!("{installation_id}:agent-mcp:{}", request.connection_id);
    let connection = state.connection()?;
    connection.execute(
        "INSERT INTO agent_site_integrations (connection_id,provider_key,resource_uri,authorization_server,client_id,redirect_uri,scopes_json,credential_key,state) VALUES (?1,'microgifter',?2,?3,?4,?5,?6,?7,'configured') ON CONFLICT(connection_id) DO UPDATE SET resource_uri=excluded.resource_uri,authorization_server=excluded.authorization_server,client_id=excluded.client_id,redirect_uri=excluded.redirect_uri,scopes_json=excluded.scopes_json,credential_key=excluded.credential_key,state='configured',token_expires_at_utc=NULL,pending_state_hash=NULL,pending_expires_at_utc=NULL,tool_catalog_json='[]',last_tool_sync_utc=NULL,last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![
            request.connection_id,
            resource_uri,
            authorization_server,
            client_id,
            CALLBACK_URI,
            serde_json::to_string(&scopes)?,
            credential_key,
        ],
    )?;
    save_credentials(
        &credential_key,
        &StoredMcpCredentials {
            access_token: None,
            refresh_token: None,
            pending_verifier: None,
        },
    )
}

fn begin_authorization(state: &AppState, connection_id: &str) -> Result<AuthorizationStartResult> {
    validate_uuid(connection_id, "connection ID")?;
    let connection = state.connection()?;
    let record = integration_by_id(&connection, connection_id)?;
    ensure!(
        record.summary.state != "revoked",
        "MCP integration was revoked and must be configured again"
    );
    let state_value = random_secret(32);
    let verifier = random_secret(64);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state_hash = sha256_hex(state_value.as_bytes());
    let expires = timestamp(Utc::now() + ChronoDuration::minutes(10));
    let mut credentials = load_credentials_or_default(&record.credential_key)?;
    credentials.pending_verifier = Some(verifier);
    save_credentials(&record.credential_key, &credentials)?;
    connection.execute(
        "UPDATE agent_site_integrations SET state='authorization_pending',pending_state_hash=?1,pending_expires_at_utc=?2,last_error=NULL,updated_at_utc=?3 WHERE connection_id=?4",
        params![state_hash,expires,now_string(),connection_id],
    )?;
    let mut url = Url::parse(&oauth_endpoint(
        &record.summary.authorization_server,
        "/oauth/authorize.php",
    )?)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &record.summary.client_id);
        query.append_pair("redirect_uri", &record.summary.redirect_uri);
        query.append_pair("scope", &record.summary.scopes.join(" "));
        query.append_pair("state", &state_value);
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        query.append_pair("resource", &record.summary.resource_uri);
    }
    Ok(AuthorizationStartResult {
        connection_id: connection_id.to_owned(),
        authorization_url: url.to_string(),
        redirect_uri: CALLBACK_URI.to_owned(),
        expires_at_utc: expires,
    })
}

async fn refresh_tools(
    state: Arc<AppState>,
    connection_id: &str,
) -> Result<SiteIntegrationSummary> {
    validate_uuid(connection_id, "connection ID")?;
    let record = {
        let connection = state.connection()?;
        integration_by_id(&connection, connection_id)?
    };
    let access_token = ensure_access_token(state.clone(), &record).await?;
    let client = McpHttpClient::new()?;
    match client.list_tools(&record, &access_token).await {
        Ok(tools) => {
            let now = now_string();
            state.connection()?.execute(
                "UPDATE agent_site_integrations SET state='connected',tool_catalog_json=?1,last_tool_sync_utc=?2,last_success_utc=?2,last_error=NULL,updated_at_utc=?2 WHERE connection_id=?3",
                params![serde_json::to_string(&tools)?,now,connection_id],
            )?;
            Ok(integration_by_id(&state.connection()?, connection_id)?.summary)
        }
        Err(error) => {
            mark_integration_error(&state, connection_id, &error)?;
            Err(error)
        }
    }
}

async fn call_tool_with_authority(
    state: Arc<AppState>,
    request: CallMcpToolRequest,
) -> Result<Value> {
    validate_uuid(&request.connection_id, "connection ID")?;
    let tool_name = normalize_tool_name(&request.tool_name)?;
    ensure_json_object(&request.arguments, 128 * 1024, "MCP tool arguments")?;
    let record = {
        let connection = state.connection()?;
        integration_by_id(&connection, &request.connection_id)?
    };
    ensure!(
        record.summary.state == "connected",
        "MCP integration is not connected"
    );
    let tool = record
        .summary
        .tools
        .iter()
        .find(|tool| tool.name == tool_name)
        .cloned()
        .context("authorized MCP tool was not found")?;
    if tool.operation_class != "read" {
        ensure!(
            request.confirmation.as_deref() == Some(&format!("CALL {tool_name}")),
            "non-read MCP tools require exact local confirmation"
        );
    }
    let access_token = ensure_access_token(state.clone(), &record).await?;
    let client = McpHttpClient::new()?;
    let started = std::time::Instant::now();
    let request_hash = sha256_hex(
        canonical_json(&json!({ "tool": tool_name, "arguments": request.arguments }))?.as_bytes(),
    );
    let result = client
        .call_tool(&record, &access_token, &tool_name, &request.arguments)
        .await;
    match result {
        Ok(value) => {
            let result_hash = sha256_hex(canonical_json(&value)?.as_bytes());
            write_mcp_receipt(
                &state,
                &request.connection_id,
                &tool_name,
                &tool.operation_class,
                &request_hash,
                Some(&result_hash),
                "completed",
                "mcp_tool_completed",
                started.elapsed().as_millis() as u64,
            )?;
            mark_integration_success(&state, &request.connection_id)?;
            Ok(value)
        }
        Err(error) => {
            write_mcp_receipt(
                &state,
                &request.connection_id,
                &tool_name,
                &tool.operation_class,
                &request_hash,
                None,
                "failed",
                &public_error_code(&error),
                started.elapsed().as_millis() as u64,
            )?;
            mark_integration_error(&state, &request.connection_id, &error)?;
            Err(error)
        }
    }
}

async fn ensure_access_token(state: Arc<AppState>, record: &IntegrationRecord) -> Result<String> {
    let mut credentials = load_credentials(&record.credential_key)?;
    let expires_soon = record
        .summary
        .token_expires_at_utc
        .as_deref()
        .map(parse_time)
        .transpose()?
        .map_or(true, |expiry| {
            expiry <= Utc::now() + ChronoDuration::seconds(60)
        });
    if !expires_soon {
        return credentials
            .access_token
            .take()
            .context("MCP access token is unavailable");
    }
    let refresh = credentials
        .refresh_token
        .as_deref()
        .context("MCP authorization expired; reconnect Microgifter")?;
    let client = McpHttpClient::new()?;
    let token = client.refresh_token(record, refresh).await?;
    validate_token_response(&token)?;
    credentials.access_token = Some(token.access_token);
    if let Some(refresh_token) = token.refresh_token {
        credentials.refresh_token = Some(refresh_token);
    }
    let access = credentials
        .access_token
        .clone()
        .context("refreshed access token is missing")?;
    save_credentials(&record.credential_key, &credentials)?;
    let expires_at = timestamp(Utc::now() + ChronoDuration::seconds(token.expires_in as i64));
    state.connection()?.execute(
        "UPDATE agent_site_integrations SET state='connected',token_expires_at_utc=?1,last_success_utc=?2,last_error=NULL,updated_at_utc=?2 WHERE connection_id=?3",
        params![expires_at,now_string(),record.summary.connection_id],
    )?;
    Ok(access)
}

fn build_guidance(
    health: &Value,
    knowledge: &Value,
    models: &Value,
    operational: &operational_data::OperationalDataSnapshot,
    clouds: &cloud_registry::CloudConnectionsSnapshot,
    backups: &microgifter_homeserver_core::BackupCatalog,
    integrations: &[SiteIntegrationSummary],
) -> Vec<AgentGuidanceItem> {
    let mut items = Vec::new();
    let service_ready = health
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|state| state == "running");
    if !service_ready {
        items.push(guidance(
            "system_attention",
            "HomeServer needs attention",
            "Review service health before enabling automated work.",
            "Review system",
            "#dashboard",
            100,
        ));
    }
    let model_ready = models
        .pointer("/runtime/state")
        .and_then(Value::as_str)
        .is_some_and(|state| state == "running")
        || models
            .pointer("/openrouter_ready")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if !model_ready {
        items.push(guidance(
            "model_setup",
            "Choose an Agent model",
            "Install and start a local model, or explicitly authorize a remote provider. Without a ready model, Agent Chat can retrieve evidence but cannot reason over it.",
            "Open Model Center",
            "#models",
            95,
        ));
    }
    if clouds.active_connections == 0 {
        items.push(guidance(
            "connect_site",
            "Connect your first site",
            "Pair Microgifter or another supported wrapper so the Agent can use its authorized data and tools.",
            "Connect Microgifter",
            "agent:connections",
            90,
        ));
    } else if !integrations.iter().any(|item| item.state == "connected") {
        items.push(guidance(
            "authorize_mcp",
            "Authorize live site tools",
            "Your site is paired for synchronization. Complete MCP authorization to let the Agent perform live, scoped queries and create governed drafts or action requests.",
            "Authorize MCP",
            "agent:integrations",
            85,
        ));
    }
    let ready_documents = knowledge
        .get("ready_documents")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if ready_documents == 0 {
        items.push(guidance(
            "add_knowledge",
            "Add private knowledge",
            "Add documents to Knowledge Vault so Agent Chat can answer with local, cited context.",
            "Open Knowledge Vault",
            "#knowledge",
            80,
        ));
    }
    if operational.enabled_grants == 0 {
        items.push(guidance(
            "grant_datasets",
            "Authorize business datasets",
            "Choose which connected-site datasets may be synchronized and used by the Agent.",
            "Review data grants",
            "#integrations",
            75,
        ));
    } else if operational.imported_records == 0 {
        items.push(guidance(
            "sync_datasets",
            "Synchronize authorized data",
            "Dataset grants exist, but no provider records are available locally yet.",
            "Synchronize now",
            "agent:connections",
            70,
        ));
    }
    if backups.backups.is_empty() {
        items.push(guidance(
            "create_backup",
            "Protect your HomeServer",
            "Create the first encrypted recovery backup before relying on the Agent for daily work.",
            "Create backup",
            "#backups",
            65,
        ));
    }
    if items.is_empty() {
        items.push(guidance(
            "daily_brief",
            "Your HomeServer is ready",
            "Ask for a daily brief across system health, Knowledge Vault, connected sites, goals, schedules, approvals, and recent activity.",
            "Start daily brief",
            "agent:prompt:Give me my HomeServer daily brief.",
            10,
        ));
    }
    items.sort_by(|left, right| right.priority.cmp(&left.priority));
    items
}

fn guidance(
    key: &str,
    title: &str,
    message: &str,
    action_label: &str,
    action_target: &str,
    priority: u32,
) -> AgentGuidanceItem {
    AgentGuidanceItem {
        key: key.to_owned(),
        title: title.to_owned(),
        message: message.to_owned(),
        action_label: action_label.to_owned(),
        action_target: action_target.to_owned(),
        priority,
    }
}

fn select_automatic_tools(prompt: &str, tools: &[McpToolSummary]) -> Vec<(McpToolSummary, Value)> {
    let prompt_lower = prompt.to_ascii_lowercase();
    let prompt_terms = prompt_lower
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|term| term.len() >= 3)
        .collect::<BTreeSet<_>>();
    let mut selected = Vec::new();
    if let Some(tool) = tools.iter().find(|tool| {
        tool.name == "microgifter.account.get_connection_context" && tool.operation_class == "read"
    }) {
        selected.push((tool.clone(), json!({})));
    }
    if prompt_terms.iter().any(|term| {
        [
            "catalog", "product", "products", "item", "menu", "deal", "gift",
        ]
        .contains(term)
    }) {
        if let Some(tool) = tools.iter().find(|tool| {
            tool.name == "microgifter.catalog.search" && tool.operation_class == "read"
        }) {
            selected.push((
                tool.clone(),
                json!({ "query": truncate_chars(prompt, 100), "limit": 10 }),
            ));
        }
    }
    let mut scored = tools
        .iter()
        .filter(|tool| {
            tool.operation_class == "read"
                && !selected
                    .iter()
                    .any(|(selected, _)| selected.name == tool.name)
                && schema_has_no_required_inputs(&tool.input_schema)
        })
        .map(|tool| {
            let haystack = format!("{} {}", tool.name, tool.description).to_ascii_lowercase();
            let score = prompt_terms
                .iter()
                .filter(|term| haystack.contains(**term))
                .count();
            (score, tool)
        })
        .filter(|(score, _)| *score >= 2)
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.name.cmp(&right.1.name))
    });
    for (_, tool) in scored.into_iter().take(MAX_MCP_AUTOMATIC_CALLS) {
        selected.push((tool.clone(), json!({})));
    }
    selected.truncate(MAX_MCP_AUTOMATIC_CALLS);
    selected
}

fn schema_has_no_required_inputs(schema: &Value) -> bool {
    schema
        .get("required")
        .and_then(Value::as_array)
        .map_or(true, |values| values.is_empty())
}

fn parse_tool(value: &Value) -> Result<McpToolSummary> {
    let name = normalize_tool_name(
        value
            .get("name")
            .and_then(Value::as_str)
            .context("MCP tool name is missing")?,
    )?;
    let description = value
        .get("description")
        .and_then(Value::as_str)
        .map(|value| truncate_chars(value, 1_000))
        .unwrap_or_default();
    let input_schema = value
        .get("inputSchema")
        .or_else(|| value.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object" }));
    ensure_json_object(&input_schema, 128 * 1024, "MCP input schema")?;
    let annotations = value
        .get("annotations")
        .cloned()
        .unwrap_or_else(|| json!({}));
    ensure_json_object(&annotations, 32 * 1024, "MCP annotations")?;
    let operation_class = if annotations
        .get("readOnlyHint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "read"
    } else if name.contains("draft") || name.contains("proposal") || name.contains("playbook") {
        "draft"
    } else if name.contains("action") || name.contains("request") {
        "action_request"
    } else {
        "unknown"
    };
    Ok(McpToolSummary {
        name,
        description,
        input_schema,
        annotations,
        operation_class: operation_class.to_owned(),
    })
}

fn read_integrations(connection: &Connection) -> Result<Vec<SiteIntegrationSummary>> {
    let mut statement = connection.prepare(
        "SELECT connection_id,provider_key,resource_uri,authorization_server,client_id,redirect_uri,scopes_json,state,token_expires_at_utc,tool_catalog_json,last_tool_sync_utc,last_success_utc,last_error FROM agent_site_integrations ORDER BY provider_key,connection_id",
    )?;
    let rows = statement.query_map([], map_integration_summary)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn map_integration_summary(row: &Row<'_>) -> rusqlite::Result<SiteIntegrationSummary> {
    let scopes_json: String = row.get(6)?;
    let tools_json: String = row.get(9)?;
    Ok(SiteIntegrationSummary {
        connection_id: row.get(0)?,
        provider_key: row.get(1)?,
        resource_uri: row.get(2)?,
        authorization_server: row.get(3)?,
        client_id: row.get(4)?,
        redirect_uri: row.get(5)?,
        scopes: serde_json::from_str(&scopes_json).unwrap_or_default(),
        state: row.get(7)?,
        token_expires_at_utc: row.get(8)?,
        tools: serde_json::from_str(&tools_json).unwrap_or_default(),
        last_tool_sync_utc: row.get(10)?,
        last_success_utc: row.get(11)?,
        last_error: row.get(12)?,
    })
}

fn integration_by_id(connection: &Connection, connection_id: &str) -> Result<IntegrationRecord> {
    connection
        .query_row(
            "SELECT connection_id,provider_key,resource_uri,authorization_server,client_id,redirect_uri,scopes_json,state,token_expires_at_utc,tool_catalog_json,last_tool_sync_utc,last_success_utc,last_error,credential_key,pending_state_hash,pending_expires_at_utc FROM agent_site_integrations WHERE connection_id=?1",
            params![connection_id],
            map_integration_record,
        )
        .optional()?
        .context("site MCP integration was not configured")
}

fn integration_by_pending_state(
    connection: &Connection,
    state_hash: &str,
) -> Result<IntegrationRecord> {
    connection
        .query_row(
            "SELECT connection_id,provider_key,resource_uri,authorization_server,client_id,redirect_uri,scopes_json,state,token_expires_at_utc,tool_catalog_json,last_tool_sync_utc,last_success_utc,last_error,credential_key,pending_state_hash,pending_expires_at_utc FROM agent_site_integrations WHERE pending_state_hash=?1 AND state='authorization_pending'",
            params![state_hash],
            map_integration_record,
        )
        .optional()?
        .context("OAuth authorization request was not found")
}

fn map_integration_record(row: &Row<'_>) -> rusqlite::Result<IntegrationRecord> {
    let summary = SiteIntegrationSummary {
        connection_id: row.get(0)?,
        provider_key: row.get(1)?,
        resource_uri: row.get(2)?,
        authorization_server: row.get(3)?,
        client_id: row.get(4)?,
        redirect_uri: row.get(5)?,
        scopes: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or_default(),
        state: row.get(7)?,
        token_expires_at_utc: row.get(8)?,
        tools: serde_json::from_str(&row.get::<_, String>(9)?).unwrap_or_default(),
        last_tool_sync_utc: row.get(10)?,
        last_success_utc: row.get(11)?,
        last_error: row.get(12)?,
    };
    Ok(IntegrationRecord {
        summary,
        credential_key: row.get(13)?,
        pending_expires_at_utc: row.get(15)?,
    })
}

fn dismissed_prompt_keys(connection: &Connection) -> Result<Vec<String>> {
    let json_value: String = connection.query_row(
        "SELECT dismissed_prompt_keys_json FROM agent_engagement_state WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    Ok(serde_json::from_str(&json_value).unwrap_or_default())
}

fn dismiss_guidance(state: &AppState, key: &str) -> Result<()> {
    let key = normalize_prompt_key(key)?;
    let connection = state.connection()?;
    let mut keys = dismissed_prompt_keys(&connection)?;
    if !keys.contains(&key) {
        keys.push(key);
        keys.sort();
    }
    connection.execute(
        "UPDATE agent_engagement_state SET dismissed_prompt_keys_json=?1,engagement_revision=engagement_revision+1,updated_at_utc=?2 WHERE singleton_id=1",
        params![serde_json::to_string(&keys)?,now_string()],
    )?;
    Ok(())
}

fn write_mcp_receipt(
    state: &AppState,
    connection_id: &str,
    tool_name: &str,
    operation_class: &str,
    request_hash: &str,
    result_hash: Option<&str>,
    outcome: &str,
    result_code: &str,
    duration_ms: u64,
) -> Result<()> {
    state.connection()?.execute(
        "INSERT INTO agent_mcp_invocation_receipts (receipt_id,connection_id,tool_name,operation_class,request_hash,result_hash,outcome,result_code,duration_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            Uuid::new_v4().to_string(),
            connection_id,
            tool_name,
            operation_class,
            request_hash,
            result_hash,
            outcome,
            truncate_chars(result_code, 160),
            duration_ms.min(i64::MAX as u64) as i64,
        ],
    )?;
    Ok(())
}

fn mark_integration_success(state: &AppState, connection_id: &str) -> Result<()> {
    state.connection()?.execute(
        "UPDATE agent_site_integrations SET state='connected',last_success_utc=?1,last_error=NULL,updated_at_utc=?1 WHERE connection_id=?2",
        params![now_string(),connection_id],
    )?;
    Ok(())
}

fn mark_integration_error(
    state: &AppState,
    connection_id: &str,
    error: &anyhow::Error,
) -> Result<()> {
    state.connection()?.execute(
        "UPDATE agent_site_integrations SET state='degraded',last_error=?1,updated_at_utc=?2 WHERE connection_id=?3",
        params![truncate_chars(&error.to_string(), 500),now_string(),connection_id],
    )?;
    Ok(())
}

fn credential_entry(key: &str) -> Result<Entry> {
    Entry::new(CREDENTIAL_SERVICE, key).context("unable to open the Agent MCP credential vault")
}

fn save_credentials(key: &str, credentials: &StoredMcpCredentials) -> Result<()> {
    credential_entry(key)?
        .set_password(&serde_json::to_string(credentials)?)
        .context("unable to save Agent MCP credentials")
}

fn load_credentials(key: &str) -> Result<StoredMcpCredentials> {
    let payload = credential_entry(key)?
        .get_password()
        .context("Agent MCP credentials are unavailable")?;
    serde_json::from_str(&payload).context("Agent MCP credentials are invalid")
}

fn load_credentials_or_default(key: &str) -> Result<StoredMcpCredentials> {
    match load_credentials(key) {
        Ok(value) => Ok(value),
        Err(_) => Ok(StoredMcpCredentials {
            access_token: None,
            refresh_token: None,
            pending_verifier: None,
        }),
    }
}

async fn decode_oauth_response(response: reqwest::Response) -> Result<OAuthTokenResponse> {
    let status = response.status();
    let bytes = bounded_response_bytes(response).await?;
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("error_description")
                    .or_else(|| value.get("error"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("OAuth token endpoint returned HTTP {}", status.as_u16()));
        bail!("{}", truncate_chars(&message, 500));
    }
    serde_json::from_slice(&bytes).context("OAuth token response is invalid")
}

async fn decode_rpc_response(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let bytes = bounded_response_bytes(response).await?;
    if !status.is_success() {
        bail!("Microgifter MCP returned HTTP {}", status.as_u16());
    }
    let value = if content_type.contains("text/event-stream") {
        let text = String::from_utf8(bytes).context("MCP event stream is not UTF-8")?;
        let data = text
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .last()
            .context("MCP event stream did not contain JSON data")?;
        serde_json::from_str::<Value>(data).context("MCP event stream JSON is invalid")?
    } else {
        serde_json::from_slice::<Value>(&bytes).context("MCP JSON response is invalid")?
    };
    let envelope: RpcEnvelope = serde_json::from_value(value)?;
    if let Some(error) = envelope.error {
        let data = error
            .data
            .map(|value| truncate_chars(&value.to_string(), 300))
            .unwrap_or_default();
        bail!("MCP {}: {} {}", error.code, error.message, data);
    }
    envelope
        .result
        .context("MCP response did not contain a result")
}

async fn bounded_response_bytes(response: reqwest::Response) -> Result<Vec<u8>> {
    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_MCP_RESPONSE_BYTES as u64,
            "remote response exceeds the Agent size limit"
        );
    }
    let bytes = response
        .bytes()
        .await
        .context("unable to read remote response")?;
    ensure!(
        bytes.len() <= MAX_MCP_RESPONSE_BYTES,
        "remote response exceeds the Agent size limit"
    );
    Ok(bytes.to_vec())
}

fn validate_token_response(token: &OAuthTokenResponse) -> Result<()> {
    ensure!(
        token.token_type.eq_ignore_ascii_case("bearer"),
        "OAuth token type is not Bearer"
    );
    ensure!(
        (32..=4_096).contains(&token.access_token.len()),
        "OAuth access token length is invalid"
    );
    ensure!(
        (60..=86_400).contains(&token.expires_in),
        "OAuth access-token lifetime is invalid"
    );
    if let Some(refresh_token) = token.refresh_token.as_deref() {
        ensure!(
            (32..=4_096).contains(&refresh_token.len()),
            "OAuth refresh token length is invalid"
        );
    }
    if let Some(scope) = token.scope.as_deref() {
        ensure!(scope.len() <= 4_096, "OAuth scope response is too large");
    }
    Ok(())
}

fn normalize_resource_uri(value: &str) -> Result<String> {
    let url = Url::parse(value.trim()).context("MCP resource URI is invalid")?;
    ensure!(url.scheme() == "https", "MCP resource URI must use HTTPS");
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "MCP resource URI cannot contain credentials"
    );
    ensure!(
        url.host_str() == Some("mcp.microgifter.com") && url.path() == "/mcp",
        "MCP resource URI must be the Microgifter MCP endpoint"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "MCP resource URI cannot contain a query or fragment"
    );
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn normalize_authorization_server(value: &str) -> Result<String> {
    let url = Url::parse(value.trim()).context("authorization server is invalid")?;
    ensure!(
        url.scheme() == "https",
        "authorization server must use HTTPS"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "authorization server cannot contain credentials"
    );
    ensure!(
        url.host_str() == Some("microgifter.com") && url.path().trim_matches('/').is_empty(),
        "authorization server must be Microgifter"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "authorization server cannot contain a query or fragment"
    );
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn oauth_endpoint(server: &str, path: &str) -> Result<String> {
    let base = normalize_authorization_server(server)?;
    Ok(format!("{base}{path}"))
}

fn normalize_scopes(values: &[String]) -> Result<Vec<String>> {
    let source = if values.is_empty() {
        vec!["profile:read".to_owned(), "catalog:read".to_owned()]
    } else {
        values.to_vec()
    };
    ensure!(source.len() <= 100, "too many MCP scopes were requested");
    let mut scopes = Vec::new();
    for value in source {
        let value = value.trim().to_ascii_lowercase();
        ensure!(
            (3..=120).contains(&value.len())
                && value.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
                }),
            "MCP scope is invalid"
        );
        if !scopes.contains(&value) {
            scopes.push(value);
        }
    }
    scopes.sort();
    Ok(scopes)
}

fn normalize_tool_name(value: &str) -> Result<String> {
    let value = value.trim();
    ensure!(
        (2..=240).contains(&value.len())
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':')
            }),
        "MCP tool name is invalid"
    );
    Ok(value.to_owned())
}

fn normalize_prompt_key(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        (2..=120).contains(&value.len())
            && value.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            ),
        "Agent prompt key is invalid"
    );
    Ok(value)
}

fn validate_uuid(value: &str, label: &str) -> Result<()> {
    ensure!(Uuid::parse_str(value).is_ok(), "{label} is invalid");
    Ok(())
}

fn bounded_text(value: &str, minimum: usize, maximum: usize, label: &str) -> Result<String> {
    let value = value.trim();
    let count = value.chars().count();
    ensure!(
        (minimum..=maximum).contains(&count),
        "{label} length is invalid"
    );
    ensure!(
        !value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t')),
        "{label} contains unsupported control characters"
    );
    Ok(value.to_owned())
}

fn ensure_json_object(value: &Value, maximum: usize, label: &str) -> Result<()> {
    ensure!(value.is_object(), "{label} must be a JSON object");
    ensure!(
        serde_json::to_vec(value)?.len() <= maximum,
        "{label} exceeds the size limit"
    );
    Ok(())
}

fn random_secret(bytes: usize) -> String {
    let mut value = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

fn canonical_json(value: &Value) -> Result<String> {
    fn sort(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                let mut result = serde_json::Map::new();
                for key in keys {
                    result.insert(key.clone(), sort(&object[&key]));
                }
                Value::Object(result)
            }
            Value::Array(values) => Value::Array(values.iter().map(sort).collect()),
            _ => value.clone(),
        }
    }
    serde_json::to_string(&sort(value)).map_err(Into::into)
}

fn parse_time(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .context("stored timestamp is invalid")?
        .with_timezone(&Utc))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn now_string() -> String {
    timestamp(Utc::now())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn public_error_code(error: &anyhow::Error) -> String {
    let value = error.to_string().to_ascii_lowercase();
    if value.contains("token") || value.contains("authorization") || value.contains("oauth") {
        "mcp_authorization_failed"
    } else if value.contains("http") || value.contains("connect") || value.contains("remote") {
        "mcp_transport_failed"
    } else if value.contains("tool") {
        "mcp_tool_failed"
    } else {
        "mcp_operation_failed"
    }
    .to_owned()
}

fn callback_page(title: &str, message: &str, success: bool) -> String {
    let title = html_escape(title);
    let message = html_escape(message);
    let tone = if success { "#0f766e" } else { "#b42318" };
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title></head><body style=\"margin:0;background:#f7f7f5;font:16px system-ui;color:#171717\"><main style=\"max-width:680px;margin:12vh auto;padding:36px;background:white;border:1px solid #ddd;border-radius:18px;box-shadow:0 18px 60px rgba(0,0,0,.08)\"><div style=\"width:46px;height:46px;border-radius:14px;background:{tone};color:white;display:grid;place-items:center;font-size:24px\">✦</div><h1>{title}</h1><p style=\"line-height:1.6\">{message}</p><p style=\"color:#666\">Return to the HomeServer Agent window.</p></main></body></html>"
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("agent_integration_task_failed", error.into())
}

fn internal_error(code: &'static str, error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(?error, code, "unified Agent integration operation failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: "HomeServer could not complete the Agent integration operation.".to_owned(),
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

    #[test]
    fn automatic_tool_selection_is_read_only_and_bounded() {
        let tools = vec![
            McpToolSummary {
                name: "microgifter.account.get_connection_context".to_owned(),
                description: "Connection context".to_owned(),
                input_schema: json!({ "type": "object" }),
                annotations: json!({ "readOnlyHint": true }),
                operation_class: "read".to_owned(),
            },
            McpToolSummary {
                name: "microgifter.catalog.search".to_owned(),
                description: "Search products".to_owned(),
                input_schema: json!({ "type": "object" }),
                annotations: json!({ "readOnlyHint": true }),
                operation_class: "read".to_owned(),
            },
            McpToolSummary {
                name: "microgifter.campaign.publish_request".to_owned(),
                description: "Request campaign publishing".to_owned(),
                input_schema: json!({ "type": "object" }),
                annotations: json!({ "readOnlyHint": false }),
                operation_class: "action_request".to_owned(),
            },
        ];
        let selected = select_automatic_tools("Find products in my catalog", &tools);
        assert!(selected.len() <= MAX_MCP_AUTOMATIC_CALLS);
        assert!(selected
            .iter()
            .all(|(tool, _)| tool.operation_class == "read"));
        assert!(selected
            .iter()
            .any(|(tool, _)| tool.name == "microgifter.catalog.search"));
    }

    #[test]
    fn microgifter_endpoints_are_closed_to_expected_hosts() {
        assert_eq!(
            normalize_resource_uri("https://mcp.microgifter.com/mcp").unwrap(),
            "https://mcp.microgifter.com/mcp"
        );
        assert!(normalize_resource_uri("https://example.com/mcp").is_err());
        assert_eq!(
            normalize_authorization_server("https://microgifter.com").unwrap(),
            "https://microgifter.com"
        );
    }
}
