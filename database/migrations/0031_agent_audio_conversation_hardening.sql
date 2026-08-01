PRAGMA foreign_keys = ON;

UPDATE audio_sessions
SET raw_audio_retained = 0,
    retention_mode = CASE WHEN retention_mode = 'audio' THEN 'transcript' ELSE retention_mode END,
    failure_code = CASE
        WHEN state = 'failed' AND (failure_code IS NULL OR trim(failure_code) = '')
            THEN 'phase23_hardening_reconciled'
        WHEN state <> 'failed' THEN NULL
        ELSE failure_code
    END,
    ended_at_utc = CASE
        WHEN state IN ('stopped','failed') THEN COALESCE(ended_at_utc, updated_at_utc)
        ELSE NULL
    END;

UPDATE audio_permission_receipts
SET retention_mode = CASE WHEN retention_mode = 'audio' THEN 'transcript' ELSE retention_mode END,
    microphone_authorized = 1,
    recording_authorized = 1;

UPDATE audio_sessions
SET state = 'failed',
    failure_code = 'phase23_hardening_duplicate_active',
    updated_at_utc = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
    ended_at_utc = strftime('%Y-%m-%dT%H:%M:%fZ','now')
WHERE session_id IN (
    SELECT session_id
    FROM audio_sessions
    WHERE state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted')
    ORDER BY updated_at_utc DESC, session_id DESC
    LIMIT -1 OFFSET 1
);

DELETE FROM audio_permission_receipts
WHERE rowid NOT IN (
    SELECT MIN(rowid)
    FROM audio_permission_receipts
    GROUP BY session_id
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_audio_sessions_single_active
ON audio_sessions(
    CASE
        WHEN state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted')
        THEN 1
    END
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_audio_permission_receipts_one_per_session
ON audio_permission_receipts(session_id);

CREATE INDEX IF NOT EXISTS idx_audio_segments_linked_message
ON audio_segments(linked_message_id)
WHERE linked_message_id IS NOT NULL;

CREATE TRIGGER IF NOT EXISTS trg_audio_sessions_phase23_insert
BEFORE INSERT ON audio_sessions
WHEN NEW.raw_audio_retained <> 0
   OR NEW.retention_mode NOT IN ('ephemeral','transcript')
   OR (NEW.state = 'failed' AND (NEW.failure_code IS NULL OR trim(NEW.failure_code) = ''))
   OR (NEW.state <> 'failed' AND NEW.failure_code IS NOT NULL)
   OR (NEW.state IN ('stopped','failed') AND NEW.ended_at_utc IS NULL)
   OR (NEW.state NOT IN ('stopped','failed') AND NEW.ended_at_utc IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 audio session boundary');
END;

CREATE TRIGGER IF NOT EXISTS trg_audio_sessions_phase23_update
BEFORE UPDATE ON audio_sessions
WHEN NEW.raw_audio_retained <> 0
   OR NEW.retention_mode NOT IN ('ephemeral','transcript')
   OR (NEW.state = 'failed' AND (NEW.failure_code IS NULL OR trim(NEW.failure_code) = ''))
   OR (NEW.state <> 'failed' AND NEW.failure_code IS NOT NULL)
   OR (NEW.state IN ('stopped','failed') AND NEW.ended_at_utc IS NULL)
   OR (NEW.state NOT IN ('stopped','failed') AND NEW.ended_at_utc IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 audio session boundary');
END;

CREATE TRIGGER IF NOT EXISTS trg_audio_sessions_phase23_transition
BEFORE UPDATE OF state ON audio_sessions
WHEN OLD.state <> NEW.state
 AND NOT (
      (OLD.state = 'armed' AND NEW.state IN ('listening','failed'))
   OR (OLD.state = 'listening' AND NEW.state IN ('user_speaking','paused','muted','finalizing_transcript','failed'))
   OR (OLD.state = 'user_speaking' AND NEW.state IN ('listening','paused','muted','finalizing_transcript','failed'))
   OR (OLD.state = 'paused' AND NEW.state IN ('listening','muted','failed'))
   OR (OLD.state = 'muted' AND NEW.state IN ('listening','paused','failed'))
   OR (OLD.state = 'finalizing_transcript' AND NEW.state IN ('stopped','failed'))
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 audio session transition');
END;

CREATE TRIGGER IF NOT EXISTS trg_audio_permission_receipts_phase23_insert
BEFORE INSERT ON audio_permission_receipts
WHEN NEW.microphone_authorized <> 1
   OR NEW.recording_authorized <> 1
   OR NEW.retention_mode NOT IN ('ephemeral','transcript')
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 audio permission receipt');
END;

CREATE TRIGGER IF NOT EXISTS trg_audio_segments_phase23_insert
BEFORE INSERT ON audio_segments
WHEN NEW.duration_ms <= 0
   OR NEW.duration_ms > 1800000
   OR NEW.byte_length <= 0
   OR NEW.byte_length > 268435456
   OR length(NEW.content_sha256) <> 64
   OR lower(NEW.content_sha256) GLOB '*[^0-9a-f]*'
   OR length(trim(NEW.mime_type)) = 0
   OR length(NEW.mime_type) > 160
   OR NOT EXISTS (
       SELECT 1
       FROM audio_sessions
       WHERE session_id = NEW.session_id
         AND state = 'finalizing_transcript'
   )
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 audio segment');
END;

CREATE TRIGGER IF NOT EXISTS trg_audio_segments_phase23_update
BEFORE UPDATE OF transcript, linked_message_id, state ON audio_segments
WHEN (OLD.linked_message_id IS NOT NULL AND (
        NEW.linked_message_id IS NOT OLD.linked_message_id
        OR NEW.transcript IS NOT OLD.transcript
        OR NEW.state <> OLD.state
     ))
   OR (NEW.linked_message_id IS NOT NULL AND NOT EXISTS (
        SELECT 1
        FROM agent_messages
        WHERE message_id = NEW.linked_message_id
          AND role = 'user'
          AND content = NEW.transcript
          AND created_at_utc >= (
              SELECT started_at_utc
              FROM audio_sessions
              WHERE session_id = NEW.session_id
          )
   ))
BEGIN
    SELECT RAISE(ABORT, 'invalid Phase 23 transcript linkage');
END;

INSERT OR IGNORE INTO schema_migrations(migration_key)
VALUES('0031_agent_audio_conversation_hardening');
