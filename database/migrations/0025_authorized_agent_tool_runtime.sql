-- Phase 17: authorized local agent tool runtime and multi-step orchestration.
-- Every executable step is bound to a Phase 16C wrapper job, a Phase 16D agent
-- assignment/policy, and Phase 16E result-egress enforcement. Private inputs and
-- full results remain local to HomeServer.

CREATE TABLE IF NOT EXISTS agent_tool_catalog (
  tool_key TEXT PRIMARY KEY,
  adapter_key TEXT NOT NULL UNIQUE CHECK (length(adapter_key) BETWEEN 1 AND 120),
  version TEXT NOT NULL CHECK (length(version) BETWEEN 1 AND 40),
  description TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 500),
  risk_class TEXT NOT NULL CHECK (risk_class IN (
    'read_only','reversible','external_side_effect','high_risk'
  )),
  approval_requirement TEXT NOT NULL CHECK (approval_requirement IN (
    'none','policy','proposal'
  )),
  allowed_job_types_json TEXT NOT NULL CHECK (json_valid(allowed_job_types_json)),
  input_schema_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(input_schema_json)),
  output_schema_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(output_schema_json)),
  max_execution_seconds INTEGER NOT NULL DEFAULT 60 CHECK (max_execution_seconds BETWEEN 1 AND 3600),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','paused','revoked')),
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_tool_catalog_state
  ON agent_tool_catalog (state, risk_class, tool_key);

CREATE TABLE IF NOT EXISTS agent_runtime_plans (
  plan_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  requested_by_user_id TEXT NOT NULL CHECK (length(requested_by_user_id) BETWEEN 1 AND 160),
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  objective TEXT NOT NULL CHECK (length(objective) BETWEEN 1 AND 4000),
  state TEXT NOT NULL CHECK (state IN (
    'queued','running','completed','failed','cancelled','expired'
  )),
  step_count INTEGER NOT NULL CHECK (step_count BETWEEN 1 AND 32),
  completed_step_count INTEGER NOT NULL DEFAULT 0 CHECK (completed_step_count BETWEEN 0 AND 32),
  correlation_id TEXT NOT NULL UNIQUE CHECK (length(correlation_id) BETWEEN 1 AND 160),
  plan_hash TEXT NOT NULL UNIQUE CHECK (length(plan_hash)=64),
  expires_at_utc TEXT NOT NULL,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_runtime_plans_state
  ON agent_runtime_plans (state, created_at_utc DESC, plan_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_runtime_plans_agent
  ON agent_runtime_plans (agent_id, state, updated_at_utc DESC, plan_id DESC);

CREATE TABLE IF NOT EXISTS agent_runtime_plan_steps (
  step_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  sequence_number INTEGER NOT NULL CHECK (sequence_number BETWEEN 1 AND 32),
  job_id TEXT NOT NULL UNIQUE,
  tool_key TEXT NOT NULL,
  adapter_key TEXT NOT NULL,
  action_type TEXT NOT NULL CHECK (length(action_type) BETWEEN 1 AND 120),
  state TEXT NOT NULL CHECK (state IN (
    'queued','leased','running','completed','failed','cancelled','expired'
  )),
  idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) BETWEEN 8 AND 160),
  argument_hash TEXT NOT NULL CHECK (length(argument_hash)=64),
  private_result_hash TEXT,
  safe_result_hash TEXT,
  result_code TEXT,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  started_at_utc TEXT,
  completed_at_utc TEXT,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE CASCADE,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (tool_key) REFERENCES agent_tool_catalog(tool_key) ON DELETE RESTRICT,
  UNIQUE (plan_id, sequence_number)
);

CREATE INDEX IF NOT EXISTS idx_agent_runtime_steps_dispatch
  ON agent_runtime_plan_steps (state, plan_id, sequence_number, step_id);
CREATE INDEX IF NOT EXISTS idx_agent_runtime_steps_job
  ON agent_runtime_plan_steps (job_id, state, step_id);

CREATE TABLE IF NOT EXISTS agent_runtime_attempts (
  attempt_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  step_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  worker_id TEXT NOT NULL,
  attempt_number INTEGER NOT NULL CHECK (attempt_number BETWEEN 1 AND 20),
  state TEXT NOT NULL CHECK (state IN ('running','completed','failed','cancelled')),
  result_code TEXT,
  private_result_hash TEXT,
  safe_result_hash TEXT,
  started_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE RESTRICT,
  FOREIGN KEY (step_id) REFERENCES agent_runtime_plan_steps(step_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (worker_id) REFERENCES wrapper_job_workers(worker_id) ON DELETE RESTRICT,
  UNIQUE (step_id, attempt_number)
);

CREATE INDEX IF NOT EXISTS idx_agent_runtime_attempts_step
  ON agent_runtime_attempts (step_id, attempt_number DESC, attempt_id DESC);

CREATE TABLE IF NOT EXISTS agent_runtime_receipts (
  receipt_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  step_id TEXT NOT NULL UNIQUE,
  job_id TEXT NOT NULL UNIQUE,
  agent_id TEXT NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  tool_key TEXT NOT NULL,
  adapter_key TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('completed','failed','cancelled','denied')),
  result_code TEXT NOT NULL CHECK (length(result_code) BETWEEN 1 AND 120),
  job_receipt_id TEXT,
  job_receipt_hash TEXT,
  safe_result_hash TEXT,
  runtime_receipt_hash TEXT NOT NULL UNIQUE CHECK (length(runtime_receipt_hash)=64),
  completed_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE RESTRICT,
  FOREIGN KEY (step_id) REFERENCES agent_runtime_plan_steps(step_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  FOREIGN KEY (tool_key) REFERENCES agent_tool_catalog(tool_key) ON DELETE RESTRICT,
  FOREIGN KEY (job_receipt_id) REFERENCES wrapper_job_execution_receipts(receipt_id) ON DELETE RESTRICT
);

CREATE TRIGGER IF NOT EXISTS trg_agent_runtime_receipts_no_update
BEFORE UPDATE ON agent_runtime_receipts
BEGIN
  SELECT RAISE(ABORT, 'agent runtime receipts are immutable');
END;

CREATE TABLE IF NOT EXISTS agent_runtime_events (
  event_id TEXT PRIMARY KEY,
  plan_id TEXT,
  step_id TEXT,
  job_id TEXT,
  agent_id TEXT,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 120),
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('local_user','agent','worker','system')),
  actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 160),
  detail_code TEXT NOT NULL CHECK (length(detail_code) BETWEEN 1 AND 120),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  event_hash TEXT NOT NULL UNIQUE CHECK (length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE SET NULL,
  FOREIGN KEY (step_id) REFERENCES agent_runtime_plan_steps(step_id) ON DELETE SET NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE SET NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_runtime_events_recent
  ON agent_runtime_events (created_at_utc DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_runtime_events_plan
  ON agent_runtime_events (plan_id, created_at_utc DESC, event_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_agent_runtime_events_no_update
BEFORE UPDATE ON agent_runtime_events
BEGIN
  SELECT RAISE(ABORT, 'agent runtime events are append-only');
END;

CREATE TABLE IF NOT EXISTS agent_runtime_audit_records (
  audit_record_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  step_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  input_hash TEXT NOT NULL CHECK (length(input_hash)=64),
  label TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 180),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE RESTRICT,
  FOREIGN KEY (step_id) REFERENCES agent_runtime_plan_steps(step_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT
);

CREATE TRIGGER IF NOT EXISTS trg_agent_runtime_audit_records_no_update
BEFORE UPDATE ON agent_runtime_audit_records
BEGIN
  SELECT RAISE(ABORT, 'agent runtime audit records are immutable');
END;

CREATE TABLE IF NOT EXISTS agent_runtime_state (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id=1),
  worker_id TEXT NOT NULL,
  runtime_revision INTEGER NOT NULL DEFAULT 1 CHECK (runtime_revision>=1),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','paused','stopped')),
  last_cycle_at_utc TEXT,
  last_error_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (worker_id) REFERENCES wrapper_job_workers(worker_id) ON DELETE RESTRICT
);

INSERT OR IGNORE INTO agent_tool_catalog (
  tool_key,adapter_key,version,description,risk_class,approval_requirement,
  allowed_job_types_json,input_schema_json,output_schema_json,max_execution_seconds,
  created_at_utc,updated_at_utc
) VALUES
  ('wrapper.status.read','wrapper.status.read','1.0.0',
   'Read safe wrapper connection and authority status without exposing credentials or private content.',
   'read_only','none','["runtime.wrapper_status"]','{}','{}',30,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  ('receipt.read','receipt.read','1.0.0',
   'Read bounded execution-receipt summaries for the same wrapper connection.',
   'read_only','none','["runtime.receipt_read"]','{}','{}',30,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  ('audit.record','audit.record','1.0.0',
   'Record an immutable local audit marker containing hashes and labels only.',
   'reversible','policy','["runtime.audit_record"]','{}','{}',30,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  ('result.compose','result.compose','1.0.0',
   'Compose a private local result that must pass Phase 16E egress enforcement before delivery.',
   'read_only','policy','["runtime.result_compose"]','{}','{}',60,
   strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'));

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0025_authorized_agent_tool_runtime');