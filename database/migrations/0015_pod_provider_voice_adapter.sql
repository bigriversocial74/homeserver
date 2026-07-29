-- POD provider voice adapter v1.
-- Additive to the provider-neutral cloud registry. This migration does not
-- modify Microgifter entitlement/update trust or the signed updater.

CREATE TABLE IF NOT EXISTS pod_provider_connections (
  connection_id TEXT PRIMARY KEY,
  provider_connection_id TEXT NOT NULL UNIQUE,
  provider_identity_id TEXT NOT NULL,
  provider_display_name TEXT NOT NULL,
  contract_version TEXT NOT NULL DEFAULT 'pod-homeserver-voice-1',
  device_signing_key_name TEXT NOT NULL UNIQUE,
  runtime_state TEXT NOT NULL DEFAULT 'unconfigured' CHECK (runtime_state IN (
    'unconfigured','ready','degraded','offline','error'
  )),
  runtime_health_message TEXT,
  last_heartbeat_at_utc TEXT,
  last_poll_at_utc TEXT,
  last_job_completed_at_utc TEXT,
  last_error_code TEXT,
  last_error_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_pod_provider_runtime_state
  ON pod_provider_connections (runtime_state, last_heartbeat_at_utc DESC);

CREATE TABLE IF NOT EXISTS pod_provider_runtime_profiles (
  connection_id TEXT PRIMARY KEY,
  transcription_enabled INTEGER NOT NULL DEFAULT 0 CHECK (transcription_enabled IN (0,1)),
  transcription_executable TEXT,
  transcription_arguments_json TEXT NOT NULL DEFAULT '[]',
  transcription_model TEXT,
  synthesis_enabled INTEGER NOT NULL DEFAULT 0 CHECK (synthesis_enabled IN (0,1)),
  synthesis_executable TEXT,
  synthesis_arguments_json TEXT NOT NULL DEFAULT '[]',
  synthesis_model TEXT,
  synthesis_voice TEXT,
  execution_timeout_seconds INTEGER NOT NULL DEFAULT 120 CHECK (
    execution_timeout_seconds BETWEEN 5 AND 1800
  ),
  maximum_input_bytes INTEGER NOT NULL DEFAULT 8388608 CHECK (
    maximum_input_bytes BETWEEN 262144 AND 16777216
  ),
  maximum_output_bytes INTEGER NOT NULL DEFAULT 8388608 CHECK (
    maximum_output_bytes BETWEEN 262144 AND 16777216
  ),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS pod_provider_voice_jobs (
  local_job_id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  remote_job_uuid TEXT NOT NULL,
  job_type TEXT NOT NULL CHECK (job_type IN (
    'speech_to_text','text_to_speech','capability_test'
  )),
  state TEXT NOT NULL CHECK (state IN (
    'leased','processing','completed','failed','retrying','cancelled'
  )),
  lease_credential_key TEXT NOT NULL UNIQUE,
  lease_token_hint TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  result_hash TEXT,
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  maximum_attempts INTEGER NOT NULL DEFAULT 1 CHECK (maximum_attempts BETWEEN 1 AND 20),
  lease_expires_at_utc TEXT NOT NULL,
  remote_expires_at_utc TEXT NOT NULL,
  model_name TEXT,
  processing_ms INTEGER,
  failure_code TEXT,
  failure_message TEXT,
  leased_at_utc TEXT NOT NULL,
  started_at_utc TEXT,
  completed_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (connection_id, remote_job_uuid)
);

CREATE INDEX IF NOT EXISTS idx_pod_voice_jobs_connection_state
  ON pod_provider_voice_jobs (connection_id, state, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS pod_provider_runtime_receipts (
  receipt_id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  local_job_id TEXT,
  event_type TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  detail_code TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (local_job_id) REFERENCES pod_provider_voice_jobs(local_job_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_pod_runtime_receipts_connection
  ON pod_provider_runtime_receipts (connection_id, created_at_utc DESC, receipt_id DESC);

CREATE TABLE IF NOT EXISTS pod_provider_worker_state (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  last_cycle_started_at_utc TEXT,
  last_cycle_completed_at_utc TEXT,
  last_connection_count INTEGER NOT NULL DEFAULT 0,
  last_job_count INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

INSERT OR IGNORE INTO pod_provider_worker_state (singleton_id) VALUES (1);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0015_pod_provider_voice_adapter');
