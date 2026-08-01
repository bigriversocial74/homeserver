PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS audio_sessions (
    session_id TEXT PRIMARY KEY,
    thread_id TEXT,
    mode TEXT NOT NULL CHECK (mode IN ('push_to_talk','live_conversation','voice_note')),
    state TEXT NOT NULL CHECK (state IN ('armed','listening','user_speaking','finalizing_transcript','paused','muted','stopped','failed')),
    retention_mode TEXT NOT NULL CHECK (retention_mode IN ('ephemeral','transcript','audio')),
    input_device_id TEXT,
    input_device_label TEXT,
    raw_audio_retained INTEGER NOT NULL DEFAULT 0 CHECK (raw_audio_retained IN (0,1)),
    failure_code TEXT,
    started_at_utc TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    ended_at_utc TEXT
);

CREATE INDEX IF NOT EXISTS idx_audio_sessions_thread_updated
    ON audio_sessions(thread_id, updated_at_utc DESC);
CREATE INDEX IF NOT EXISTS idx_audio_sessions_state_updated
    ON audio_sessions(state, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS audio_segments (
    segment_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES audio_sessions(session_id) ON DELETE CASCADE,
    sequence_no INTEGER NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('captured','transcript_pending','final','committed','deleted','failed')),
    mime_type TEXT NOT NULL,
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    byte_length INTEGER NOT NULL CHECK (byte_length >= 0),
    content_sha256 TEXT NOT NULL,
    transcript TEXT,
    linked_message_id TEXT,
    created_at_utc TEXT NOT NULL,
    updated_at_utc TEXT NOT NULL,
    finalized_at_utc TEXT,
    UNIQUE(session_id, sequence_no)
);

CREATE INDEX IF NOT EXISTS idx_audio_segments_session_sequence
    ON audio_segments(session_id, sequence_no DESC);

CREATE TABLE IF NOT EXISTS conversation_events (
    event_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES audio_sessions(session_id) ON DELETE CASCADE,
    segment_id TEXT REFERENCES audio_segments(segment_id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_conversation_events_session_created
    ON conversation_events(session_id, created_at_utc DESC);

CREATE TABLE IF NOT EXISTS audio_permission_receipts (
    receipt_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES audio_sessions(session_id) ON DELETE CASCADE,
    microphone_authorized INTEGER NOT NULL CHECK (microphone_authorized IN (0,1)),
    recording_authorized INTEGER NOT NULL CHECK (recording_authorized IN (0,1)),
    retention_mode TEXT NOT NULL CHECK (retention_mode IN ('ephemeral','transcript','audio')),
    actor_id TEXT NOT NULL,
    created_at_utc TEXT NOT NULL
);

INSERT OR IGNORE INTO schema_migrations(migration_key)
VALUES('0030_agent_audio_conversation');
