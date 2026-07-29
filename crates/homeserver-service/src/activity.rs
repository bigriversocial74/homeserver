use crate::AppState;
use anyhow::{Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

const MAX_CONTROL_BODY_BYTES: usize = 16 * 1024;
const MAX_ACTIVITY_ROWS: i64 = 250;
const USER_ACTIVITY_RECEIPT_INTERVAL_MINUTES: i64 = 15;

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

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/activity", get(activity_snapshot))
        .route("/v1/activity/active", post(mark_user_active))
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

async fn activity_snapshot(State(state): State<Arc<AppState>>) -> ApiResult<ActivitySnapshot> {
    tokio::task::spawn_blocking(move || snapshot(&state.connection()?))
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

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("activity_task_failed", error.into())
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

    #[test]
    fn lifecycle_events_produce_a_durable_snapshot() {
        let connection = Connection::open_in_memory().expect("open database");
        connection
            .execute_batch(include_str!("../../../database/migrations/0001_initial.sql"))
            .expect("apply initial migration");
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
}
