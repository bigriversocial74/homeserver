PRAGMA foreign_keys = ON;

DROP TRIGGER IF EXISTS trg_audio_segments_phase23_update;
DROP TRIGGER IF EXISTS trg_audio_sessions_phase23_thread_binding;
DROP INDEX IF EXISTS idx_audio_segments_linked_message;

UPDATE audio_segments
SET state = 'final',
    linked_message_id = NULL,
    updated_at_utc = strftime('%Y-%m-%dT%H:%M:%fZ','now')
WHERE linked_message_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM agent_messages m
      JOIN audio_sessions s ON s.session_id = audio_segments.session_id
      JOIN agent_threads t ON t.thread_id = m.thread_id
      WHERE m.message_id = audio_segments.linked_message_id
        AND m.role = 'user'
        AND m.content = audio_segments.transcript
        AND m.created_at_utc >= audio_segments.created_at_utc
        AND (s.thread_id IS NULL OR s.thread_id = m.thread_id)
  );

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

UPDATE audio_sessions
SET thread_id = NULL,
    updated_at_utc = strftime('%Y-%m-%dT%H:%M:%fZ','now')
WHERE thread_id IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM agent_threads
      WHERE agent_threads.thread_id = audio_sessions.thread_id
  );

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

CREATE TRIGGER IF NOT EXISTS trg_audio_sessions_phase23_thread_binding
BEFORE UPDATE OF thread_id ON audio_sessions
WHEN NEW.thread_id IS NOT OLD.thread_id
 AND NOT (OLD.thread_id IS NULL AND NEW.thread_id IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'immutable Phase 23 audio thread binding');
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
        JOIN agent_threads t ON t.thread_id = m.thread_id
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
