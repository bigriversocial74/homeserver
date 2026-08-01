from __future__ import annotations

import re
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(text: str, token: str, label: str) -> None:
    if token not in text:
        raise SystemExit(f"Missing {label}: {token}")


def require_pattern(text: str, pattern: str, label: str) -> None:
    if re.search(pattern, text, flags=re.MULTILINE | re.DOTALL) is None:
        raise SystemExit(f"Missing {label}: /{pattern}/")


def forbid(text: str, token: str, label: str) -> None:
    if token in text:
        raise SystemExit(f"Forbidden {label}: {token}")


def expect_sql_failure(connection: sqlite3.Connection, sql: str, label: str) -> None:
    try:
        connection.execute(sql)
    except sqlite3.DatabaseError:
        return
    raise SystemExit(f"Database contract did not reject {label}")


def validate_database_contract(base_migration: str, hardening_migration: str) -> None:
    connection = sqlite3.connect(":memory:")
    connection.executescript(
        """
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
        """
    )
    connection.executescript(base_migration)
    connection.executescript(hardening_migration)

    connection.execute(
        """
        INSERT INTO audio_sessions(
            session_id,thread_id,mode,state,retention_mode,input_device_id,
            input_device_label,raw_audio_retained,failure_code,started_at_utc,
            updated_at_utc,ended_at_utc
        ) VALUES(
            'aud_contract',NULL,'push_to_talk','armed','transcript',NULL,
            NULL,0,NULL,'2026-08-01T00:00:00.000Z',
            '2026-08-01T00:00:00.000Z',NULL
        )
        """
    )
    expect_sql_failure(
        connection,
        """
        INSERT INTO audio_sessions(
            session_id,thread_id,mode,state,retention_mode,input_device_id,
            input_device_label,raw_audio_retained,failure_code,started_at_utc,
            updated_at_utc,ended_at_utc
        ) VALUES(
            'aud_contract_two',NULL,'voice_note','armed','transcript',NULL,
            NULL,0,NULL,'2026-08-01T00:00:01.000Z',
            '2026-08-01T00:00:01.000Z',NULL
        )
        """,
        "a second active session",
    )
    expect_sql_failure(
        connection,
        """
        INSERT INTO audio_sessions(
            session_id,thread_id,mode,state,retention_mode,input_device_id,
            input_device_label,raw_audio_retained,failure_code,started_at_utc,
            updated_at_utc,ended_at_utc
        ) VALUES(
            'aud_raw',NULL,'voice_note','failed','audio',NULL,
            NULL,1,'raw_audio_forbidden','2026-08-01T00:00:01.000Z',
            '2026-08-01T00:00:01.000Z','2026-08-01T00:00:01.000Z'
        )
        """,
        "raw-audio retention",
    )
    expect_sql_failure(
        connection,
        """
        UPDATE audio_sessions
        SET state='stopped',ended_at_utc='2026-08-01T00:00:02.000Z'
        WHERE session_id='aud_contract'
        """,
        "an invalid state transition",
    )

    connection.execute(
        "UPDATE audio_sessions SET state='listening' WHERE session_id='aud_contract'"
    )
    connection.execute(
        """
        UPDATE audio_sessions
        SET state='finalizing_transcript'
        WHERE session_id='aud_contract'
        """
    )
    connection.execute(
        """
        INSERT INTO audio_segments(
            segment_id,session_id,sequence_no,state,mime_type,duration_ms,
            byte_length,content_sha256,transcript,linked_message_id,
            created_at_utc,updated_at_utc,finalized_at_utc
        ) VALUES(
            'audseg_contract','aud_contract',1,'transcript_pending',
            'audio/webm',1000,100,
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            NULL,NULL,'2026-08-01T00:00:03.000Z',
            '2026-08-01T00:00:03.000Z',NULL
        )
        """
    )
    expect_sql_failure(
        connection,
        """
        UPDATE audio_segments
        SET state='committed',transcript='hello',
            linked_message_id='missing_message',
            finalized_at_utc='2026-08-01T00:00:04.000Z'
        WHERE segment_id='audseg_contract'
        """,
        "an unverified Agent-message link",
    )

    connection.execute(
        """
        INSERT INTO agent_messages(
            message_id,thread_id,role,mode,content,context_json,created_at_utc
        ) VALUES(
            'msg_contract','thread_contract','user','ask','hello','{}',
            '2026-08-01T00:00:04.000Z'
        )
        """
    )
    connection.execute(
        """
        UPDATE audio_segments
        SET state='committed',transcript='hello',
            linked_message_id='msg_contract',
            finalized_at_utc='2026-08-01T00:00:04.000Z'
        WHERE segment_id='audseg_contract'
        """
    )
    expect_sql_failure(
        connection,
        """
        UPDATE audio_segments
        SET transcript='rewritten'
        WHERE segment_id='audseg_contract'
        """,
        "a committed transcript mutation",
    )


base_migration = read("database/migrations/0030_agent_audio_conversation.sql")
for table in (
    "audio_sessions",
    "audio_segments",
    "conversation_events",
    "audio_permission_receipts",
):
    require(
        base_migration,
        f"CREATE TABLE IF NOT EXISTS {table}",
        f"{table} schema",
    )
require(
    base_migration,
    "0030_agent_audio_conversation",
    "base migration registration",
)

hardening_migration = read(
    "database/migrations/0031_agent_audio_conversation_hardening.sql"
)
for token, label in (
    (
        "idx_audio_sessions_single_active",
        "database-enforced single active session",
    ),
    (
        "trg_audio_sessions_phase23_transition",
        "closed session transition trigger",
    ),
    (
        "trg_audio_sessions_phase23_insert",
        "raw-audio session boundary trigger",
    ),
    (
        "trg_audio_permission_receipts_phase23_update",
        "immutable permission boundary",
    ),
    (
        "trg_audio_segments_phase23_insert",
        "segment finalization boundary",
    ),
    (
        "trg_audio_segments_phase23_update",
        "verified immutable transcript linkage",
    ),
    (
        "0031_agent_audio_conversation_hardening",
        "hardening migration registration",
    ),
):
    require(hardening_migration, token, label)
require(
    hardening_migration,
    "NEW.raw_audio_retained <> 0",
    "database raw-audio retention denial",
)
require(
    hardening_migration,
    "m.created_at_utc >= s.started_at_utc",
    "message temporal-link boundary",
)
require(
    hardening_migration,
    "(s.thread_id IS NULL OR s.thread_id = m.thread_id)",
    "message thread-link boundary",
)
validate_database_contract(base_migration, hardening_migration)

runtime = read("crates/homeserver-service/src/audio_runtime.rs")
for route in (
    "/v1/audio/status",
    "/v1/audio/sessions/start",
    "/v1/audio/sessions/state",
    "/v1/audio/sessions/delete",
    "/v1/audio/segments",
    "/v1/audio/segments/transcript",
):
    require(runtime, route, f"protected audio route {route}")
for token, label in (
    (
        "0031_agent_audio_conversation_hardening.sql",
        "hardening migration execution",
    ),
    (
        "TransactionBehavior::Immediate",
        "serialized audio write transactions",
    ),
    (
        "fn allowed_transition",
        "closed service transition matrix",
    ),
    (
        "verified_agent_message_linkage",
        "verified Agent-message linkage capability",
    ),
    (
        "linked Agent message was not found or does not match the transcript",
        "service message-link verification",
    ),
    (
        "committed transcript linkage is immutable",
        "service immutable transcript linkage",
    ),
    (
        "MAX_RECORDING_DURATION_MS",
        "recording duration limit",
    ),
    (
        "MAX_RECORDING_BYTES",
        "recording byte limit",
    ),
    (
        "raw_audio_persistence\": false",
        "ephemeral raw-audio capability boundary",
    ),
    (
        "\"cloud_egress\": false",
        "local-only egress boundary",
    ),
    (
        "#[cfg(test)]",
        "native audio contract tests",
    ),
    (
        "hardening_migration_enforces_one_active_session",
        "single-session native test",
    ),
    (
        "committed_transcript_linkage_is_verified_and_immutable",
        "linkage native test",
    ),
):
    require(runtime, token, label)
forbid(
    runtime,
    'const RETENTION_MODES: &[&str] = &["ephemeral", "transcript", "audio"]',
    "runtime raw-audio retention mode",
)

app = read("crates/homeserver-service/src/app.rs")
for token in (
    '#[path = "audio_runtime.rs"]',
    "audio_runtime::initialize(&connection)?;",
    ".merge(audio_runtime::router(state.clone()))",
    "audio_runtime::maintain_history(&connection)",
    "let protected_router = http::secure(",
):
    require(app, token, "protected audio service registration")

bridge = read("src-tauri/src/agent.rs")
require(bridge, '"audio".to_owned()', "audio status in Agent workspace")
for action, route in (
    ("audio_status", "/v1/audio/status"),
    ("audio_start_session", "/v1/audio/sessions/start"),
    ("audio_set_state", "/v1/audio/sessions/state"),
    ("audio_finalize_segment", "/v1/audio/segments"),
    ("audio_update_transcript", "/v1/audio/segments/transcript"),
    ("audio_delete_session", "/v1/audio/sessions/delete"),
):
    require(bridge, f'Some("{action}")', f"trusted Tauri action {action}")
    require(bridge, route, f"trusted Tauri route {route}")

index = read("index.html")
require(index, "/src/homeserver-agent-audio.js", "Agent audio module loading")

chat = read("src/homeserver-agent-audio.js")
for token, label in (
    (
        "navigator.mediaDevices.getUserMedia",
        "local microphone capture",
    ),
    (
        "new MediaRecorder",
        "local MediaRecorder capture",
    ),
    (
        'retention_mode: "transcript"',
        "persistent transcript metadata with ephemeral raw audio",
    ),
    (
        "CAPTURE_STOP_MS",
        "bounded recording duration",
    ),
    (
        "CAPTURE_STOP_BYTES",
        "bounded in-memory recording size",
    ),
    (
        "reconcileOrphanedSession",
        "stale webview-session recovery",
    ),
    (
        "releaseLocalRecording",
        "object URL cleanup",
    ),
    (
        "MAX_LOCAL_RECORDINGS",
        "bounded local playback memory",
    ),
    (
        "workspaceMessageSnapshot",
        "pre-submit message identity baseline",
    ),
    (
        "!pending.messageIds.has(message.message_id)",
        "new-message-only transcript linkage",
    ),
    (
        "Agent Chat already contains an unsent draft",
        "composer draft protection",
    ),
    (
        "pagehide",
        "capture-host shutdown handling",
    ),
    (
        "devicechange",
        "microphone device lifecycle handling",
    ),
    (
        'raw_audio_retained: false',
        "raw-audio persistence denial",
    ),
):
    require(chat, token, label)

for token, label in (
    ("SpeechRecognition", "browser/cloud speech recognition"),
    ("webkitSpeechRecognition", "browser/cloud speech recognition"),
    ("audio_base64", "raw audio JSON upload"),
    ("FileReader", "raw audio serialization"),
    ("fetch(", "direct network egress"),
    ("XMLHttpRequest", "direct network egress"),
    ("WebSocket", "direct network egress"),
    ("RTCPeerConnection", "peer audio egress"),
    ("sendBeacon", "beacon audio egress"),
):
    forbid(chat, token, label)

for javascript_path in (ROOT / "src").rglob("*.js"):
    javascript = javascript_path.read_text(encoding="utf-8")
    forbid(
        javascript,
        "webkitSpeechRecognition",
        f"browser/cloud speech recognition in {javascript_path.relative_to(ROOT)}",
    )
    require_pattern(
        javascript,
        r"\A[\s\S]*\Z",
        f"readable JavaScript file {javascript_path.relative_to(ROOT)}",
    )

css = read("src/homeserver-agent-audio.css")
require(
    css,
    "Phase 23 Agent Chat ears and conversation engine",
    "audio UI styles",
)
require(css, ".hs-agent-audio-panel", "audio panel styles")
require(css, ".hs-agent-audio-mic", "microphone control styles")
require(css, ":focus-visible", "keyboard focus styles")
require(css, "prefers-reduced-motion", "reduced-motion styles")

package = read("package.json")
require(
    package,
    "node --check src/homeserver-agent-audio.js",
    "Agent audio JavaScript syntax gate",
)
require(
    package,
    "validate-agent-audio-conversation.py",
    "Agent audio permanent validator gate",
)

for temporary_path in (
    ROOT / "src-tauri/src/audio.rs",
    ROOT / "scripts/apply-phase23-agent-audio.py",
    ROOT / ".github/workflows/phase-23-bootstrap.yml",
):
    if temporary_path.exists():
        raise SystemExit(
            f"Temporary Phase 23 staging file remains: "
            f"{temporary_path.relative_to(ROOT)}"
        )

print(
    "Phase 23A validates protected local microphone capture, database-enforced "
    "single-session and state-transition rules, bounded ephemeral raw audio, "
    "verified immutable Agent-message linkage, and permanent native/SQLite tests."
)
