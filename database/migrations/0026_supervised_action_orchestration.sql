-- Phase 18: supervised action orchestration and approval-resume checkpoints.
-- Sensitive runtime steps remain bound to the certified Phase 16D proposal,
-- approval, execution, and receipt lifecycle. Phase 17 remains the ordered plan
-- owner and Phase 16E remains mandatory for the proposal job safe projection.

INSERT OR IGNORE INTO agent_tool_catalog (
  tool_key,adapter_key,version,description,risk_class,approval_requirement,
  allowed_job_types_json,input_schema_json,output_schema_json,max_execution_seconds,
  created_at_utc,updated_at_utc
) VALUES (
  'action.supervised','action.supervised','1.0.0',
  'Pause an ordered runtime plan at a Phase 16D action proposal, resume only after exact approval revalidation, and retain one immutable cross-phase evidence chain.',
  'external_side_effect','proposal','["action.propose"]','{}','{}',300,
  strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')
);

CREATE TABLE IF NOT EXISTS agent_supervised_action_checkpoints (
  checkpoint_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  step_id TEXT NOT NULL UNIQUE,
  sequence_number INTEGER NOT NULL CHECK (sequence_number BETWEEN 1 AND 32),
  job_id TEXT NOT NULL UNIQUE,
  agent_id TEXT NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  proposal_id TEXT NOT NULL UNIQUE,
  approval_id TEXT UNIQUE,
  policy_id TEXT NOT NULL,
  action_type TEXT NOT NULL CHECK (length(action_type) BETWEEN 1 AND 120),
  risk_class TEXT NOT NULL CHECK (risk_class IN (
    'read_only','reversible','external_side_effect','high_risk'
  )),
  tool_adapter TEXT NOT NULL CHECK (length(tool_adapter) BETWEEN 1 AND 120),
  state TEXT NOT NULL CHECK (state IN (
    'awaiting_approval','approved','executing','completed','failed',
    'rejected','cancelled','expired'
  )),
  compensation_mode TEXT NOT NULL DEFAULT 'manual' CHECK (compensation_mode IN (
    'manual','automatic','disabled'
  )),
  compensation_supported INTEGER NOT NULL DEFAULT 0 CHECK (compensation_supported IN (0,1)),
  compensation_state TEXT NOT NULL DEFAULT 'not_supported' CHECK (compensation_state IN (
    'not_supported','available','running','completed','failed','disabled'
  )),
  runtime_plan_hash TEXT NOT NULL CHECK (length(runtime_plan_hash)=64),
  proposal_plan_hash TEXT NOT NULL CHECK (length(proposal_plan_hash)=64),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  agent_revision INTEGER NOT NULL CHECK (agent_revision>=1),
  assignment_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL CHECK (assignment_revision>=1),
  policy_revision INTEGER NOT NULL CHECK (policy_revision>=1),
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision>=1),
  connection_authority_revision INTEGER NOT NULL CHECK (connection_authority_revision>=0),
  authorization_decision_id TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE RESTRICT,
  FOREIGN KEY (step_id) REFERENCES agent_runtime_plan_steps(step_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE RESTRICT,
  FOREIGN KEY (approval_id) REFERENCES agent_action_approvals(approval_id) ON DELETE RESTRICT,
  FOREIGN KEY (policy_id) REFERENCES agent_execution_policies(policy_id) ON DELETE RESTRICT,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_supervised_checkpoints_dispatch
  ON agent_supervised_action_checkpoints (state, created_at_utc, checkpoint_id);
CREATE INDEX IF NOT EXISTS idx_supervised_checkpoints_plan
  ON agent_supervised_action_checkpoints (plan_id, sequence_number, checkpoint_id);
CREATE INDEX IF NOT EXISTS idx_supervised_checkpoints_approval
  ON agent_supervised_action_checkpoints (approval_id, state, checkpoint_id);

CREATE TABLE IF NOT EXISTS agent_supervised_action_receipts (
  receipt_id TEXT PRIMARY KEY,
  checkpoint_id TEXT NOT NULL UNIQUE,
  plan_id TEXT NOT NULL,
  step_id TEXT NOT NULL UNIQUE,
  job_id TEXT NOT NULL UNIQUE,
  proposal_id TEXT NOT NULL UNIQUE,
  approval_id TEXT,
  action_receipt_id TEXT UNIQUE,
  action_receipt_hash TEXT CHECK (action_receipt_hash IS NULL OR length(action_receipt_hash)=64),
  wrapper_job_receipt_id TEXT NOT NULL,
  wrapper_job_receipt_hash TEXT NOT NULL CHECK (length(wrapper_job_receipt_hash)=64),
  runtime_receipt_id TEXT NOT NULL UNIQUE,
  runtime_receipt_hash TEXT NOT NULL CHECK (length(runtime_receipt_hash)=64),
  runtime_plan_hash TEXT NOT NULL CHECK (length(runtime_plan_hash)=64),
  proposal_plan_hash TEXT NOT NULL CHECK (length(proposal_plan_hash)=64),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  outcome TEXT NOT NULL CHECK (outcome IN ('completed','failed','cancelled','denied')),
  result_code TEXT NOT NULL CHECK (length(result_code) BETWEEN 1 AND 120),
  safe_result_hash TEXT,
  phase16e_detail_code TEXT NOT NULL CHECK (length(phase16e_detail_code) BETWEEN 1 AND 120),
  receipt_hash TEXT NOT NULL UNIQUE CHECK (length(receipt_hash)=64),
  completed_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (checkpoint_id) REFERENCES agent_supervised_action_checkpoints(checkpoint_id) ON DELETE RESTRICT,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE RESTRICT,
  FOREIGN KEY (step_id) REFERENCES agent_runtime_plan_steps(step_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE RESTRICT,
  FOREIGN KEY (approval_id) REFERENCES agent_action_approvals(approval_id) ON DELETE RESTRICT,
  FOREIGN KEY (action_receipt_id) REFERENCES agent_action_receipts(receipt_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_job_receipt_id) REFERENCES wrapper_job_execution_receipts(receipt_id) ON DELETE RESTRICT,
  FOREIGN KEY (runtime_receipt_id) REFERENCES agent_runtime_receipts(receipt_id) ON DELETE RESTRICT
);

CREATE TRIGGER IF NOT EXISTS trg_supervised_action_receipts_no_update
BEFORE UPDATE ON agent_supervised_action_receipts
BEGIN
  SELECT RAISE(ABORT, 'supervised action receipts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_supervised_action_receipts_no_delete
BEFORE DELETE ON agent_supervised_action_receipts
BEGIN
  SELECT RAISE(ABORT, 'supervised action receipts are immutable');
END;

CREATE TABLE IF NOT EXISTS agent_supervised_compensation_receipts (
  compensation_receipt_id TEXT PRIMARY KEY,
  checkpoint_id TEXT NOT NULL UNIQUE,
  action_receipt_id TEXT NOT NULL,
  adapter_key TEXT NOT NULL CHECK (length(adapter_key) BETWEEN 1 AND 120),
  outcome TEXT NOT NULL CHECK (outcome IN ('completed','failed','denied')),
  result_code TEXT NOT NULL CHECK (length(result_code) BETWEEN 1 AND 120),
  target_hash TEXT NOT NULL CHECK (length(target_hash)=64),
  receipt_hash TEXT NOT NULL UNIQUE CHECK (length(receipt_hash)=64),
  completed_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (checkpoint_id) REFERENCES agent_supervised_action_checkpoints(checkpoint_id) ON DELETE RESTRICT,
  FOREIGN KEY (action_receipt_id) REFERENCES agent_action_receipts(receipt_id) ON DELETE RESTRICT
);

CREATE TRIGGER IF NOT EXISTS trg_supervised_compensation_receipts_no_update
BEFORE UPDATE ON agent_supervised_compensation_receipts
BEGIN
  SELECT RAISE(ABORT, 'supervised compensation receipts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_supervised_compensation_receipts_no_delete
BEFORE DELETE ON agent_supervised_compensation_receipts
BEGIN
  SELECT RAISE(ABORT, 'supervised compensation receipts are immutable');
END;

CREATE TABLE IF NOT EXISTS agent_supervised_action_events (
  event_id TEXT PRIMARY KEY,
  checkpoint_id TEXT,
  plan_id TEXT,
  step_id TEXT,
  job_id TEXT,
  proposal_id TEXT,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 120),
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('local_user','agent','worker','system')),
  actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 160),
  detail_code TEXT NOT NULL CHECK (length(detail_code) BETWEEN 1 AND 120),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  event_hash TEXT NOT NULL UNIQUE CHECK (length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (checkpoint_id) REFERENCES agent_supervised_action_checkpoints(checkpoint_id) ON DELETE SET NULL,
  FOREIGN KEY (plan_id) REFERENCES agent_runtime_plans(plan_id) ON DELETE SET NULL,
  FOREIGN KEY (step_id) REFERENCES agent_runtime_plan_steps(step_id) ON DELETE SET NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE SET NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_supervised_action_events_recent
  ON agent_supervised_action_events (created_at_utc DESC,event_id DESC);
CREATE INDEX IF NOT EXISTS idx_supervised_action_events_checkpoint
  ON agent_supervised_action_events (checkpoint_id,created_at_utc DESC,event_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_supervised_action_events_no_update
BEFORE UPDATE ON agent_supervised_action_events
BEGIN
  SELECT RAISE(ABORT, 'supervised action events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS trg_supervised_action_events_no_delete
BEFORE DELETE ON agent_supervised_action_events
BEGIN
  SELECT RAISE(ABORT, 'supervised action events are append-only');
END;

CREATE TABLE IF NOT EXISTS agent_supervised_action_state (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id=1),
  worker_id TEXT NOT NULL,
  orchestration_revision INTEGER NOT NULL DEFAULT 1 CHECK (orchestration_revision>=1),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','paused','stopped')),
  last_cycle_at_utc TEXT,
  last_error_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (worker_id) REFERENCES wrapper_job_workers(worker_id) ON DELETE RESTRICT
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0026_supervised_action_orchestration');
