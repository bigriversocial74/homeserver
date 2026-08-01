use crate::AppState;
use anyhow::{ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0030_agent_audio_conversation.sql");
const HARDENING_MIGRATION: &str =
    include_str!("../../../database/migrations/0031_agent_audio_conversation_hardening.sql");
const FINAL_INTEGRITY_MIGRATION: &str =
    include_str!("../../../database/migrations/0032_agent_audio_final_integrity.sql");
const MIGRATION_KEYS: &[&str] = &[
    "0030_agent_audio_conversation",
    "0031_agent_audio_conversation_hardening",
    "0032_agent_audio_final_integrity",
];
const LOCAL_ACTOR_ID: &str = "local_control_center";
const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_TRANSCRIPT_CHARS: usize = 20_000;
const MAX_DEVICE_VALUE_CHARS: usize = 500;
const MAX_MIME_CHARS: usize = 160;
const MAX_IDENTIFIER_CHARS: usize = 160;
const MAX_EVENT_DETAIL_BYTES: usize = 16 * 1024;
const MAX_RECORDING_DURATION_MS: i64 = 30 * 60 * 1_000;
const MAX_RECORDING_BYTES: i64 = 256 * 1024 * 1024;
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
    connection.execute_batch(HARDENING_MIGRATION)?;
    connection.execute_batch(FINAL_INTEGRITY_MIGRATION)?;

    let interrupted_sessions = {
        let mut statement = connection.prepare(
            "SELECT session_id FROM audio_sessions WHERE state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted')",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    if !interrupted_sessions.is_empty() {
        let now = now_utc();
        connection.execute(
            "UPDATE audio_sessions SET state='failed',failure_code='service_restarted',updated_at_utc=?1,ended_at_utc=?1 WHERE state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted')",
            params![now],
        )?;
        for session_id in interrupted_sessions {
            insert_event(
                connection,
                &session_id,
                None,
                "session_failed",
                json!({ "failure_code": "service_restarted" }),
            )?;
        }
    }

    maintain_history(connection)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    for migration_key in MIGRATION_KEYS {
        let migration_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
            params![migration_key],
            |row| row.get(0),
        )?;
        ensure!(
            migration_count == 1,
            "Agent audio migration {migration_key} is not registered exactly once"
        );
    }

    for table in [
        "audio_sessions",
        "audio_segments",
        "conversation_events",
        "audio_permission_receipts",
    ] {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        let _: i64 = connection.query_row(&sql, [], |row| row.get(0))?;
    }

    let active_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM audio_sessions WHERE state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted')",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        active_count <= 1,
        "more than one governed audio session is active"
    );

    let unsafe_session_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM audio_sessions WHERE raw_audio_retained<>0 OR retention_mode NOT IN ('ephemeral','transcript')",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        unsafe_session_count == 0,
        "an audio session violates the Phase 23 raw-audio boundary"
    );

    let invalid_receipt_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM (SELECT s.session_id,COUNT(r.receipt_id) AS receipt_count FROM audio_sessions s LEFT JOIN audio_permission_receipts r ON r.session_id=s.session_id GROUP BY s.session_id HAVING receipt_count<>1)",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        invalid_receipt_count == 0,
        "an audio session has an invalid permission-receipt count"
    );

    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM audio_sessions WHERE retention_mode='ephemeral' AND state IN ('stopped','failed') AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-24 hours')",
        [],
    )?;
    connection.execute(
        "DELETE FROM audio_sessions WHERE retention_mode='transcript' AND state='failed' AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-7 days') AND NOT EXISTS (SELECT 1 FROM audio_segments WHERE audio_segments.session_id=audio_sessions.session_id AND transcript IS NOT NULL)",
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
    health_check(&connection)?;

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
            "verified_agent_message_linkage": true,
            "raw_audio_persistence": false,
            "raw_audio_policy": "ephemeral_in_control_center_session",
            "max_recording_duration_ms": MAX_RECORDING_DURATION_MS,
            "max_recording_bytes": MAX_RECORDING_BYTES,
            "local_stt": "planned_phase_23b_c",
            "vad": "planned_phase_23b_c",
            "cloud_egress": false
        }),
    })
}

fn save_session(
    state: &AppState,
    request: StartAudioSessionRequest,
) -> Result<AudioSessionSummary> {
    ensure!(
        SESSION_MODES.contains(&request.mode.as_str()),
        "unsupported audio mode"
    );
    ensure!(
        RETENTION_MODES.contains(&request.retention_mode.as_str()),
        "Phase 23A supports ephemeral audio or transcript retention; encrypted raw-audio retention is not enabled yet"
    );
    ensure!(
        request.microphone_authorized,
        "microphone authorization is required"
    );
    ensure!(
        request.recording_authorized,
        "recording authorization is required"
    );

    let thread_id = normalized_optional_identifier(request.thread_id, "thread ID")?;
    let input_device_id = normalized_optional_value(
        request.input_device_id,
        MAX_DEVICE_VALUE_CHARS,
        "input device ID",
    )?;
    let input_device_label = normalized_optional_value(
        request.input_device_label,
        MAX_DEVICE_VALUE_CHARS,
        "input device label",
    )?;

    let session_id = format!("aud_{}", Uuid::new_v4().simple());
    let receipt_id = format!("audperm_{}", Uuid::new_v4().simple());
    let now = now_utc();
    let mut connection = state.connection()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

    if let Some(thread_id) = thread_id.as_deref() {
        let thread_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_threads WHERE thread_id=?1)",
            params![thread_id],
            |row| row.get(0),
        )?;
        ensure!(thread_exists, "Agent Chat thread was not found");
    }

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
            thread_id,
            request.mode,
            request.retention_mode,
            input_device_id,
            input_device_label,
            now,
        ],
    )?;
    transaction.execute(
        "INSERT INTO audio_permission_receipts(receipt_id,session_id,microphone_authorized,recording_authorized,retention_mode,actor_id,created_at_utc) VALUES(?1,?2,1,1,?3,?4,?5)",
        params![
            receipt_id,
            session_id,
            request.retention_mode,
            LOCAL_ACTOR_ID,
            now
        ],
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
    validate_identifier(&request.session_id, "audio session ID")?;
    ensure!(
        SESSION_STATES.contains(&request.state.as_str()),
        "unsupported audio session state"
    );

    let failure_code = normalized_optional_value(request.failure_code, 160, "failure code")?;
    if request.state == "failed" {
        ensure!(
            failure_code.is_some(),
            "failed audio sessions require a failure code"
        );
    } else {
        ensure!(
            failure_code.is_none(),
            "failure code is only valid for failed audio sessions"
        );
    }
    validate_event_detail(&request.detail)?;
    let detail = if request.detail.is_null() {
        json!({})
    } else {
        request.detail
    };

    let now = now_utc();
    let terminal = matches!(request.state.as_str(), "stopped" | "failed");
    let mut connection = state.connection()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (current_state, current_failure_code): (String, Option<String>) = transaction
        .query_row(
            "SELECT state,failure_code FROM audio_sessions WHERE session_id=?1",
            params![request.session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .context("audio session was not found")?;

    if current_state == request.state {
        ensure!(
            current_failure_code == failure_code,
            "idempotent audio state retry does not match the stored failure code"
        );
        transaction.commit()?;
        return read_session(&connection, &request.session_id);
    }

    ensure!(
        allowed_transition(&current_state, &request.state),
        "invalid audio session state transition"
    );

    transaction.execute(
        "UPDATE audio_sessions SET state=?2,failure_code=?3,updated_at_utc=?4,ended_at_utc=CASE WHEN ?5=1 THEN ?4 ELSE NULL END WHERE session_id=?1",
        params![
            request.session_id,
            request.state,
            failure_code,
            now,
            if terminal { 1_i64 } else { 0_i64 },
        ],
    )?;
    let event_type = if request.state == "failed" {
        "session_failed".to_owned()
    } else {
        format!("state_{}", request.state)
    };
    let event_detail = if request.state == "failed" {
        merge_failure_detail(detail, failure_code.as_deref())
    } else {
        detail
    };
    insert_event(
        &transaction,
        &request.session_id,
        None,
        &event_type,
        event_detail,
    )?;
    transaction.commit()?;
    read_session(&connection, &request.session_id)
}

fn save_segment(
    state: &AppState,
    request: FinalizeAudioSegmentRequest,
) -> Result<AudioSegmentSummary> {
    validate_identifier(&request.session_id, "audio session ID")?;
    let mime_type = normalized_mime_type(&request.mime_type)?;
    ensure!(
        (1..=MAX_RECORDING_DURATION_MS).contains(&request.duration_ms),
        "recording duration is outside the local limit"
    );
    ensure!(
        (1..=MAX_RECORDING_BYTES).contains(&request.byte_length),
        "recording byte length is outside the local limit"
    );
    let content_sha256 = normalized_sha256(&request.content_sha256)?;
    let transcript = normalized_optional_text(request.transcript);
    validate_transcript(transcript.as_deref())?;

    let segment_id = format!("audseg_{}", Uuid::new_v4().simple());
    let now = now_utc();
    let mut connection = state.connection()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing_segment: Option<(String, String, Option<String>)> = transaction
        .query_row(
            "SELECT segment_id,mime_type,transcript FROM audio_segments WHERE session_id=?1 AND content_sha256=?2 AND duration_ms=?3 AND byte_length=?4 ORDER BY sequence_no DESC LIMIT 1",
            params![
                request.session_id,
                content_sha256,
                request.duration_ms,
                request.byte_length
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((existing_segment_id, existing_mime_type, existing_transcript)) = existing_segment {
        ensure!(
            existing_mime_type == mime_type,
            "idempotent recording retry does not match the stored MIME type"
        );
        if let Some(transcript) = transcript.as_deref() {
            ensure!(
                existing_transcript.as_deref() == Some(transcript),
                "idempotent recording retry does not match the stored transcript"
            );
        }
        transaction.commit()?;
        return read_segment(&connection, &existing_segment_id);
    }

    let session_state: String = transaction
        .query_row(
            "SELECT state FROM audio_sessions WHERE session_id=?1",
            params![request.session_id],
            |row| row.get(0),
        )
        .optional()?
        .context("audio session was not found")?;
    ensure!(
        session_state == "finalizing_transcript",
        "audio recordings can be finalized only from the finalizing transcript state"
    );

    let sequence_no: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence_no),0)+1 FROM audio_segments WHERE session_id=?1",
        params![request.session_id],
        |row| row.get(0),
    )?;
    let segment_state = if transcript.is_some() {
        "final"
    } else {
        "transcript_pending"
    };

    transaction.execute(
        "INSERT INTO audio_segments(segment_id,session_id,sequence_no,state,mime_type,duration_ms,byte_length,content_sha256,transcript,linked_message_id,created_at_utc,updated_at_utc,finalized_at_utc) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,?10,?10,CASE WHEN ?9 IS NULL THEN NULL ELSE ?10 END)",
        params![
            segment_id,
            request.session_id,
            sequence_no,
            segment_state,
            mime_type,
            request.duration_ms,
            request.byte_length,
            content_sha256,
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
            "mime_type": mime_type,
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
    validate_identifier(&request.segment_id, "audio segment ID")?;
    let transcript = request.transcript.trim().to_owned();
    ensure!(!transcript.is_empty(), "transcript cannot be empty");
    validate_transcript(Some(&transcript))?;
    let linked_message_id =
        normalized_optional_identifier(request.linked_message_id, "linked message ID")?;

    let now = now_utc();
    let mut connection = state.connection()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (
        session_id,
        session_thread_id,
        segment_created_at,
        segment_state,
        existing_transcript,
        existing_linked_message_id,
    ): (
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = transaction
        .query_row(
            "SELECT s.session_id,s.thread_id,g.created_at_utc,g.state,g.transcript,g.linked_message_id FROM audio_segments g JOIN audio_sessions s ON s.session_id=g.session_id WHERE g.segment_id=?1",
            params![request.segment_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?
        .context("audio segment was not found")?;

    ensure!(
        !matches!(segment_state.as_str(), "deleted" | "failed"),
        "audio segment cannot accept a transcript"
    );

    if let Some(existing_linked_message_id) = existing_linked_message_id {
        ensure!(
            linked_message_id.as_deref() == Some(existing_linked_message_id.as_str())
                && existing_transcript.as_deref() == Some(transcript.as_str()),
            "committed transcript linkage is immutable"
        );
        transaction.commit()?;
        return read_segment(&connection, &request.segment_id);
    }

    if linked_message_id.is_none()
        && segment_state == "final"
        && existing_transcript.as_deref() == Some(transcript.as_str())
    {
        transaction.commit()?;
        return read_segment(&connection, &request.segment_id);
    }

    let mut resolved_thread_id = session_thread_id;
    if let Some(linked_message_id) = linked_message_id.as_deref() {
        let message_thread_id: String = transaction
            .query_row(
                "SELECT thread_id FROM agent_messages WHERE message_id=?1 AND role='user' AND content=?2 AND created_at_utc>=?3",
                params![linked_message_id, transcript, segment_created_at],
                |row| row.get(0),
            )
            .optional()?
            .context("linked Agent message was not found or does not match the transcript")?;

        if let Some(session_thread_id) = resolved_thread_id.as_deref() {
            ensure!(
                session_thread_id == message_thread_id,
                "linked Agent message belongs to a different chat thread"
            );
        } else {
            transaction.execute(
                "UPDATE audio_sessions SET thread_id=?2 WHERE session_id=?1",
                params![session_id, message_thread_id],
            )?;
            resolved_thread_id = Some(message_thread_id);
        }
    }

    transaction.execute(
        "UPDATE audio_segments SET state=CASE WHEN ?3 IS NULL THEN 'final' ELSE 'committed' END,transcript=?2,linked_message_id=?3,updated_at_utc=?4,finalized_at_utc=COALESCE(finalized_at_utc,?4) WHERE segment_id=?1",
        params![request.segment_id, transcript, linked_message_id, now],
    )?;
    transaction.execute(
        "UPDATE audio_sessions SET updated_at_utc=?2 WHERE session_id=?1",
        params![session_id, now],
    )?;
    insert_event(
        &transaction,
        &session_id,
        Some(&request.segment_id),
        if linked_message_id.is_some() {
            "transcript_committed"
        } else {
            "transcript_updated"
        },
        json!({
            "linked_message_id": linked_message_id,
            "thread_id": resolved_thread_id
        }),
    )?;
    transaction.commit()?;
    read_segment(&connection, &request.segment_id)
}

fn remove_session(state: &AppState, request: DeleteAudioSessionRequest) -> Result<Value> {
    validate_identifier(&request.session_id, "audio session ID")?;
    ensure!(
        request.confirmation == "DELETE AUDIO SESSION",
        "exact deletion confirmation is required"
    );

    let mut connection = state.connection()?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let deleted = transaction.execute(
        "DELETE FROM audio_sessions WHERE session_id=?1 AND state IN ('stopped','failed')",
        params![request.session_id],
    )?;
    ensure!(deleted == 1, "completed audio session was not found");
    transaction.commit()?;

    Ok(json!({
        "ok": true,
        "session_id": request.session_id,
        "deleted": true
    }))
}

fn query_sessions(connection: &Connection, limit: i64) -> Result<Vec<AudioSessionSummary>> {
    let sql = format!(
        "SELECT {SESSION_COLUMNS} FROM audio_sessions ORDER BY updated_at_utc DESC,session_id DESC LIMIT ?1"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![limit], map_session)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn query_segments(connection: &Connection, limit: i64) -> Result<Vec<AudioSegmentSummary>> {
    let sql = format!(
        "SELECT {SEGMENT_COLUMNS} FROM audio_segments ORDER BY created_at_utc DESC,segment_id DESC LIMIT ?1"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![limit], map_segment)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn query_events(connection: &Connection, limit: i64) -> Result<Vec<ConversationEventSummary>> {
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM conversation_events ORDER BY created_at_utc DESC,event_id DESC LIMIT ?1"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params![limit], map_event)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
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
    validate_event_detail(&detail)?;
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

fn allowed_transition(current: &str, next: &str) -> bool {
    matches!(
        (current, next),
        ("armed", "listening")
            | ("armed", "failed")
            | ("listening", "user_speaking")
            | ("listening", "paused")
            | ("listening", "muted")
            | ("listening", "finalizing_transcript")
            | ("listening", "failed")
            | ("user_speaking", "listening")
            | ("user_speaking", "paused")
            | ("user_speaking", "muted")
            | ("user_speaking", "finalizing_transcript")
            | ("user_speaking", "failed")
            | ("paused", "listening")
            | ("paused", "muted")
            | ("paused", "failed")
            | ("muted", "listening")
            | ("muted", "paused")
            | ("muted", "failed")
            | ("finalizing_transcript", "stopped")
            | ("finalizing_transcript", "failed")
    )
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    ensure!(!value.is_empty(), "{label} cannot be empty");
    ensure!(
        value.chars().count() <= MAX_IDENTIFIER_CHARS,
        "{label} is too long"
    );
    ensure!(
        value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.' | ':')
        }),
        "{label} contains unsupported characters"
    );
    Ok(())
}

fn normalized_optional_identifier(value: Option<String>, label: &str) -> Result<Option<String>> {
    let value = normalized_optional_text(value);
    if let Some(value) = value.as_deref() {
        validate_identifier(value, label)?;
    }
    Ok(value)
}

fn normalized_optional_value(
    value: Option<String>,
    max_chars: usize,
    label: &str,
) -> Result<Option<String>> {
    let value = normalized_optional_text(value);
    if let Some(value) = value.as_deref() {
        ensure!(value.chars().count() <= max_chars, "{label} is too long");
        ensure!(!value.contains('\0'), "{label} contains a null character");
    }
    Ok(value)
}

fn normalized_mime_type(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(!value.is_empty(), "recording MIME type is empty");
    ensure!(
        value.chars().count() <= MAX_MIME_CHARS,
        "recording MIME type is too long"
    );
    ensure!(
        value.starts_with("audio/") || value == "application/octet-stream",
        "recording MIME type is not an audio type"
    );
    ensure!(
        value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '/' | ';' | '=' | '+' | '-' | '.')
        }),
        "recording MIME type contains unsupported characters"
    );
    Ok(value)
}

fn normalized_sha256(value: &str) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    ensure!(
        value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()),
        "recording SHA-256 is invalid"
    );
    Ok(value)
}

fn validate_transcript(value: Option<&str>) -> Result<()> {
    if let Some(value) = value {
        ensure!(
            value.chars().count() <= MAX_TRANSCRIPT_CHARS,
            "transcript exceeds the local size limit"
        );
        ensure!(
            !value.contains('\0'),
            "transcript contains a null character"
        );
    }
    Ok(())
}

fn validate_event_detail(value: &Value) -> Result<()> {
    ensure!(
        value.is_null() || value.is_object(),
        "audio event detail must be a JSON object"
    );
    ensure!(
        serde_json::to_vec(value)?.len() <= MAX_EVENT_DETAIL_BYTES,
        "audio event detail exceeds the local size limit"
    );
    Ok(())
}

fn normalized_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    })
}

fn merge_failure_detail(mut detail: Value, failure_code: Option<&str>) -> Value {
    if detail.is_null() {
        detail = json!({});
    }
    if let (Some(object), Some(failure_code)) = (detail.as_object_mut(), failure_code) {
        object.insert(
            "failure_code".to_owned(),
            Value::String(failure_code.to_owned()),
        );
    }
    detail
}

fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<ApiError>) {
    internal_error("audio_task_failed", error)
}

fn internal_error(
    code: &'static str,
    error: impl std::fmt::Display,
) -> (StatusCode, Json<ApiError>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("in-memory database");
        connection
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                CREATE TABLE schema_migrations(migration_key TEXT PRIMARY KEY);
                CREATE TABLE agent_threads(
                    thread_id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at_utc TEXT NOT NULL,
                    updated_at_utc TEXT NOT NULL
                );
                CREATE TABLE agent_messages(
                    message_id TEXT PRIMARY KEY,
                    thread_id TEXT NOT NULL,
                    role TEXT NOT NULL,
                    mode TEXT NOT NULL,
                    content TEXT NOT NULL,
                    context_json TEXT NOT NULL,
                    created_at_utc TEXT NOT NULL
                );
                ",
            )
            .expect("supporting schema");
        connection.execute_batch(MIGRATION).expect("base migration");
        connection
            .execute_batch(HARDENING_MIGRATION)
            .expect("hardening migration");
        connection
            .execute_batch(FINAL_INTEGRITY_MIGRATION)
            .expect("final integrity migration");
        connection
    }

    fn insert_armed_session(connection: &Connection, session_id: &str) {
        connection
            .execute(
                "INSERT INTO audio_sessions(session_id,thread_id,mode,state,retention_mode,input_device_id,input_device_label,raw_audio_retained,failure_code,started_at_utc,updated_at_utc,ended_at_utc) VALUES(?1,NULL,'push_to_talk','armed','transcript',NULL,NULL,0,NULL,'2026-08-01T00:00:00.000Z','2026-08-01T00:00:00.000Z',NULL)",
                params![session_id],
            )
            .expect("armed session");
    }

    #[test]
    fn transition_matrix_is_closed() {
        assert!(allowed_transition("armed", "listening"));
        assert!(allowed_transition("listening", "finalizing_transcript"));
        assert!(allowed_transition("finalizing_transcript", "stopped"));
        assert!(allowed_transition("muted", "failed"));
        assert!(!allowed_transition("armed", "stopped"));
        assert!(!allowed_transition("stopped", "listening"));
        assert!(!allowed_transition("finalizing_transcript", "listening"));
    }

    #[test]
    fn hardening_migration_enforces_one_active_session() {
        let connection = test_connection();
        insert_armed_session(&connection, "aud_one");
        let error = connection
            .execute(
                "INSERT INTO audio_sessions(session_id,thread_id,mode,state,retention_mode,input_device_id,input_device_label,raw_audio_retained,failure_code,started_at_utc,updated_at_utc,ended_at_utc) VALUES('aud_two',NULL,'voice_note','armed','transcript',NULL,NULL,0,NULL,'2026-08-01T00:00:01.000Z','2026-08-01T00:00:01.000Z',NULL)",
                [],
            )
            .expect_err("second active session must fail");
        assert!(error.to_string().contains("UNIQUE constraint failed"));
    }

    #[test]
    fn hardening_migration_rejects_raw_audio_retention() {
        let connection = test_connection();
        let error = connection
            .execute(
                "INSERT INTO audio_sessions(session_id,thread_id,mode,state,retention_mode,input_device_id,input_device_label,raw_audio_retained,failure_code,started_at_utc,updated_at_utc,ended_at_utc) VALUES('aud_raw',NULL,'voice_note','armed','audio',NULL,NULL,1,NULL,'2026-08-01T00:00:00.000Z','2026-08-01T00:00:00.000Z',NULL)",
                [],
            )
            .expect_err("raw audio retention must fail");
        assert!(error
            .to_string()
            .contains("invalid Phase 23 audio session boundary"));
    }

    #[test]
    fn hardening_migration_requires_finalizing_state_for_segments() {
        let connection = test_connection();
        insert_armed_session(&connection, "aud_segment");
        let error = connection
            .execute(
                "INSERT INTO audio_segments(segment_id,session_id,sequence_no,state,mime_type,duration_ms,byte_length,content_sha256,transcript,linked_message_id,created_at_utc,updated_at_utc,finalized_at_utc) VALUES('audseg_bad','aud_segment',1,'transcript_pending','audio/webm',1000,100,?1,NULL,NULL,'2026-08-01T00:00:01.000Z','2026-08-01T00:00:01.000Z',NULL)",
                params!["a".repeat(64)],
            )
            .expect_err("segment outside finalizing state must fail");
        assert!(error.to_string().contains("invalid Phase 23 audio segment"));
    }

    #[test]
    fn committed_transcript_linkage_is_verified_and_immutable() {
        let connection = test_connection();
        insert_armed_session(&connection, "aud_link");
        connection
            .execute(
                "UPDATE audio_sessions SET state='listening' WHERE session_id='aud_link'",
                [],
            )
            .expect("listening");
        connection
            .execute(
                "UPDATE audio_sessions SET state='finalizing_transcript' WHERE session_id='aud_link'",
                [],
            )
            .expect("finalizing");
        connection
            .execute(
                "INSERT INTO audio_segments(segment_id,session_id,sequence_no,state,mime_type,duration_ms,byte_length,content_sha256,transcript,linked_message_id,created_at_utc,updated_at_utc,finalized_at_utc) VALUES('audseg_link','aud_link',1,'transcript_pending','audio/webm',1000,100,?1,NULL,NULL,'2026-08-01T00:00:01.000Z','2026-08-01T00:00:01.000Z',NULL)",
                params!["b".repeat(64)],
            )
            .expect("segment");
        connection
            .execute(
                "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_missing_thread','thread_missing','user','ask','hello','{}','2026-08-01T00:00:02.000Z')",
                [],
            )
            .expect("message without retained thread");
        let missing_thread_error = connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_missing_thread',updated_at_utc='2026-08-01T00:00:02.000Z',finalized_at_utc='2026-08-01T00:00:02.000Z' WHERE segment_id='audseg_link'",
                [],
            )
            .expect_err("message thread must exist");
        assert!(missing_thread_error
            .to_string()
            .contains("invalid Phase 23 transcript linkage"));
        connection
            .execute(
                "INSERT INTO agent_threads(thread_id,title,state,created_at_utc,updated_at_utc) VALUES('thread_link','Linked audio','active','2026-08-01T00:00:00.000Z','2026-08-01T00:00:00.000Z')",
                [],
            )
            .expect("message thread");
        connection
            .execute(
                "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_link','thread_link','user','ask','hello','{}','2026-08-01T00:00:02.000Z')",
                [],
            )
            .expect("message");
        connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_link',updated_at_utc='2026-08-01T00:00:02.000Z',finalized_at_utc='2026-08-01T00:00:02.000Z' WHERE segment_id='audseg_link'",
                [],
            )
            .expect("verified link");
        let error = connection
            .execute(
                "UPDATE audio_segments SET transcript='changed' WHERE segment_id='audseg_link'",
                [],
            )
            .expect_err("committed link must be immutable");
        assert!(error
            .to_string()
            .contains("invalid Phase 23 transcript linkage"));
    }

    #[test]
    fn linked_agent_message_is_unique_and_postdates_capture() {
        let connection = test_connection();
        connection
            .execute(
                "INSERT INTO agent_threads(thread_id,title,state,created_at_utc,updated_at_utc) VALUES('thread_integrity','Integrity','active','2026-08-01T00:00:00.000Z','2026-08-01T00:00:00.000Z')",
                [],
            )
            .expect("thread");
        insert_armed_session(&connection, "aud_integrity");
        connection
            .execute(
                "UPDATE audio_sessions SET thread_id='thread_integrity',state='listening' WHERE session_id='aud_integrity'",
                [],
            )
            .expect("bound listening session");
        connection
            .execute(
                "UPDATE audio_sessions SET state='finalizing_transcript' WHERE session_id='aud_integrity'",
                [],
            )
            .expect("finalizing");
        for (segment_id, sequence_no, hash) in [
            ("audseg_integrity_one", 1_i64, "c".repeat(64)),
            ("audseg_integrity_two", 2_i64, "d".repeat(64)),
        ] {
            connection
                .execute(
                    "INSERT INTO audio_segments(segment_id,session_id,sequence_no,state,mime_type,duration_ms,byte_length,content_sha256,transcript,linked_message_id,created_at_utc,updated_at_utc,finalized_at_utc) VALUES(?1,'aud_integrity',?2,'transcript_pending','audio/webm',1000,100,?3,NULL,NULL,'2026-08-01T00:00:02.000Z','2026-08-01T00:00:02.000Z',NULL)",
                    params![segment_id, sequence_no, hash],
                )
                .expect("segment");
        }
        connection
            .execute(
                "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_too_early','thread_integrity','user','ask','hello','{}','2026-08-01T00:00:01.000Z')",
                [],
            )
            .expect("early message");
        let early_error = connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_too_early',updated_at_utc='2026-08-01T00:00:03.000Z',finalized_at_utc='2026-08-01T00:00:03.000Z' WHERE segment_id='audseg_integrity_one'",
                [],
            )
            .expect_err("message before capture must fail");
        assert!(early_error
            .to_string()
            .contains("invalid Phase 23 transcript linkage"));

        connection
            .execute(
                "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_integrity','thread_integrity','user','ask','hello','{}','2026-08-01T00:00:03.000Z')",
                [],
            )
            .expect("message");
        connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_integrity',updated_at_utc='2026-08-01T00:00:03.000Z',finalized_at_utc='2026-08-01T00:00:03.000Z' WHERE segment_id='audseg_integrity_one'",
                [],
            )
            .expect("first link");
        let duplicate_error = connection
            .execute(
                "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_integrity',updated_at_utc='2026-08-01T00:00:04.000Z',finalized_at_utc='2026-08-01T00:00:04.000Z' WHERE segment_id='audseg_integrity_two'",
                [],
            )
            .expect_err("message can link only once");
        assert!(duplicate_error
            .to_string()
            .contains("UNIQUE constraint failed"));
    }

    #[test]
    fn mime_and_hash_validation_are_closed() {
        assert_eq!(
            normalized_mime_type(" Audio/WebM;Codecs=Opus ").expect("mime"),
            "audio/webm;codecs=opus"
        );
        assert!(normalized_mime_type("text/plain").is_err());
        assert!(normalized_mime_type("audio/webm\r\nx-test").is_err());
        assert_eq!(
            normalized_sha256(&"A".repeat(64)).expect("hash"),
            "a".repeat(64)
        );
        assert!(normalized_sha256("abc").is_err());
    }
}
