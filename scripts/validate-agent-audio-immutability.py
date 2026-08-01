from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = (ROOT / "database/migrations/0030_agent_audio_conversation.sql").read_text(
    encoding="utf-8"
)
HARDENING = (
    ROOT / "database/migrations/0031_agent_audio_conversation_hardening.sql"
).read_text(encoding="utf-8")


def require(token: str, label: str) -> None:
    if token not in HARDENING:
        raise SystemExit(f"Missing {label}: {token}")


def reject(connection: sqlite3.Connection, sql: str, label: str) -> None:
    try:
        connection.execute(sql)
    except sqlite3.DatabaseError:
        return
    raise SystemExit(f"Phase 23 evidence mutation was accepted: {label}")


for token, label in (
    ("trg_audio_sessions_phase23_immutable", "immutable session evidence trigger"),
    ("trg_audio_sessions_phase23_thread_binding", "one-way thread binding trigger"),
    (
        "trg_audio_permission_receipts_phase23_immutable",
        "immutable permission receipt trigger",
    ),
    (
        "trg_audio_segments_phase23_capture_immutable",
        "immutable captured segment evidence trigger",
    ),
    (
        "trg_conversation_events_phase23_immutable",
        "append-only conversation event trigger",
    ),
):
    require(token, label)

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
connection.executescript(BASE)
connection.executescript(HARDENING)

connection.execute(
    """
    INSERT INTO audio_sessions(
        session_id,thread_id,mode,state,retention_mode,input_device_id,
        input_device_label,raw_audio_retained,failure_code,started_at_utc,
        updated_at_utc,ended_at_utc
    ) VALUES(
        'aud_immutable',NULL,'push_to_talk','armed','transcript','device-1',
        'Local microphone',0,NULL,'2026-08-01T00:00:00.000Z',
        '2026-08-01T00:00:00.000Z',NULL
    )
    """
)
connection.execute(
    """
    INSERT INTO audio_permission_receipts(
        receipt_id,session_id,microphone_authorized,recording_authorized,
        retention_mode,actor_id,created_at_utc
    ) VALUES(
        'audperm_immutable','aud_immutable',1,1,'transcript',
        'local_control_center','2026-08-01T00:00:00.000Z'
    )
    """
)

reject(
    connection,
    "UPDATE audio_sessions SET mode='voice_note' WHERE session_id='aud_immutable'",
    "session mode",
)
reject(
    connection,
    "UPDATE audio_sessions SET input_device_label='Changed' WHERE session_id='aud_immutable'",
    "input-device evidence",
)
connection.execute(
    "UPDATE audio_sessions SET thread_id='thread_one' WHERE session_id='aud_immutable'"
)
reject(
    connection,
    "UPDATE audio_sessions SET thread_id='thread_two' WHERE session_id='aud_immutable'",
    "bound Agent thread",
)
reject(
    connection,
    "UPDATE audio_permission_receipts SET actor_id='other_actor' WHERE receipt_id='audperm_immutable'",
    "permission receipt actor",
)

connection.execute(
    "UPDATE audio_sessions SET state='listening' WHERE session_id='aud_immutable'"
)
connection.execute(
    "UPDATE audio_sessions SET state='finalizing_transcript' WHERE session_id='aud_immutable'"
)
connection.execute(
    """
    INSERT INTO audio_segments(
        segment_id,session_id,sequence_no,state,mime_type,duration_ms,
        byte_length,content_sha256,transcript,linked_message_id,
        created_at_utc,updated_at_utc,finalized_at_utc
    ) VALUES(
        'audseg_immutable','aud_immutable',1,'transcript_pending','audio/webm',
        1000,100,
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        NULL,NULL,'2026-08-01T00:00:01.000Z',
        '2026-08-01T00:00:01.000Z',NULL
    )
    """
)
reject(
    connection,
    "UPDATE audio_segments SET byte_length=101 WHERE segment_id='audseg_immutable'",
    "captured segment byte length",
)
reject(
    connection,
    "UPDATE audio_segments SET content_sha256='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE segment_id='audseg_immutable'",
    "captured segment hash",
)

connection.execute(
    """
    INSERT INTO conversation_events(
        event_id,session_id,segment_id,event_type,detail_json,created_at_utc
    ) VALUES(
        'audevt_immutable','aud_immutable','audseg_immutable',
        'recording_captured','{}','2026-08-01T00:00:01.000Z'
    )
    """
)
reject(
    connection,
    "UPDATE conversation_events SET event_type='rewritten' WHERE event_id='audevt_immutable'",
    "conversation event",
)

connection.close()
print(
    "Phase 23 audio session identity, permission receipts, captured segment "
    "evidence, thread binding, and conversation events are immutable."
)
