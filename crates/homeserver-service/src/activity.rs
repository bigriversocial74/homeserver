use crate::AppState;
use anyhow::{anyhow, ensure, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

const MAX_CONTROL_BODY_BYTES: usize = 16 * 1024;
const MAX_ACTIVITY_ROWS: i64 = 250;
const USER_ACTIVITY_RECEIPT_INTERVAL_MINUTES: i64 = 15;
const MAX_THREAD_TITLE_CHARS: usize = 160;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityEvent {
    pub event_id: i64,
    pub event_type: String,
    pub message: String,
    pub metadata: Value,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivitySnapshot {
    pub last_user_active_at_utc: Option<String>,
    pub current_session_started_at_utc: Option<String>,
    pub previous_session_started_at_utc: Option<String>,
    pub previous_session_stopped_at_utc: Option<String>,
    pub previous_session_clean: bool,
    pub recent_events: Vec<ActivityEvent>,
}

#[derive(Debug, Deserialize)]
struct RenameAgentThreadRequest {
    thread_id: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct DeleteAgentThreadRequest {
    thread_id: String,
    confirmation: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct AgentThreadMutationResult {
    thread_id: String,
    title: String,
    state: String,
    created_at_utc: String,
    updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct DeletedAgentThreadResult {
    thread_id: String,
    deleted: bool,
    deleted_messages: i64,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/activity", get(activity_snapshot))
        .route("/v1/activity/active", post(mark_user_active))
        .route("/v1/agent/threads/rename", post(rename_agent_thread))
        .route("/v1/agent/threads/delete", post(delete_agent_thread))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

pub fn initialize(connection: &Connection) -> Result<()> {
    let session_id = Uuid::new_v4().to_string();
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO homeserver_settings (setting_key,setting_value,updated_at_utc) VALUES ('service_session_id',?1,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(setting_key) DO UPDATE SET setting_value=excluded.setting_value,updated_at_utc=excluded.updated_at_utc",
        params![session_id],
    )?;
    transaction.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('service.started','HomeServer LocalSystem service started',json_object('session_id',?1,'version',?2))",
        params![session_id, env!("CARGO_PKG_VERSION")],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn record_service_stopped(state: &AppState) -> Result<()> {
    let connection = state.connection()?;
    let session_id = setting(&connection, "service_session_id")?;
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('service.stopped','HomeServer LocalSystem service stopped cleanly',json_object('session_id',?1,'reason','graceful_shutdown'))",
        params![session_id],
    )?;
    Ok(())
}

fn setting(connection: &Connection, key: &str) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT setting_value FROM homeserver_settings WHERE setting_key=?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn set_setting(connection: &Connection, key: &str, value: &str) -> Result<()> {
    connection.execute(
        "INSERT INTO homeserver_settings (setting_key,setting_value,updated_at_utc) VALUES (?1,?2,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(setting_key) DO UPDATE SET setting_value=excluded.setting_value,updated_at_utc=excluded.updated_at_utc",
        params![key, value],
    )?;
    Ok(())
}

fn parse_utc(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn event_time(
    connection: &Connection,
    event_type: &str,
    before: Option<&str>,
) -> Result<Option<String>> {
    if let Some(before) = before {
        return connection
            .query_row(
                "SELECT created_at_utc FROM service_events WHERE event_type=?1 AND created_at_utc < ?2 ORDER BY created_at_utc DESC,event_id DESC LIMIT 1",
                params![event_type, before],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into);
    }
    connection
        .query_row(
            "SELECT created_at_utc FROM service_events WHERE event_type=?1 ORDER BY created_at_utc DESC,event_id DESC LIMIT 1",
            params![event_type],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

fn recent_events(connection: &Connection) -> Result<Vec<ActivityEvent>> {
    let mut statement = connection.prepare(
        "SELECT event_id,event_type,message,metadata_json,created_at_utc FROM service_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map(params![MAX_ACTIVITY_ROWS], |row| {
        let metadata_json: Option<String> = row.get(3)?;
        Ok(ActivityEvent {
            event_id: row.get(0)?,
            event_type: row.get(1)?,
            message: row.get(2)?,
            metadata: metadata_json
                .as_deref()
                .and_then(|value| serde_json::from_str(value).ok())
                .unwrap_or_else(|| json!({})),
            created_at_utc: row.get(4)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn snapshot(connection: &Connection) -> Result<ActivitySnapshot> {
    let current_started = event_time(connection, "service.started", None)?;
    let previous_started = current_started
        .as_deref()
        .map(|value| event_time(connection, "service.started", Some(value)))
        .transpose()?
        .flatten();
    let previous_stopped = current_started
        .as_deref()
        .map(|value| event_time(connection, "service.stopped", Some(value)))
        .transpose()?
        .flatten();
    let previous_session_clean = match (&previous_started, &previous_stopped) {
        (Some(started), Some(stopped)) => stopped > started,
        _ => false,
    };

    Ok(ActivitySnapshot {
        last_user_active_at_utc: setting(connection, "control_center_last_active_at_utc")?,
        current_session_started_at_utc: current_started,
        previous_session_started_at_utc: previous_started,
        previous_session_stopped_at_utc: previous_stopped,
        previous_session_clean,
        recent_events: recent_events(connection)?,
    })
}

fn mark_active(connection: &Connection) -> Result<()> {
    let now = Utc::now();
    let previous = setting(connection, "control_center_last_active_at_utc")?;
    set_setting(
        connection,
        "control_center_last_active_at_utc",
        &now.to_rfc3339(),
    )?;

    let should_record = previous
        .as_deref()
        .and_then(parse_utc)
        .map(|value| now - value >= Duration::minutes(USER_ACTIVITY_RECEIPT_INTERVAL_MINUTES))
        .unwrap_or(true);
    if should_record {
        connection.execute(
            "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('control_center.active','Agent Workspace user activity recorded',json_object('surface','agent_workspace'))",
            [],
        )?;
    }
    Ok(())
}

fn validate_thread_id(value: &str) -> Result<&str> {
    let value = value.trim();
    Uuid::parse_str(value).map_err(|_| anyhow!("Agent chat id is invalid."))?;
    Ok(value)
}

fn normalize_thread_title(value: &str) -> Result<String> {
    let value = value.trim();
    let count = value.chars().count();
    ensure!(count > 0, "Chat name cannot be empty.");
    ensure!(
        count <= MAX_THREAD_TITLE_CHARS,
        "Chat name cannot exceed {MAX_THREAD_TITLE_CHARS} characters."
    );
    ensure!(
        !value.chars().any(char::is_control),
        "Chat name contains unsupported control characters."
    );
    Ok(value.to_owned())
}

fn thread_result(connection: &Connection, thread_id: &str) -> Result<AgentThreadMutationResult> {
    connection
        .query_row(
            "SELECT thread_id,title,state,created_at_utc,updated_at_utc FROM agent_threads WHERE thread_id=?1",
            params![thread_id],
            |row| {
                Ok(AgentThreadMutationResult {
                    thread_id: row.get(0)?,
                    title: row.get(1)?,
                    state: row.get(2)?,
                    created_at_utc: row.get(3)?,
                    updated_at_utc: row.get(4)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| anyhow!("Agent chat was not found."))
}

fn rename_thread_record(
    connection: &Connection,
    request: RenameAgentThreadRequest,
) -> Result<AgentThreadMutationResult> {
    let thread_id = validate_thread_id(&request.thread_id)?;
    let title = normalize_thread_title(&request.title)?;
    let changed = connection.execute(
        "UPDATE agent_threads SET title=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE thread_id=?1 AND state='active'",
        params![thread_id, title],
    )?;
    ensure!(changed == 1, "Agent chat was not found or is archived.");
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('agent.thread_renamed','Agent chat renamed',json_object('thread_id',?1,'surface','agent_workspace'))",
        params![thread_id],
    )?;
    thread_result(connection, thread_id)
}

fn delete_thread_record(
    connection: &Connection,
    request: DeleteAgentThreadRequest,
) -> Result<DeletedAgentThreadResult> {
    ensure!(
        request.confirmation == "DELETE",
        "Deleting an Agent chat requires the exact DELETE confirmation."
    );
    let thread_id = validate_thread_id(&request.thread_id)?.to_owned();
    let transaction = connection.unchecked_transaction()?;
    let deleted_messages: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_messages WHERE thread_id=?1",
        params![thread_id],
        |row| row.get(0),
    )?;
    let changed = transaction.execute(
        "DELETE FROM agent_threads WHERE thread_id=?1",
        params![thread_id],
    )?;
    ensure!(changed == 1, "Agent chat was not found.");
    transaction.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('agent.thread_deleted','Agent chat and messages deleted',json_object('thread_id',?1,'deleted_messages',?2,'surface','agent_workspace'))",
        params![thread_id, deleted_messages],
    )?;
    transaction.commit()?;
    Ok(DeletedAgentThreadResult {
        thread_id,
        deleted: true,
        deleted_messages,
    })
}

async fn activity_snapshot(State(state): State<Arc<AppState>>) -> ApiResult<ActivitySnapshot> {
    tokio::task::spawn_blocking(move || {
        let connection = state.connection()?;
        snapshot(&connection)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| internal_error("activity_snapshot_failed", error))
}

async fn mark_user_active(State(state): State<Arc<AppState>>) -> ApiResult<ActivitySnapshot> {
    tokio::task::spawn_blocking(move || {
        let connection = state.connection()?;
        let prior = snapshot(&connection)?;
        mark_active(&connection)?;
        Ok::<ActivitySnapshot, anyhow::Error>(prior)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| internal_error("activity_mark_failed", error))
}

async fn rename_agent_thread(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RenameAgentThreadRequest>,
) -> ApiResult<AgentThreadMutationResult> {
    tokio::task::spawn_blocking(move || {
        let connection = state.connection()?;
        rename_thread_record(&connection, request)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| action_error("agent_thread_rename_rejected", error))
}

async fn delete_agent_thread(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DeleteAgentThreadRequest>,
) -> ApiResult<DeletedAgentThreadResult> {
    tokio::task::spawn_blocking(move || {
        let connection = state.connection()?;
        delete_thread_record(&connection, request)
    })
    .await
    .map_err(task_error)?
    .map(Json)
    .map_err(|error| action_error("agent_thread_delete_rejected", error))
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("activity_task_failed", error.into())
}

fn action_error(error: &'static str, source: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error,
            message: source.to_string(),
        }),
    )
}

fn internal_error(error: &'static str, source: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error,
            message: format!("HomeServer activity history is unavailable: {source}"),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("open database");
        connection
            .execute_batch(include_str!(
                "../../../database/migrations/0001_initial.sql"
            ))
            .expect("apply initial migration");
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE agent_threads (
                   thread_id TEXT PRIMARY KEY,
                   title TEXT NOT NULL,
                   state TEXT NOT NULL DEFAULT 'active',
                   created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                   updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
                 );
                 CREATE TABLE agent_messages (
                   message_id TEXT PRIMARY KEY,
                   thread_id TEXT NOT NULL,
                   content TEXT NOT NULL,
                   FOREIGN KEY (thread_id) REFERENCES agent_threads(thread_id) ON DELETE CASCADE
                 );",
            )
            .expect("create Agent chat tables");
        connection
    }

    #[test]
    fn lifecycle_events_produce_a_durable_snapshot() {
        let connection = test_connection();
        initialize(&connection).expect("record startup");
        let first = snapshot(&connection).expect("read activity");
        assert!(first.current_session_started_at_utc.is_some());
        assert!(first
            .recent_events
            .iter()
            .any(|event| event.event_type == "service.started"));
        mark_active(&connection).expect("mark active");
        let second = snapshot(&connection).expect("read marked activity");
        assert!(second.last_user_active_at_utc.is_some());
    }

    #[test]
    fn agent_chat_can_be_renamed_and_deleted_without_retaining_messages() {
        let connection = test_connection();
        let thread_id = Uuid::new_v4().to_string();
        connection
            .execute(
                "INSERT INTO agent_threads (thread_id,title) VALUES (?1,'Original chat')",
                params![thread_id],
            )
            .expect("insert thread");
        connection
            .execute(
                "INSERT INTO agent_messages (message_id,thread_id,content) VALUES (?1,?2,'Private message')",
                params![Uuid::new_v4().to_string(), thread_id],
            )
            .expect("insert message");

        let renamed = rename_thread_record(
            &connection,
            RenameAgentThreadRequest {
                thread_id: thread_id.clone(),
                title: "Renamed chat".to_owned(),
            },
        )
        .expect("rename thread");
        assert_eq!(renamed.title, "Renamed chat");

        let deleted = delete_thread_record(
            &connection,
            DeleteAgentThreadRequest {
                thread_id: thread_id.clone(),
                confirmation: "DELETE".to_owned(),
            },
        )
        .expect("delete thread");
        assert!(deleted.deleted);
        assert_eq!(deleted.deleted_messages, 1);
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_messages WHERE thread_id=?1",
                params![thread_id],
                |row| row.get(0),
            )
            .expect("count messages");
        assert_eq!(remaining, 0);
    }
}
