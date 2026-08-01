from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = (ROOT / "database/migrations/0030_agent_audio_conversation.sql").read_text(encoding="utf-8")
HARDENING = (ROOT / "database/migrations/0031_agent_audio_conversation_hardening.sql").read_text(encoding="utf-8")
FINAL = (ROOT / "database/migrations/0032_agent_audio_final_integrity.sql").read_text(encoding="utf-8")
RUNTIME = (ROOT / "crates/homeserver-service/src/audio_runtime.rs").read_text(encoding="utf-8")
CHAT = (ROOT / "src/homeserver-agent-chat.js").read_text(encoding="utf-8")
AUDIO = (ROOT / "src/homeserver-agent-audio.js").read_text(encoding="utf-8")


def require(text: str, token: str, label: str) -> None:
    if token not in text:
        raise SystemExit(f"Missing {label}: {token}")


def reject(connection: sqlite3.Connection, sql: str, label: str) -> None:
    try:
        connection.execute(sql)
    except sqlite3.DatabaseError:
        return
    raise SystemExit(f"Final Phase 23 integrity contract accepted {label}")


for token, label in (
    ("idx_audio_segments_unique_linked_message", "one-message/one-segment index"),
    ("m.created_at_utc >= NEW.created_at_utc", "post-capture message boundary"),
    ("trg_audio_sessions_phase23_thread_insert", "thread insert verification"),
    ("trg_audio_sessions_phase23_thread_update", "thread update verification"),
    ("0032_agent_audio_final_integrity", "migration registration"),
):
    require(FINAL, token, label)

require(RUNTIME, "0032_agent_audio_final_integrity.sql", "final migration execution")
require(RUNTIME, "idempotent recording retry does not match the stored MIME type", "finalize retry integrity")
require(RUNTIME, "segment_created_at", "post-capture service linkage")
require(RUNTIME, 'segment_state == "final"', "transcript retry idempotency")
require(CHAT, 'new CustomEvent("homeserver:agent-message-sent"', "exact Agent message event")
require(CHAT, "result.user_message_id", "exact Agent user message identity")
require(AUDIO, 'window.addEventListener("homeserver:agent-message-sent"', "exact message listener")
require(AUDIO, 'recorder.addEventListener(\n      "error"', "MediaRecorder error handling")
require(AUDIO, "pending.messageIds.has(detail.message_id)", "pre-submit message exclusion")

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
connection.executescript(FINAL)

reject(
    connection,
    """
    INSERT INTO audio_sessions(
        session_id,thread_id,mode,state,retention_mode,input_device_id,
        input_device_label,raw_audio_retained,failure_code,started_at_utc,
        updated_at_utc,ended_at_utc
    ) VALUES(
        'aud_bad_thread','missing_thread','push_to_talk','armed','transcript',
        NULL,NULL,0,NULL,'2026-08-01T00:00:00.000Z',
        '2026-08-01T00:00:00.000Z',NULL
    )
    """,
    "a missing Agent thread",
)
connection.execute(
    "INSERT INTO agent_threads(thread_id,title,state,created_at_utc,updated_at_utc) VALUES('thread_final','Final','active','2026-08-01T00:00:00.000Z','2026-08-01T00:00:00.000Z')"
)
connection.execute(
    """
    INSERT INTO audio_sessions(
        session_id,thread_id,mode,state,retention_mode,input_device_id,
        input_device_label,raw_audio_retained,failure_code,started_at_utc,
        updated_at_utc,ended_at_utc
    ) VALUES(
        'aud_final','thread_final','push_to_talk','armed','transcript',
        NULL,NULL,0,NULL,'2026-08-01T00:00:00.000Z',
        '2026-08-01T00:00:00.000Z',NULL
    )
    """
)
connection.execute("UPDATE audio_sessions SET state='listening' WHERE session_id='aud_final'")
connection.execute("UPDATE audio_sessions SET state='finalizing_transcript' WHERE session_id='aud_final'")
for segment_id, sequence_no, digest in (
    ("audseg_final_one", 1, "e" * 64),
    ("audseg_final_two", 2, "f" * 64),
):
    connection.execute(
        """
        INSERT INTO audio_segments(
            segment_id,session_id,sequence_no,state,mime_type,duration_ms,
            byte_length,content_sha256,transcript,linked_message_id,
            created_at_utc,updated_at_utc,finalized_at_utc
        ) VALUES(?, 'aud_final', ?, 'transcript_pending','audio/webm',1000,100,?,
            NULL,NULL,'2026-08-01T00:00:02.000Z','2026-08-01T00:00:02.000Z',NULL)
        """,
        (segment_id, sequence_no, digest),
    )
connection.execute(
    "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_early','thread_final','user','ask','hello','{}','2026-08-01T00:00:01.000Z')"
)
reject(
    connection,
    "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_early',updated_at_utc='2026-08-01T00:00:03.000Z',finalized_at_utc='2026-08-01T00:00:03.000Z' WHERE segment_id='audseg_final_one'",
    "a message created before capture",
)
connection.execute(
    "INSERT INTO agent_messages(message_id,thread_id,role,mode,content,context_json,created_at_utc) VALUES('msg_final','thread_final','user','ask','hello','{}','2026-08-01T00:00:03.000Z')"
)
connection.execute(
    "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_final',updated_at_utc='2026-08-01T00:00:03.000Z',finalized_at_utc='2026-08-01T00:00:03.000Z' WHERE segment_id='audseg_final_one'"
)
reject(
    connection,
    "UPDATE audio_segments SET state='committed',transcript='hello',linked_message_id='msg_final',updated_at_utc='2026-08-01T00:00:04.000Z',finalized_at_utc='2026-08-01T00:00:04.000Z' WHERE segment_id='audseg_final_two'",
    "one Agent message linked to two segments",
)
connection.close()
print("Phase 23A final integrity validates exact message identity, post-capture linkage, unique evidence binding, retry-safe finalization, and recorder failure handling.")
