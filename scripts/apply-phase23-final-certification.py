from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one replacement in {path}, found {count}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


migration = r'''PRAGMA foreign_keys = ON;

DROP TRIGGER IF EXISTS trg_audio_segments_phase23_update;
DROP INDEX IF EXISTS idx_audio_segments_linked_message;

WITH duplicate_links AS (
    SELECT segment_id
    FROM (
        SELECT
            segment_id,
            ROW_NUMBER() OVER (
                PARTITION BY linked_message_id
                ORDER BY created_at_utc ASC, segment_id ASC
            ) AS link_rank
        FROM audio_segments
        WHERE linked_message_id IS NOT NULL
    )
    WHERE link_rank > 1
)
UPDATE audio_segments
SET state = 'final',
    linked_message_id = NULL,
    updated_at_utc = strftime('%Y-%m-%dT%H:%M:%fZ','now')
WHERE segment_id IN (SELECT segment_id FROM duplicate_links);

CREATE UNIQUE INDEX IF NOT EXISTS idx_audio_segments_unique_linked_message
ON audio_segments(linked_message_id)
WHERE linked_message_id IS NOT NULL;

CREATE TRIGGER IF NOT EXISTS trg_audio_sessions_phase23_thread_insert
BEFORE INSERT ON audio_sessions
WHEN NEW.thread_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM agent_threads WHERE thread_id = NEW.thread_id
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 Agent thread binding');
END;

CREATE TRIGGER IF NOT EXISTS trg_audio_sessions_phase23_thread_update
BEFORE UPDATE OF thread_id ON audio_sessions
WHEN NEW.thread_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM agent_threads WHERE thread_id = NEW.thread_id
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 Agent thread binding');
END;

CREATE TRIGGER IF NOT EXISTS trg_audio_segments_phase23_update
BEFORE UPDATE OF transcript, linked_message_id, state ON audio_segments
WHEN (OLD.linked_message_id IS NOT NULL AND (
        NEW.linked_message_id IS NOT OLD.linked_message_id
        OR NEW.transcript IS NOT OLD.transcript
        OR NEW.state <> OLD.state
     ))
   OR (NEW.linked_message_id IS NULL AND NEW.state = 'committed')
   OR (NEW.linked_message_id IS NOT NULL AND NEW.state <> 'committed')
   OR (NEW.state IN ('final','committed') AND (
        NEW.transcript IS NULL OR trim(NEW.transcript) = ''
   ))
   OR (NEW.linked_message_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
        FROM agent_messages m
        JOIN audio_sessions s ON s.session_id = NEW.session_id
        WHERE m.message_id = NEW.linked_message_id
          AND m.role = 'user'
          AND m.content = NEW.transcript
          AND m.created_at_utc >= NEW.created_at_utc
          AND (s.thread_id IS NULL OR s.thread_id = m.thread_id)
   ))
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 transcript linkage');
END;

INSERT OR IGNORE INTO schema_migrations(migration_key)
VALUES('0032_agent_audio_final_integrity');
'''
(ROOT / "database/migrations/0032_agent_audio_final_integrity.sql").write_text(
    migration, encoding="utf-8"
)

runtime_path = "crates/homeserver-service/src/audio_runtime.rs"
replace_once(
    runtime_path,
    '''const HARDENING_MIGRATION: &str =
    include_str!("../../../database/migrations/0031_agent_audio_conversation_hardening.sql");
const MIGRATION_KEYS: &[&str] = &[
    "0030_agent_audio_conversation",
    "0031_agent_audio_conversation_hardening",
];''',
    '''const HARDENING_MIGRATION: &str =
    include_str!("../../../database/migrations/0031_agent_audio_conversation_hardening.sql");
const FINAL_INTEGRITY_MIGRATION: &str =
    include_str!("../../../database/migrations/0032_agent_audio_final_integrity.sql");
const MIGRATION_KEYS: &[&str] = &[
    "0030_agent_audio_conversation",
    "0031_agent_audio_conversation_hardening",
    "0032_agent_audio_final_integrity",
];''',
)
replace_once(
    runtime_path,
    '''    connection.execute_batch(MIGRATION)?;
    connection.execute_batch(HARDENING_MIGRATION)?;
''',
    '''    connection.execute_batch(MIGRATION)?;
    connection.execute_batch(HARDENING_MIGRATION)?;
    connection.execute_batch(FINAL_INTEGRITY_MIGRATION)?;
''',
)
replace_once(
    runtime_path,
    '''    let session_state: String = transaction
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

    let existing_segment_id: Option<String> = transaction
        .query_row(
            "SELECT segment_id FROM audio_segments WHERE session_id=?1 AND content_sha256=?2 AND duration_ms=?3 AND byte_length=?4 ORDER BY sequence_no DESC LIMIT 1",
            params![
                request.session_id,
                content_sha256,
                request.duration_ms,
                request.byte_length
            ],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing_segment_id) = existing_segment_id {
        transaction.commit()?;
        return read_segment(&connection, &existing_segment_id);
    }
''',
    '''    let existing_segment: Option<(String, String, Option<String>)> = transaction
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
    if let Some((existing_segment_id, existing_mime_type, existing_transcript)) =
        existing_segment
    {
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
''',
)
replace_once(
    runtime_path,
    '''        session_started_at,
        segment_state,''',
    '''        segment_created_at,
        segment_state,''',
)
replace_once(
    runtime_path,
    '''            "SELECT s.session_id,s.thread_id,s.started_at_utc,g.state,g.transcript,g.linked_message_id FROM audio_segments g JOIN audio_sessions s ON s.session_id=g.session_id WHERE g.segment_id=?1",''',
    '''            "SELECT s.session_id,s.thread_id,g.created_at_utc,g.state,g.transcript,g.linked_message_id FROM audio_segments g JOIN audio_sessions s ON s.session_id=g.session_id WHERE g.segment_id=?1",''',
)
replace_once(
    runtime_path,
    '''    let mut resolved_thread_id = session_thread_id;
    if let Some(linked_message_id) = linked_message_id.as_deref() {''',
    '''    if linked_message_id.is_none()
        && segment_state == "final"
        && existing_transcript.as_deref() == Some(transcript.as_str())
    {
        transaction.commit()?;
        return read_segment(&connection, &request.segment_id);
    }

    let mut resolved_thread_id = session_thread_id;
    if let Some(linked_message_id) = linked_message_id.as_deref() {''',
)
replace_once(
    runtime_path,
    '''                "SELECT thread_id FROM agent_messages WHERE message_id=?1 AND role='user' AND content=?2 AND created_at_utc>=?3",
                params![linked_message_id, transcript, session_started_at],''',
    '''                "SELECT thread_id FROM agent_messages WHERE message_id=?1 AND role='user' AND content=?2 AND created_at_utc>=?3",
                params![linked_message_id, transcript, segment_created_at],''',
)
replace_once(
    runtime_path,
    '''        connection
            .execute_batch(HARDENING_MIGRATION)
            .expect("hardening migration");
        connection
''',
    '''        connection
            .execute_batch(HARDENING_MIGRATION)
            .expect("hardening migration");
        connection
            .execute_batch(FINAL_INTEGRITY_MIGRATION)
            .expect("final integrity migration");
        connection
''',
)
replace_once(
    runtime_path,
    '''    #[test]
    fn mime_and_hash_validation_are_closed() {''',
    '''    #[test]
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
        assert!(duplicate_error.to_string().contains("UNIQUE constraint failed"));
    }

    #[test]
    fn mime_and_hash_validation_are_closed() {''',
)

replace_once(
    "src/homeserver-agent-chat.js",
    '''    const result = await invoke("homeserver_agent_prompt", { request });
    activeThreadId = result.thread_id;
    workspace = await invoke("homeserver_agent_workspace");''',
    '''    const result = await invoke("homeserver_agent_prompt", { request });
    activeThreadId = result.thread_id;
    window.dispatchEvent(
      new CustomEvent("homeserver:agent-message-sent", {
        detail: {
          message_id: result.user_message_id,
          thread_id: result.thread_id,
          prompt,
        },
      }),
    );
    workspace = await invoke("homeserver_agent_workspace");''',
)

replace_once(
    "src/homeserver-agent-audio.js",
    '''    recorder.addEventListener("stop", () => void finalizeCapture(), { once: true });
    track.addEventListener(''',
    '''    recorder.addEventListener("stop", () => void finalizeCapture(), { once: true });
    recorder.addEventListener(
      "error",
      (event) => {
        if (!state.intentionalStop) {
          const detail = event.error?.message || "MediaRecorder reported an error.";
          void failCapture("media_recorder_error", detail);
        }
      },
      { once: true },
    );
    track.addEventListener(''',
)
replace_once(
    "src/homeserver-agent-audio.js",
    '''window.addEventListener("homeserver:rendered", scheduleDecorate);
window.addEventListener("homeserver-agent-route", scheduleDecorate);''',
    '''window.addEventListener("homeserver:agent-message-sent", (event) => {
  const pending = state.pendingLink;
  const detail = event.detail || {};
  if (
    !pending
    || detail.prompt !== pending.transcript
    || !detail.message_id
    || pending.messageIds.has(detail.message_id)
    || (pending.threadId && detail.thread_id !== pending.threadId)
  ) {
    return;
  }

  runUiAction(
    async () => {
      await audioAction("audio_update_transcript", {
        segment_id: pending.segmentId,
        transcript: pending.transcript,
        linked_message_id: detail.message_id,
      });
      if (state.pendingLink?.token === pending.token) state.pendingLink = null;
      await refreshStatus();
      notify("Transcript linked to the exact Agent Chat message.", "success");
      decorate(true);
    },
    "Unable to link the Agent Chat message",
  );
});
window.addEventListener("homeserver:rendered", scheduleDecorate);
window.addEventListener("homeserver-agent-route", scheduleDecorate);''',
)

validator = r'''from __future__ import annotations

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
'''
(ROOT / "scripts/validate-agent-audio-final-integrity.py").write_text(
    validator, encoding="utf-8"
)

replace_once(
    "package.json",
    "validate-agent-audio-conversation.py validate-agent-audio-immutability.py validate-notification-menu.py",
    "validate-agent-audio-conversation.py validate-agent-audio-immutability.py validate-agent-audio-final-integrity.py validate-notification-menu.py",
)

print("Phase 23A final certification repair applied.")
