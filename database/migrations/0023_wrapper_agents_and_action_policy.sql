-- Phase 16D: HomeServer-owned agent lifecycle and sensitive action authority.
-- Agents are bounded actors, never owners. Wrapper assignment grants no authority by itself.
-- Private action payloads remain local; wrapper-visible records contain hashes and safe summaries only.

CREATE TABLE IF NOT EXISTS homeserver_agents (
  agent_id TEXT PRIMARY KEY,
  worker_id TEXT NOT NULL UNIQUE,
  owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 160),
  display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 120),
  purpose TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 1000),
  lifecycle_state TEXT NOT NULL DEFAULT 'draft' CHECK (lifecycle_state IN (
    'draft','active','suspended','revoked','expired'
  )),
  autonomy_level INTEGER NOT NULL DEFAULT 0 CHECK (autonomy_level BETWEEN 0 AND 4),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  allowed_job_types_json TEXT NOT NULL CHECK (json_valid(allowed_job_types_json)),
  allowed_model_ids_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_model_ids_json)),
  allowed_tool_ids_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_tool_ids_json)),
  expires_at_utc TEXT,
  activated_at_utc TEXT,
  suspended_at_utc TEXT,
  revoked_at_utc TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (worker_id) REFERENCES wrapper_job_workers(worker_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_homeserver_agents_state
  ON homeserver_agents (lifecycle_state, updated_at_utc DESC, agent_id);
CREATE INDEX IF NOT EXISTS idx_homeserver_agents_owner
  ON homeserver_agents (owner_user_id, lifecycle_state, updated_at_utc DESC);

CREATE TABLE IF NOT EXISTS wrapper_agent_assignments (
  assignment_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL DEFAULT 1 CHECK (assignment_revision >= 1),
  state TEXT NOT NULL CHECK (state IN ('active','suspended','revoked','expired')),
  assigned_by_user_id TEXT NOT NULL CHECK (length(assigned_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  revoked_by_user_id TEXT,
  revoked_at_utc TEXT,
  revocation_reason TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (agent_id, connection_id, assignment_revision)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_wrapper_agent_one_active_assignment
  ON wrapper_agent_assignments (agent_id, connection_id)
  WHERE state='active';
CREATE INDEX IF NOT EXISTS idx_wrapper_agent_assignments_connection
  ON wrapper_agent_assignments (connection_id, state, expires_at_utc, agent_id);

CREATE TABLE IF NOT EXISTS agent_capability_bindings (
  binding_id TEXT PRIMARY KEY,
  assignment_id TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  capability_key TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  allowed_operations_json TEXT NOT NULL CHECK (json_valid(allowed_operations_json)),
  state TEXT NOT NULL CHECK (state IN ('active','suspended','revoked','expired')),
  issued_by_user_id TEXT NOT NULL CHECK (length(issued_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  revoked_by_user_id TEXT,
  revoked_at_utc TEXT,
  revocation_reason TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE CASCADE,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE CASCADE,
  FOREIGN KEY (capability_key) REFERENCES wrapper_capability_catalog(capability_key) ON DELETE RESTRICT,
  UNIQUE (assignment_id, grant_id, capability_key, grant_revision)
);

CREATE INDEX IF NOT EXISTS idx_agent_capability_bindings_authorize
  ON agent_capability_bindings (
    agent_id, assignment_id, capability_key, state, not_before_utc, expires_at_utc
  );

CREATE TABLE IF NOT EXISTS agent_execution_policies (
  policy_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL CHECK (policy_revision >= 1),
  max_autonomy_level INTEGER NOT NULL CHECK (max_autonomy_level BETWEEN 0 AND 4),
  approval_mode TEXT NOT NULL CHECK (approval_mode IN (
    'disabled','per_action','standing_low_risk'
  )),
  allowed_risk_classes_json TEXT NOT NULL CHECK (json_valid(allowed_risk_classes_json)),
  allowed_action_types_json TEXT NOT NULL CHECK (json_valid(allowed_action_types_json)),
  allowed_adapter_ids_json TEXT NOT NULL CHECK (json_valid(allowed_adapter_ids_json)),
  max_executions_per_hour INTEGER NOT NULL CHECK (max_executions_per_hour BETWEEN 0 AND 1000),
  max_executions_per_day INTEGER NOT NULL CHECK (max_executions_per_day BETWEEN 0 AND 10000),
  state TEXT NOT NULL CHECK (state IN ('active','suspended','revoked','expired')),
  issued_by_user_id TEXT NOT NULL CHECK (length(issued_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  revoked_by_user_id TEXT,
  revoked_at_utc TEXT,
  revocation_reason TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  UNIQUE (agent_id, policy_revision)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_one_active_policy
  ON agent_execution_policies (agent_id)
  WHERE state='active';
CREATE INDEX IF NOT EXISTS idx_agent_execution_policies_active
  ON agent_execution_policies (agent_id, state, not_before_utc, expires_at_utc);

CREATE TABLE IF NOT EXISTS agent_action_proposals (
  proposal_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  agent_revision INTEGER NOT NULL CHECK (agent_revision >= 1),
  assignment_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL CHECK (assignment_revision >= 1),
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  job_request_hash TEXT NOT NULL CHECK (length(job_request_hash)=64),
  job_payload_hash TEXT NOT NULL CHECK (length(job_payload_hash)=64),
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  connection_authority_revision INTEGER NOT NULL CHECK (connection_authority_revision >= 0),
  action_type TEXT NOT NULL CHECK (length(action_type) BETWEEN 1 AND 120),
  risk_class TEXT NOT NULL CHECK (risk_class IN (
    'read_only','reversible','external_side_effect','high_risk'
  )),
  adapter_id TEXT NOT NULL CHECK (length(adapter_id) BETWEEN 1 AND 160),
  public_summary TEXT NOT NULL CHECK (length(public_summary) BETWEEN 1 AND 2000),
  private_payload_hash TEXT NOT NULL CHECK (length(private_payload_hash)=64),
  plan_hash TEXT NOT NULL UNIQUE CHECK (length(plan_hash)=64),
  requires_fresh_approval INTEGER NOT NULL CHECK (requires_fresh_approval IN (0,1)),
  state TEXT NOT NULL CHECK (state IN (
    'awaiting_approval','approved','rejected','executing','completed','failed','cancelled','expired'
  )),
  requested_by_type TEXT NOT NULL CHECK (requested_by_type IN (
    'wrapper','local_user','agent','system'
  )),
  requested_by_id TEXT NOT NULL CHECK (length(requested_by_id) BETWEEN 1 AND 160),
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  failure_code TEXT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_action_proposals_connection
  ON agent_action_proposals (connection_id, state, created_at_utc DESC, proposal_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_action_proposals_agent
  ON agent_action_proposals (agent_id, state, expires_at_utc, proposal_id);

CREATE TABLE IF NOT EXISTS agent_action_private_payloads (
  proposal_id TEXT PRIMARY KEY,
  classification TEXT NOT NULL DEFAULT 'private' CHECK (classification='private'),
  private_payload_json TEXT NOT NULL CHECK (json_valid(private_payload_json)),
  private_payload_hash TEXT NOT NULL CHECK (length(private_payload_hash)=64),
  private_payload_bytes INTEGER NOT NULL CHECK (private_payload_bytes BETWEEN 2 AND 1048576),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_action_approvals (
  approval_id TEXT PRIMARY KEY,
  proposal_id TEXT NOT NULL UNIQUE,
  agent_id TEXT NOT NULL,
  assignment_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  plan_hash TEXT NOT NULL CHECK (length(plan_hash)=64),
  private_payload_hash TEXT NOT NULL CHECK (length(private_payload_hash)=64),
  agent_revision INTEGER NOT NULL CHECK (agent_revision >= 1),
  assignment_revision INTEGER NOT NULL CHECK (assignment_revision >= 1),
  job_request_hash TEXT NOT NULL CHECK (length(job_request_hash)=64),
  state TEXT NOT NULL CHECK (state IN (
    'pending','approved','rejected','consumed','revoked','expired'
  )),
  requested_at_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  decided_by_user_id TEXT,
  decided_at_utc TEXT,
  decision_reason TEXT,
  consumed_at_utc TEXT,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE CASCADE,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE CASCADE,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_action_approvals_pending
  ON agent_action_approvals (state, expires_at_utc, approval_id);

CREATE TABLE IF NOT EXISTS agent_action_attempts (
  attempt_id TEXT PRIMARY KEY,
  proposal_id TEXT NOT NULL,
  approval_id TEXT,
  agent_id TEXT NOT NULL,
  adapter_id TEXT NOT NULL,
  execution_token_hash TEXT NOT NULL UNIQUE CHECK (length(execution_token_hash)=64),
  state TEXT NOT NULL CHECK (state IN (
    'authorized','running','completed','failed','cancelled','expired'
  )),
  authorized_by_type TEXT NOT NULL CHECK (authorized_by_type IN ('approval','standing_policy')),
  authorized_by_id TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  started_at_utc TEXT,
  completed_at_utc TEXT,
  outcome_code TEXT,
  safe_summary_hash TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE CASCADE,
  FOREIGN KEY (approval_id) REFERENCES agent_action_approvals(approval_id) ON DELETE SET NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_one_active_attempt
  ON agent_action_attempts (proposal_id)
  WHERE state IN ('authorized','running');
CREATE INDEX IF NOT EXISTS idx_agent_action_attempts_expiration
  ON agent_action_attempts (state, expires_at_utc, attempt_id);

CREATE TABLE IF NOT EXISTS agent_action_receipts (
  receipt_id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL UNIQUE,
  proposal_id TEXT NOT NULL,
  approval_id TEXT,
  agent_id TEXT NOT NULL,
  agent_revision INTEGER NOT NULL,
  assignment_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  plan_hash TEXT NOT NULL CHECK (length(plan_hash)=64),
  private_payload_hash TEXT NOT NULL CHECK (length(private_payload_hash)=64),
  action_type TEXT NOT NULL,
  risk_class TEXT NOT NULL,
  adapter_id TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('completed','failed','cancelled','expired')),
  result_code TEXT NOT NULL CHECK (length(result_code) BETWEEN 1 AND 120),
  safe_summary TEXT NOT NULL CHECK (length(safe_summary) BETWEEN 1 AND 2000),
  safe_summary_hash TEXT NOT NULL CHECK (length(safe_summary_hash)=64),
  receipt_hash TEXT NOT NULL UNIQUE CHECK (length(receipt_hash)=64),
  completed_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (attempt_id) REFERENCES agent_action_attempts(attempt_id) ON DELETE RESTRICT,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE RESTRICT,
  FOREIGN KEY (approval_id) REFERENCES agent_action_approvals(approval_id) ON DELETE SET NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_action_receipts_connection
  ON agent_action_receipts (connection_id, completed_at_utc DESC, receipt_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_action_receipts_agent
  ON agent_action_receipts (agent_id, completed_at_utc DESC, receipt_id DESC);

CREATE TABLE IF NOT EXISTS agent_lifecycle_events (
  event_id TEXT PRIMARY KEY,
  agent_id TEXT,
  assignment_id TEXT,
  proposal_id TEXT,
  wrapper_id TEXT,
  connection_id TEXT,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 120),
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('local_user','wrapper','agent','adapter','system')),
  actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 160),
  detail_code TEXT NOT NULL CHECK (length(detail_code) BETWEEN 1 AND 120),
  visibility TEXT NOT NULL CHECK (visibility IN ('internal','security','wrapper')),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  event_hash TEXT NOT NULL UNIQUE CHECK (length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE SET NULL,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE SET NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE SET NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE SET NULL,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_events_recent
  ON agent_lifecycle_events (created_at_utc DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_events_connection
  ON agent_lifecycle_events (connection_id, visibility, created_at_utc DESC, event_id DESC);

CREATE TABLE IF NOT EXISTS agent_emergency_stops (
  stop_id TEXT PRIMARY KEY,
  state TEXT NOT NULL CHECK (state IN ('active','released')),
  engaged_by_user_id TEXT NOT NULL CHECK (length(engaged_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 1000),
  engaged_at_utc TEXT NOT NULL,
  released_by_user_id TEXT,
  release_reason TEXT,
  released_at_utc TEXT,
  updated_at_utc TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_one_active_emergency_stop
  ON agent_emergency_stops (state)
  WHERE state='active';

CREATE TRIGGER IF NOT EXISTS trg_agent_lifecycle_events_no_update
BEFORE UPDATE ON agent_lifecycle_events
BEGIN
  SELECT RAISE(ABORT, 'agent lifecycle events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_action_receipts_no_update
BEFORE UPDATE ON agent_action_receipts
BEGIN
  SELECT RAISE(ABORT, 'agent action receipts are immutable');
END;

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0023_wrapper_agents_and_action_policy');
