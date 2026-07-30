-- Phase 16D: HomeServer-owned agent lifecycle and sensitive action authority.
-- Agents are private HomeServer principals. Wrappers receive only independently scoped
-- assignments and capability bindings; assignment never implies HomeServer-wide authority.

CREATE TABLE IF NOT EXISTS homeserver_agents (
  agent_id TEXT PRIMARY KEY,
  owner_user_id TEXT NOT NULL CHECK (length(owner_user_id) BETWEEN 1 AND 160),
  display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 120),
  purpose TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 500),
  description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 4000),
  state TEXT NOT NULL DEFAULT 'draft' CHECK (state IN (
    'draft','active','suspended','revoked','expired'
  )),
  autonomy_level INTEGER NOT NULL DEFAULT 0 CHECK (autonomy_level BETWEEN 0 AND 4),
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  allowed_job_types_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_job_types_json)),
  model_restrictions_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(model_restrictions_json)),
  tool_restrictions_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(tool_restrictions_json)),
  expires_at_utc TEXT NOT NULL,
  activated_at_utc TEXT,
  suspended_at_utc TEXT,
  revoked_at_utc TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_homeserver_agents_state
  ON homeserver_agents (state, expires_at_utc, updated_at_utc DESC, agent_id);

CREATE TABLE IF NOT EXISTS wrapper_agent_assignments (
  assignment_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL DEFAULT 1 CHECK (assignment_revision >= 1),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN (
    'active','suspended','revoked','expired'
  )),
  assigned_by_user_id TEXT NOT NULL CHECK (length(assigned_by_user_id) BETWEEN 1 AND 160),
  purpose TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 500),
  allowed_job_types_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_job_types_json)),
  expires_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  UNIQUE (agent_id, connection_id, assignment_revision)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_wrapper_agent_one_active
  ON wrapper_agent_assignments (agent_id, connection_id)
  WHERE state='active';

CREATE INDEX IF NOT EXISTS idx_wrapper_agent_connection
  ON wrapper_agent_assignments (connection_id, state, updated_at_utc DESC, assignment_id);

CREATE TABLE IF NOT EXISTS agent_capability_bindings (
  binding_id TEXT PRIMARY KEY,
  assignment_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  capability_key TEXT NOT NULL,
  allowed_operations_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_operations_json)),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','suspended','revoked','expired')),
  expires_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE CASCADE,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE CASCADE,
  FOREIGN KEY (capability_key) REFERENCES wrapper_capability_catalog(capability_key) ON DELETE RESTRICT,
  UNIQUE (assignment_id, grant_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_capability_bindings_authorize
  ON agent_capability_bindings (assignment_id, grant_id, state, expires_at_utc, binding_id);

CREATE TABLE IF NOT EXISTS agent_execution_policies (
  policy_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL DEFAULT 1 CHECK (policy_revision >= 1),
  action_type TEXT NOT NULL CHECK (length(action_type) BETWEEN 1 AND 120),
  risk_class TEXT NOT NULL CHECK (risk_class IN (
    'read_only','reversible','external_side_effect','high_risk'
  )),
  approval_mode TEXT NOT NULL CHECK (approval_mode IN (
    'always','per_action','none'
  )),
  tool_adapter TEXT NOT NULL CHECK (length(tool_adapter) BETWEEN 1 AND 120),
  max_executions INTEGER NOT NULL DEFAULT 1 CHECK (max_executions BETWEEN 1 AND 10000),
  window_seconds INTEGER NOT NULL DEFAULT 3600 CHECK (window_seconds BETWEEN 60 AND 2592000),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','suspended','revoked','expired')),
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  created_by_user_id TEXT NOT NULL CHECK (length(created_by_user_id) BETWEEN 1 AND 160),
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  UNIQUE (agent_id, action_type, policy_revision)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_policy_one_active
  ON agent_execution_policies (agent_id, action_type)
  WHERE state='active';

CREATE INDEX IF NOT EXISTS idx_agent_policy_authorize
  ON agent_execution_policies (agent_id, state, risk_class, expires_at_utc, policy_id);

CREATE TABLE IF NOT EXISTS agent_job_bindings (
  job_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  agent_revision INTEGER NOT NULL CHECK (agent_revision >= 1),
  assignment_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL CHECK (assignment_revision >= 1),
  binding_id TEXT NOT NULL,
  policy_context_hash TEXT NOT NULL CHECK (length(policy_context_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE CASCADE,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE CASCADE,
  FOREIGN KEY (binding_id) REFERENCES agent_capability_bindings(binding_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_job_bindings_agent
  ON agent_job_bindings (agent_id, created_at_utc DESC, job_id);

CREATE TABLE IF NOT EXISTS agent_action_proposals (
  proposal_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  agent_revision INTEGER NOT NULL CHECK (agent_revision >= 1),
  assignment_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL CHECK (assignment_revision >= 1),
  job_id TEXT NOT NULL UNIQUE,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  connection_authority_revision INTEGER NOT NULL CHECK (connection_authority_revision >= 0),
  authorization_decision_id TEXT NOT NULL,
  policy_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL CHECK (policy_revision >= 1),
  action_type TEXT NOT NULL CHECK (length(action_type) BETWEEN 1 AND 120),
  risk_class TEXT NOT NULL CHECK (risk_class IN (
    'read_only','reversible','external_side_effect','high_risk'
  )),
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  rationale TEXT NOT NULL CHECK (length(rationale) BETWEEN 1 AND 4000),
  safe_summary_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(safe_summary_json)),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  plan_hash TEXT NOT NULL UNIQUE CHECK (length(plan_hash)=64),
  state TEXT NOT NULL CHECK (state IN (
    'proposed','awaiting_approval','approved','executing','completed','failed',
    'rejected','cancelled','expired'
  )),
  approval_required INTEGER NOT NULL CHECK (approval_required IN (0,1)),
  expires_at_utc TEXT NOT NULL,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE CASCADE,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE RESTRICT,
  FOREIGN KEY (policy_id) REFERENCES agent_execution_policies(policy_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_action_proposals_connection
  ON agent_action_proposals (connection_id, state, created_at_utc DESC, proposal_id);
CREATE INDEX IF NOT EXISTS idx_agent_action_proposals_agent
  ON agent_action_proposals (agent_id, state, expires_at_utc, proposal_id);

CREATE TABLE IF NOT EXISTS agent_action_private_payloads (
  proposal_id TEXT PRIMARY KEY,
  classification TEXT NOT NULL DEFAULT 'private' CHECK (classification='private'),
  private_payload_json TEXT NOT NULL CHECK (json_valid(private_payload_json)),
  payload_bytes INTEGER NOT NULL CHECK (payload_bytes BETWEEN 2 AND 1048576),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_action_approvals (
  approval_id TEXT PRIMARY KEY,
  proposal_id TEXT NOT NULL UNIQUE,
  plan_hash TEXT NOT NULL CHECK (length(plan_hash)=64),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  agent_revision INTEGER NOT NULL CHECK (agent_revision >= 1),
  assignment_revision INTEGER NOT NULL CHECK (assignment_revision >= 1),
  policy_revision INTEGER NOT NULL CHECK (policy_revision >= 1),
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 1),
  connection_authority_revision INTEGER NOT NULL CHECK (connection_authority_revision >= 0),
  state TEXT NOT NULL CHECK (state IN (
    'pending','approved','rejected','consumed','expired','cancelled'
  )),
  requested_by_user_id TEXT NOT NULL CHECK (length(requested_by_user_id) BETWEEN 1 AND 160),
  decided_by_user_id TEXT,
  decision_reason TEXT,
  requested_at_utc TEXT NOT NULL,
  decided_at_utc TEXT,
  consumed_at_utc TEXT,
  expires_at_utc TEXT NOT NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_agent_action_approvals_pending
  ON agent_action_approvals (state, expires_at_utc, approval_id);

CREATE TABLE IF NOT EXISTS agent_action_attempts (
  attempt_id TEXT PRIMARY KEY,
  proposal_id TEXT NOT NULL,
  approval_id TEXT,
  tool_adapter TEXT NOT NULL,
  idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) BETWEEN 8 AND 160),
  state TEXT NOT NULL CHECK (state IN ('executing','completed','failed','cancelled')),
  result_code TEXT,
  safe_result_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(safe_result_json)),
  safe_result_hash TEXT,
  started_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE RESTRICT,
  FOREIGN KEY (approval_id) REFERENCES agent_action_approvals(approval_id) ON DELETE RESTRICT,
  UNIQUE (proposal_id)
);

CREATE TABLE IF NOT EXISTS agent_action_private_results (
  attempt_id TEXT PRIMARY KEY,
  classification TEXT NOT NULL DEFAULT 'private' CHECK (classification='private'),
  private_result_json TEXT NOT NULL CHECK (json_valid(private_result_json)),
  private_result_hash TEXT NOT NULL CHECK (length(private_result_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (attempt_id) REFERENCES agent_action_attempts(attempt_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_action_receipts (
  receipt_id TEXT PRIMARY KEY,
  proposal_id TEXT NOT NULL UNIQUE,
  attempt_id TEXT NOT NULL UNIQUE,
  agent_id TEXT NOT NULL,
  agent_revision INTEGER NOT NULL,
  assignment_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL,
  job_id TEXT NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  grant_id TEXT NOT NULL,
  grant_revision INTEGER NOT NULL,
  connection_authority_revision INTEGER NOT NULL,
  authorization_decision_id TEXT NOT NULL,
  policy_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL,
  approval_id TEXT,
  plan_hash TEXT NOT NULL CHECK (length(plan_hash)=64),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  action_type TEXT NOT NULL,
  risk_class TEXT NOT NULL,
  tool_adapter TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('completed','failed','cancelled','denied')),
  result_code TEXT NOT NULL,
  safe_result_hash TEXT,
  receipt_hash TEXT NOT NULL UNIQUE CHECK (length(receipt_hash)=64),
  completed_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE RESTRICT,
  FOREIGN KEY (attempt_id) REFERENCES agent_action_attempts(attempt_id) ON DELETE RESTRICT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id) ON DELETE RESTRICT
);

CREATE TRIGGER IF NOT EXISTS trg_agent_action_receipts_no_update
BEFORE UPDATE ON agent_action_receipts
BEGIN
  SELECT RAISE(ABORT, 'agent action receipts are immutable');
END;

CREATE TABLE IF NOT EXISTS agent_lifecycle_events (
  event_id TEXT PRIMARY KEY,
  agent_id TEXT,
  wrapper_id TEXT,
  connection_id TEXT,
  assignment_id TEXT,
  proposal_id TEXT,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 120),
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('local_user','wrapper','agent','system')),
  actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 160),
  detail_code TEXT NOT NULL CHECK (length(detail_code) BETWEEN 1 AND 120),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  event_hash TEXT NOT NULL UNIQUE CHECK (length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE SET NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE SET NULL,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE SET NULL,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE SET NULL,
  FOREIGN KEY (proposal_id) REFERENCES agent_action_proposals(proposal_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_events_recent
  ON agent_lifecycle_events (agent_id, created_at_utc DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_lifecycle_events_connection
  ON agent_lifecycle_events (connection_id, created_at_utc DESC, event_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_agent_lifecycle_events_no_update
BEFORE UPDATE ON agent_lifecycle_events
BEGIN
  SELECT RAISE(ABORT, 'agent lifecycle events are append-only');
END;

CREATE TABLE IF NOT EXISTS agent_emergency_stops (
  stop_id TEXT PRIMARY KEY,
  scope_type TEXT NOT NULL CHECK (scope_type IN ('global','agent','wrapper','connection')),
  agent_id TEXT,
  wrapper_id TEXT,
  connection_id TEXT,
  state TEXT NOT NULL CHECK (state IN ('active','released','expired')),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 1000),
  stop_hash TEXT NOT NULL UNIQUE CHECK (length(stop_hash)=64),
  activated_by_user_id TEXT NOT NULL CHECK (length(activated_by_user_id) BETWEEN 1 AND 160),
  activated_at_utc TEXT NOT NULL,
  expires_at_utc TEXT,
  released_by_user_id TEXT,
  released_at_utc TEXT,
  release_reason TEXT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE CASCADE,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  CHECK (
    (scope_type='global' AND agent_id IS NULL AND wrapper_id IS NULL AND connection_id IS NULL)
    OR (scope_type='agent' AND agent_id IS NOT NULL AND wrapper_id IS NULL AND connection_id IS NULL)
    OR (scope_type='wrapper' AND agent_id IS NULL AND wrapper_id IS NOT NULL AND connection_id IS NULL)
    OR (scope_type='connection' AND agent_id IS NULL AND wrapper_id IS NULL AND connection_id IS NOT NULL)
  )
);

CREATE INDEX IF NOT EXISTS idx_agent_emergency_stops_active
  ON agent_emergency_stops (state, scope_type, activated_at_utc DESC, stop_id);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0023_wrapper_agents_and_action_approvals');
