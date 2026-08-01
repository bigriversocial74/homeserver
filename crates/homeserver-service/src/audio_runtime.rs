use crate::AppState;
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0030_agent_audio_conversation.sql");
const MIGRATION_KEY: &str = "0030_agent_audio_conversation";
const LOCAL_ACTOR_ID: &str = "local_control_center";
const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_TRANSCRIPT_CHARS: usize = 20_000;
const MAX_DEVICE_VALUE_CHARS: usize = 500;
const MAX_MIME_CHARS: usize = 160;
const MAX_RECORDING_BYTES: i64 = 512 * 1024 * 1024;
const ACTIVE_STATES: &[&str] = &[
    "armed",
    "listening",
    "user_speaking",
    "finalizing_transcript",
    "paused",
    "muted",
];
const SESSION_MODES: &[&str] = &["push_to_talk", "live_conversation", "voice_note"];
const SESSION_STATES: &[&str] = &[
    "armed",
    "listening",
    "user_speaking",
    "finalizing_transcript",
    "paused",
    "muted",
    "stopped",
    "failed",
];
const RETENTION_MODES: &[&str] = &["ephemeral", "transcript"];

const SESSION_COLUMNS: &str = "session_id,thread_id,mode,state,retention_mode,input_device_id,input_device_label,raw_audio_retained,failure_code,started_at_utc,updated_at_utc,ended_at_utc";
const SEGMENT_COLUMNS: &str = "segment_id,session_id,sequence_no,state,mime_type,duration_ms,byte_length,content_sha256,transcript,linked_message_id,created_at_utc,updated_at_utc,finalized_at_utc";
const EVENT_COLUMNS: &str = "event_id,session_id,segment_id,event_type,detail_json,created_at_utc";

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

#[derive(Debug, Serialize)]
struct ApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioSessionSummary {
    pub session_id: String,
    pub thread_id: Option<String>,
    pub mode: String,
    pub state: String,
    pub retention_mode: String,
    pub input_device_id: Option<String>,
    pub input_device_label: Option<String>,
    pub raw_audio_retained: bool,
    pub failure_code: Option<String>,
    pub started_at_utc: String,
    pub updated_at_utc: String,
    pub ended_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioSegmentSummary {
    pub segment_id: String,
    pub session_id: String,
    pub sequence_no: i64,
    pub state: String,
    pub mime_type: String,
    pub duration_ms: i64,
    pub byte_length: i64,
    pub content_sha256: String,
    pub transcript: Option<String>,
    pub linked_message_id: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub finalized_at_utc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationEventSummary {
    pub event_id: String,
    pub session_id: String,
    pub segment_id: Option<String>,
    pub event_type: String,
    pub detail: Value,
    pub created_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioStatusSnapshot {
    pub schema: String,
    pub host_state: String,
    pub active_session: Option<AudioSessionSummary>,
    pub sessions: Vec<AudioSessionSummary>,
    pub segments: Vec<AudioSegmentSummary>,
    pub events: Vec<ConversationEventSummary>,
    pub capabilities: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartAudioSessionRequest {
    pub thread_id: Option<String>,
    pub mode: String,
    pub retention_mode: String,
    pub input_device_id: Option<String>,
    pub input_device_label: Option<String>,
    pub microphone_authorized: bool,
    pub recording_authorized: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioSessionStateRequest {
    pub session_id: String,
    pub state: String,
    pub failure_code: Option<String>,
    #[serde(default)]
    pub detail: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FinalizeAudioSegmentRequest {
    pub session_id: String,
    pub mime_type: String,
    pub duration_ms: i64,
    pub byte_length: i64,
    pub content_sha256: String,
    pub transcript: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAudioTranscriptRequest {
    pub segment_id: String,
    pub transcript: String,
    pub linked_message_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteAudioSessionRequest {
    pub session_id: String,
    pub confirmation: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    connection.execute(
        "UPDATE audio_sessions SET state='failed',failure_code='service_restarted',updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),ended_at_utc=COALESCE(ended_at_utc,strftime('%Y-%m-%dT%H:%M:%fZ','now')) WHERE state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted')",
        [],
    )?;
    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "Agent audio conversation migration is not registered exactly once"
    );
    for table in [
        "audio_sessions",
        "audio_segments",
        "conversation_events",
        "audio_permission_receipts",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM audio_sessions WHERE retention_mode='ephemeral' AND state IN ('stopped','failed') AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-24 hours')",
        [],
    )?;
    connection.execute(
        "DELETE FROM conversation_events WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-90 days')",
        [],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/audio/status", get(status))
        .route("/v1/audio/sessions/start", post(start_session))
        .route("/v1/audio/sessions/state", post(set_session_state))
        .route("/v1/audio/sessions/delete", post(delete_session))
        .route("/v1/audio/segments", post(finalize_segment))
        .route("/v1/audio/segments/transcript", post(update_transcript))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn status(State(state): State<Arc<AppState>>) -> ApiResult<AudioStatusSnapshot> {
    tokio::task::spawn_blocking(move || read_status(&state))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| internal_error("audio_status_failed", error))
}

async fn start_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<StartAudioSessionRequest>,
) -> ApiResult<AudioSessionSummary> {
    tokio::task::spawn_blocking(move || save_session(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("audio_session_rejected", error))
}

async fn set_session_state(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AudioSessionStateRequest>,
) -> ApiResult<AudioSessionSummary> {
    tokio::task::spawn_blocking(move || update_session_state(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("audio_state_rejected", error))
}

async fn finalize_segment(
    State(state): State<Arc<AppState>>,
    Json(request): Json<FinalizeAudioSegmentRequest>,
) -> ApiResult<AudioSegmentSummary> {
    tokio::task::spawn_blocking(move || save_segment(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("audio_segment_rejected", error))
}

async fn update_transcript(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdateAudioTranscriptRequest>,
) -> ApiResult<AudioSegmentSummary> {
    tokio::task::spawn_blocking(move || save_transcript(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("audio_transcript_rejected", error))
}

async fn delete_session(
    State(state): State<Arc<AppState>>,
    Json(request): Json<DeleteAudioSessionRequest>,
) -> ApiResult<Value> {
    tokio::task::spawn_blocking(move || remove_session(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| action_error("audio_session_delete_rejected", error))
}

fn read_status(state: &AppState) -> Result<AudioStatusSnapshot> {
    let connection = state.connection()?;
    maintain_history(&connection)?;
    let sessions = query_sessions(&connection, 24)?;
    let active_session = sessions
        .iter()
        .find(|session| ACTIVE_STATES.contains(&session.state.as_str()))
        .cloned();
    let segments = query_segments(&connection, 60)?;
    let events = query_events(&connection, 80)?;
    Ok(AudioStatusSnapshot {
        schema: "homeserver.agent-audio.v1".to_owned(),
        host_state: if active_session.is_some() {
            "session_active".to_owned()
        } else {
            "ready".to_owned()
        },
        active_session,
        sessions,
        segments,
        events,
        capabilities: json!({
            "capture_host": "control_center_webview",
            "microphone_capture": true,
            "media_recorder": true,
            "conversation_events": true,
            "transcript_metadata": true,
            "raw_audio_persistence": false,
            "raw_audio_policy": "ephemeral_in_control_center_session",
            "local_stt": "planned_phase_23d",
            "vad": "planned_phase_23c",
            "cloud_egress": false
        }),
    })
}

fn save_session(state: &AppState, request: StartAudioSessionRequest) -> Result<AudioSessionSummary> {
    ensure!(SESSION_MODES.contains(&request.mode.as_str()), "unsupported audio mode");
    ensure!(
        RETENTION_MODES.contains(&request.retention_mode.as_str()),
        "Phase 23A supports ephemeral audio or transcript retention; encrypted raw-audio retention is not enabled yet"
    );
    ensure!(request.microphone_authorized, "microphone authorization is required");
    ensure!(request.recording_authorized, "recording authorization is required");
    validate_optional_value(request.thread_id.as_deref(), 160, "thread ID")?;
    validate_optional_value(
        request.input_device_id.as_deref(),
        MAX_DEVICE_VALUE_CHARS,
        "input device ID",
    )?;
    validate_optional_value(
        request.input_device_label.as_deref(),
        MAX_DEVICE_VALUE_CHARS,
        "input device label",
    )?;

    let session_id = format!("aud_{}", Uuid::new_v4().simple());
    let receipt_id = format!("audperm_{}", Uuid::new_v4().simple());
    let now = now_utc();
    let mut connection = state.connection()?;
    let transaction = connection.transaction()?;
    let active_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM audio_sessions WHERE state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted')",
        [],
        |row| row.get(0),
    )?;
    ensure!(active_count == 0, "another audio session is already active");
    transaction.execute(
        "INSERT INTO audio_sessions(session_id,thread_id,mode,state,retention_mode,input_device_id,input_device_label,raw_audio_retained,failure_code,started_at_utc,updated_at_utc,ended_at_utc) VALUES(?1,?2,?3,'armed',?4,?5,?6,0,NULL,?7,?7,NULL)",
        params![
            session_id,
            request.thread_id,
            request.mode,
            request.retention_mode,
            request.input_device_id,
            request.input_device_label,
            now,
        ],
    )?;
    transaction.execute(
        "INSERT INTO audio_permission_receipts(receipt_id,session_id,microphone_authorized,recording_authorized,retention_mode,actor_id,created_at_utc) VALUES(?1,?2,1,1,?3,?4,?5)",
        params![receipt_id, session_id, request.retention_mode, LOCAL_ACTOR_ID, now],
    )?;
    insert_event(
        &transaction,
        &session_id,
        None,
        "session_started",
        json!({
            "mode": request.mode,
            "retention_mode": request.retention_mode,
            "capture_host": "control_center_webview"
        }),
    )?;
    transaction.commit()?;
    read_session(&connection, &session_id)
}

fn update_session_state(
    state: &AppState,
    request: AudioSessionStateRequest,
) -> Result<AudioSessionSummary> {
    ensure!(
        SESSION_STATES.contains(&request.state.as_str()),
        "unsupported audio session state"
    );
    ensure!(request.session_id.len() <= 160, "audio session ID is too long");
    validate_optional_value(request.failure_code.as_deref(), 160, "failure code")?;
    if request.state == "failed" {
        ensure!(
            request.failure_code.as_deref().is_some_and(|value| !value.trim().is_empty()),
            "failed audio sessions require a failure code"
        );
    }
    let now = now_utc();
    let terminal = matches!(request.state.as_str(), "stopped" | "failed");
    let mut connection = state.connection()?;
    let transaction = connection.transaction()?;
    let current_state: String = transaction
        .query_row(
            "SELECT state FROM audio_sessions WHERE session_id=?1",
            params![request.session_id],
            |row| row.get(0),
        )
        .optional()?
        .context("audio session was not found")?;
    ensure!(
        !matches!(current_state.as_str(), "stopped" | "failed"),
        "audio session is already complete"
    );
    transaction.execute(
        "UPDATE audio_sessions SET state=?2,failure_code=?3,updated_at_utc=?4,ended_at_utc=CASE WHEN ?5=1 THEN ?4 ELSE ended_at_utc END WHERE session_id=?1",
        params![
            request.session_id,
            request.state,
            request.failure_code,
            now,
            i64::from(terminal),
        ],
    )?;
    insert_event(
        &transaction,
        &request.session_id,
        None,
        &format!("state_{}", request.state),
        request.detail,
    )?;
    transaction.commit()?;
    read_session(&connection, &request.session_id)
}

fn save_segment(
    state: &AppState,
    request: FinalizeAudioSegmentRequest,
) -> Result<AudioSegmentSummary> {
    ensure!(request.session_id.len() <= 160, "audio session ID is too long");
    ensure!(
        !request.mime_type.trim().is_empty() && request.mime_type.chars().count() <= MAX_MIME_CHARS,
        "recording MIME type is invalid"
    );
    ensure!(request.duration_ms >= 0, "recording duration cannot be negative");
    ensure!(
        (0..=MAX_RECORDING_BYTES).contains(&request.byte_length),
        "recording byte length is invalid"
    );
    ensure!(
        request.content_sha256.len() == 64
            && request.content_sha256.chars().all(|character| character.is_ascii_hexdigit()),
        "recording SHA-256 is invalid"
    );
    validate_transcript(request.transcript.as_deref())?;

    let segment_id = format!("audseg_{}", Uuid::new_v4().simple());
    let now = now_utc();
    let mut connection = state.connection()?;
    let transaction = connection.transaction()?;
    let session_state: String = transaction
        .query_row(
            "SELECT state FROM audio_sessions WHERE session_id=?1",
            params![request.session_id],
            |row| row.get(0),
        )
        .optional()?
        .context("audio session was not found")?;
    ensure!(session_state != "failed", "failed audio sessions cannot accept recordings");
    let sequence_no: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence_no),0)+1 FROM audio_segments WHERE session_id=?1",
        params![request.session_id],
        |row| row.get(0),
    )?;
    let transcript = normalized_optional_text(request.transcript);
    let segment_state = if transcript.is_some() {
        "final"
    } else {
        "transcript_pending"
    };
    transaction.execute(
        "INSERT INTO audio_segments(segment_id,session_id,sequence_no,state,mime_type,duration_ms,byte_length,content_sha256,transcript,linked_message_id,created_at_utc,updated_at_utc,finalized_at_utc) VALUES(?1,?2,?3,?4,?5,?6,?7,lower(?8),?9,NULL,?10,?10,CASE WHEN ?9 IS NULL THEN NULL ELSE ?10 END)",
        params![
            segment_id,
            request.session_id,
            sequence_no,
            segment_state,
            request.mime_type,
            request.duration_ms,
            request.byte_length,
            request.content_sha256,
            transcript,
            now,
        ],
    )?;
    transaction.execute(
        "UPDATE audio_sessions SET updated_at_utc=?2 WHERE session_id=?1",
        params![request.session_id, now],
    )?;
    insert_event(
        &transaction,
        &request.session_id,
        Some(&segment_id),
        "recording_captured",
        json!({
            "duration_ms": request.duration_ms,
            "byte_length": request.byte_length,
            "mime_type": request.mime_type,
            "raw_audio_retained": false
        }),
    )?;
    transaction.commit()?;
    read_segment(&connection, &segment_id)
}

fn save_transcript(
    state: &AppState,
    request: UpdateAudioTranscriptRequest,
) -> Result<AudioSegmentSummary> {
    let transcript = request.transcript.trim();
    ensure!(!transcript.is_empty(), "transcript cannot be empty");
    validate_transcript(Some(transcript))?;
    validate_optional_value(request.linked_message_id.as_deref(), 160, "linked message ID")?;
    let now = now_utc();
    let mut connection = state.connection()?;
    let transaction = connection.transaction()?;
    let session_id: String = transaction
        .query_row(
            "SELECT session_id FROM audio_segments WHERE segment_id=?1",
            params![request.segment_id],
            |row| row.get(0),
        )
        .optional()?
        .context("audio segment was not found")?;
    transaction.execute(
        "UPDATE audio_segments SET state=CASE WHEN ?3 IS NULL THEN 'final' ELSE 'committed' END,transcript=?2,linked_message_id=?3,updated_at_utc=?4,finalized_at_utc=COALESCE(finalized_at_utc,?4) WHERE segment_id=?1",
        params![request.segment_id, transcript, request.linked_message_id, now],
    )?;
    transaction.execute(
        "UPDATE audio_sessions SET updated_at_utc=?2 WHERE session_id=?1",
        params![session_id, now],
    )?;
    insert_event(
        &transaction,
        &session_id,
        Some(&request.segment_id),
        if request.linked_message_id.is_some() {
            "transcript_committed"
        } else {
            "transcript_updated"
        },
        json!({ "linked_message_id": request.linked_message_id }),
    )?;
    transaction.commit()?;
    read_segment(&connection, &request.segment_id)
}

fn remove_session(state: &AppState, request: DeleteAudioSessionRequest) -> Result<Value> {
    ensure!(
        request.confirmation == "DELETE AUDIO SESSION",
        "exact deletion confirmation is required"
    );
    let connection = state.connection()?;
    let deleted = connection.execute(
        "DELETE FROM audio_sessions WHERE session_id=?1 AND state IN ('stopped','failed')",
        params![request.session_id],
    )?;
    ensure!(deleted == 1, "completed audio session was not found");
    Ok(json!({ "ok": true, "session_id": request.session_id, "deleted": true }))
}

fn query_sessions(connection: &Connection, limit: i64) -> Result<Vec<AudioSessionSummary>> {
    let sql = format!(
        "SELECT {SESSION_COLUMNS} FROM audio_sessions ORDER BY updated_at_utc DESC,session_id DESC LIMIT ?1"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![limit], map_session)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn query_segments(connection: &Connection, limit: i64) -> Result<Vec<AudioSegmentSummary>> {
    let sql = format!(
        "SELECT {SEGMENT_COLUMNS} FROM audio_segments ORDER BY created_at_utc DESC,segment_id DESC LIMIT ?1"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![limit], map_segment)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn query_events(connection: &Connection, limit: i64) -> Result<Vec<ConversationEventSummary>> {
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM conversation_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![limit], map_event)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

fn read_session(connection: &Connection, session_id: &str) -> Result<AudioSessionSummary> {
    let sql = format!("SELECT {SESSION_COLUMNS} FROM audio_sessions WHERE session_id=?1");
    connection
        .query_row(&sql, params![session_id], map_session)
        .optional()?
        .context("audio session was not found")
}

fn read_segment(connection: &Connection, segment_id: &str) -> Result<AudioSegmentSummary> {
    let sql = format!("SELECT {SEGMENT_COLUMNS} FROM audio_segments WHERE segment_id=?1");
    connection
        .query_row(&sql, params![segment_id], map_segment)
        .optional()?
        .context("audio segment was not found")
}

fn map_session(row: &Row<'_>) -> rusqlite::Result<AudioSessionSummary> {
    Ok(AudioSessionSummary {
        session_id: row.get(0)?,
        thread_id: row.get(1)?,
        mode: row.get(2)?,
        state: row.get(3)?,
        retention_mode: row.get(4)?,
        input_device_id: row.get(5)?,
        input_device_label: row.get(6)?,
        raw_audio_retained: row.get::<_, i64>(7)? == 1,
        failure_code: row.get(8)?,
        started_at_utc: row.get(9)?,
        updated_at_utc: row.get(10)?,
        ended_at_utc: row.get(11)?,
    })
}

fn map_segment(row: &Row<'_>) -> rusqlite::Result<AudioSegmentSummary> {
    Ok(AudioSegmentSummary {
        segment_id: row.get(0)?,
        session_id: row.get(1)?,
        sequence_no: row.get(2)?,
        state: row.get(3)?,
        mime_type: row.get(4)?,
        duration_ms: row.get(5)?,
        byte_length: row.get(6)?,
        content_sha256: row.get(7)?,
        transcript: row.get(8)?,
        linked_message_id: row.get(9)?,
        created_at_utc: row.get(10)?,
        updated_at_utc: row.get(11)?,
        finalized_at_utc: row.get(12)?,
    })
}

fn map_event(row: &Row<'_>) -> rusqlite::Result<ConversationEventSummary> {
    let detail_json: String = row.get(4)?;
    Ok(ConversationEventSummary {
        event_id: row.get(0)?,
        session_id: row.get(1)?,
        segment_id: row.get(2)?,
        event_type: row.get(3)?,
        detail: serde_json::from_str(&detail_json).unwrap_or_else(|_| json!({})),
        created_at_utc: row.get(5)?,
    })
}

fn insert_event(
    connection: &Connection,
    session_id: &str,
    segment_id: Option<&str>,
    event_type: &str,
    detail: Value,
) -> Result<()> {
    connection.execute(
        "INSERT INTO conversation_events(event_id,session_id,segment_id,event_type,detail_json,created_at_utc) VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            format!("audevt_{}", Uuid::new_v4().simple()),
            session_id,
            segment_id,
            event_type,
            serde_json::to_string(&detail)?,
            now_utc(),
        ],
    )?;
    Ok(())
}

fn validate_optional_value(value: Option<&str>, max_chars: usize, label: &str) -> Result<()> {
    if let Some(value) = value {
        ensure!(value.chars().count() <= max_chars, "{label} is too long");
    }
    Ok(())
}

fn validate_transcript(value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        ensure!(
            value.chars().count() <= MAX_TRANSCRIPT_CHARS,
            "transcript exceeds the local size limit"
        );
    }
    Ok(())
}

fn normalized_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("audio_task_failed", error)
}

fn internal_error(code: &'static str, error: impl std::fmt::Display) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}

fn action_error(code: &'static str, error: impl std::fmt::Display) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}
