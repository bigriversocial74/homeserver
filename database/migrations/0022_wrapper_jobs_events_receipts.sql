-- Phase 16C: wrapper-neutral jobs, events, safe results, deliveries, and receipts.
-- Private inputs and full execution results remain local to HomeServer. Wrapper-facing
-- records contain only authority evidence, safe projections, provenance summaries,
-- and immutable execution receipts.

CREATE TABLE IF NOT EXISTS wrapper_job_workers (
  worker_id TEXT PRIMARY KEY,
  worker_kind TEXT NOT NULL CHECK (worker_kind IN (
    'agent','model','tool','connector','media','system'
  )),
  display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 120),
  allowed_job_types_json TEXT NOT NULL CHECK (json_valid(allowed_job_types_json)),
  max_concurrent_jobs INTEGER NOT NULL CHECK (max_concurrent_jobs BETWEEN 1 AND 32),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','paused','revoked')),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  last_seen_at_utc TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wrapper_job_workers_state
  ON wrapper_job_workers (state, worker_kind, updated_at_utc DESC, worker_id);

CREATE TABLE IF NOT EXISTS wrapper_jobs (
  job_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  authorization_decision_id TEXT NOT NULL,
  capability_key TEXT NOT NULL,
  operation TEXT NOT NULL,
  job_type TEXT NOT NULL CHECK (length(job_type) BETWEEN 1 AND 80),
  state TEXT NOT NULL CHECK (state IN (
    'queued','leased','running','waiting','completed','failed','cancelled','expired','dead_letter'
  )),
  priority INTEGER NOT NULL DEFAULT 5 CHECK (priority BETWEEN 0 AND 9),
  idempotency_key TEXT NOT NULL CHECK (length(idempotency_key) BETWEEN 8 AND 160),
  request_hash TEXT NOT NULL CHECK (length(request_hash) = 64),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash) = 64),
  scope_kind TEXT,
  scope_value TEXT,
  scope_hash TEXT,
  result_policy TEXT NOT NULL CHECK (result_policy IN (
    'safe_result','metadata_only','aggregate_only','proposal_only','receipt_only'
  )),
  allowed_result_fields_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_result_fields_json)),
  max_result_bytes INTEGER NOT NULL CHECK (max_result_bytes BETWEEN 1024 AND 1048576),
  max_execution_seconds INTEGER NOT NULL CHECK (max_execution_seconds BETWEEN 1 AND 3600),
  max_attempts INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts BETWEEN 1 AND 20),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count BETWEEN 0 AND 20),
  approval_id TEXT,
  plan_hash TEXT,
  correlation_id TEXT NOT NULL CHECK (length(correlation_id) BETWEEN 1 AND 160),
  causation_id TEXT,
  submitted_by_type TEXT NOT NULL CHECK (submitted_by_type IN ('wrapper','local_user','agent','system')),
  submitted_by_id TEXT NOT NULL CHECK (length(submitted_by_id) BETWEEN 1 AND 160),
  available_at_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  lease_owner_id TEXT,
  lease_token_hash TEXT,
  lease_expires_at_utc TEXT,
  started_at_utc TEXT,
  completed_at_utc TEXT,
  cancelled_at_utc TEXT,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE RESTRICT,
  FOREIGN KEY (approval_id) REFERENCES wrapper_grant_approvals(approval_id) ON DELETE SET NULL,
  FOREIGN KEY (lease_owner_id) REFERENCES wrapper_job_workers(worker_id) ON DELETE SET NULL,
  UNIQUE (connection_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_jobs_dispatch
  ON wrapper_jobs (state, available_at_utc, priority DESC, created_at_utc, job_id);
CREATE INDEX IF NOT EXISTS idx_wrapper_jobs_connection
  ON wrapper_jobs (connection_id, created_at_utc DESC, job_id DESC);
CREATE INDEX IF NOT EXISTS idx_wrapper_jobs_authority
  ON wrapper_jobs (grant_id, grant_revision, state, expires_at_utc);
CREATE INDEX IF NOT EXISTS idx_wrapper_jobs_lease
  ON wrapper_jobs (lease_owner_id, lease_expires_at_utc, state);

CREATE TABLE IF NOT EXISTS wrapper_job_inputs (
  job_id TEXT PRIMARY KEY,
  classification TEXT NOT NULL DEFAULT 'private' CHECK (classification='private'),
  private_input_json TEXT NOT NULL CHECK (json_valid(private_input_json)),
  private_input_bytes INTEGER NOT NULL CHECK (private_input_bytes BETWEEN 2 AND 1048576),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS wrapper_job_events (
  event_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  sequence_number INTEGER NOT NULL CHECK (sequence_number >= 1),
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 100),
  previous_state TEXT,
  current_state TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  detail_code TEXT NOT NULL CHECK (length(detail_code) BETWEEN 1 AND 120),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('wrapper','local_user','worker','agent','system')),
  actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 160),
  visibility TEXT NOT NULL CHECK (visibility IN ('internal','security','wrapper')),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  event_hash TEXT NOT NULL CHECK (length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (job_id, sequence_number),
  UNIQUE (event_hash)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_job_events_connection
  ON wrapper_job_events (connection_id, created_at_utc DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_wrapper_job_events_visibility
  ON wrapper_job_events (connection_id, visibility, created_at_utc DESC, event_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_wrapper_job_events_no_update
BEFORE UPDATE ON wrapper_job_events
BEGIN
  SELECT RAISE(ABORT, 'wrapper job events are append-only');
END;

CREATE TABLE IF NOT EXISTS wrapper_job_private_results (
  job_id TEXT PRIMARY KEY,
  classification TEXT NOT NULL DEFAULT 'private' CHECK (classification='private'),
  private_result_json TEXT NOT NULL CHECK (json_valid(private_result_json)),
  private_provenance_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(private_provenance_json)),
  private_result_hash TEXT NOT NULL CHECK (length(private_result_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS wrapper_job_safe_results (
  job_id TEXT PRIMARY KEY,
  result_policy TEXT NOT NULL CHECK (result_policy IN (
    'safe_result','metadata_only','aggregate_only','proposal_only','receipt_only'
  )),
  safe_result_json TEXT NOT NULL CHECK (json_valid(safe_result_json)),
  safe_result_hash TEXT NOT NULL CHECK (length(safe_result_hash)=64),
  provenance_summary_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(provenance_summary_json)),
  provenance_summary_hash TEXT NOT NULL CHECK (length(provenance_summary_hash)=64),
  filter_version TEXT NOT NULL CHECK (length(filter_version) BETWEEN 1 AND 40),
  result_bytes INTEGER NOT NULL CHECK (result_bytes BETWEEN 2 AND 1048576),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS wrapper_job_execution_receipts (
  receipt_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL UNIQUE,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  authorization_decision_id TEXT NOT NULL,
  capability_key TEXT NOT NULL,
  operation TEXT NOT NULL,
  job_type TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  request_hash TEXT NOT NULL CHECK (length(request_hash)=64),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  approval_id TEXT,
  plan_hash TEXT,
  correlation_id TEXT NOT NULL,
  causation_id TEXT,
  outcome TEXT NOT NULL CHECK (outcome IN ('completed','failed','cancelled','expired','dead_letter')),
  result_code TEXT NOT NULL CHECK (length(result_code) BETWEEN 1 AND 120),
  safe_result_hash TEXT,
  provenance_summary_hash TEXT,
  worker_id TEXT,
  worker_kind TEXT,
  attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
  started_at_utc TEXT,
  completed_at_utc TEXT NOT NULL,
  receipt_hash TEXT NOT NULL UNIQUE CHECK (length(receipt_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE RESTRICT,
  FOREIGN KEY (worker_id) REFERENCES wrapper_job_workers(worker_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_wrapper_job_receipts_connection
  ON wrapper_job_execution_receipts (connection_id, completed_at_utc DESC, receipt_id DESC);
CREATE INDEX IF NOT EXISTS idx_wrapper_job_receipts_correlation
  ON wrapper_job_execution_receipts (correlation_id, completed_at_utc DESC, receipt_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_wrapper_job_receipts_no_update
BEFORE UPDATE ON wrapper_job_execution_receipts
BEGIN
  SELECT RAISE(ABORT, 'wrapper job execution receipts are immutable');
END;

CREATE TABLE IF NOT EXISTS wrapper_job_deliveries (
  delivery_id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL,
  receipt_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('pending','in_flight','acknowledged','expired')),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
  next_attempt_at_utc TEXT NOT NULL,
  last_attempt_at_utc TEXT,
  acknowledged_at_utc TEXT,
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY (receipt_id) REFERENCES wrapper_job_execution_receipts(receipt_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (receipt_id)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_job_deliveries_poll
  ON wrapper_job_deliveries (connection_id, state, next_attempt_at_utc, created_at_utc, delivery_id);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0022_wrapper_jobs_events_receipts');
