-- Phase 16B: independently scoped wrapper capability grants.
-- Pairing remains authority-free. Grants are additive, expiring, auditable, and
-- bound to one wrapper connection. Cross-wrapper authority requires a separate
-- explicitly approved bridge grant.

CREATE TABLE IF NOT EXISTS wrapper_capability_catalog (
  capability_key TEXT PRIMARY KEY,
  description TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 300),
  risk_tier TEXT NOT NULL CHECK (risk_tier IN ('low','medium','high','critical')),
  default_approval_mode TEXT NOT NULL CHECK (
    default_approval_mode IN ('none','explicit','per_request')
  ),
  result_mode TEXT NOT NULL CHECK (
    result_mode IN ('safe_result','proposed_action','receipt_only','metadata_only')
  ),
  requires_scope INTEGER NOT NULL DEFAULT 0 CHECK (requires_scope IN (0,1)),
  allowed_operations_json TEXT NOT NULL CHECK (json_valid(allowed_operations_json)),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','deprecated','disabled')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_wrapper_capability_catalog_state
  ON wrapper_capability_catalog (state, risk_tier, capability_key);

CREATE TABLE IF NOT EXISTS wrapper_capability_grants (
  grant_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  capability_key TEXT NOT NULL,
  grant_revision INTEGER NOT NULL DEFAULT 1 CHECK (grant_revision >= 1),
  allowed_operations_json TEXT NOT NULL CHECK (json_valid(allowed_operations_json)),
  approval_mode TEXT NOT NULL CHECK (approval_mode IN ('none','explicit','per_request')),
  state TEXT NOT NULL CHECK (state IN (
    'pending_approval','active','suspended','expired','revoked','superseded'
  )),
  issued_by_user_id TEXT NOT NULL CHECK (length(issued_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  request_hash TEXT NOT NULL CHECK (length(request_hash) = 64),
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  approved_by_user_id TEXT,
  approved_at_utc TEXT,
  revoked_by_user_id TEXT,
  revoked_at_utc TEXT,
  revocation_reason TEXT,
  supersedes_grant_id TEXT,
  superseded_by_grant_id TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (capability_key) REFERENCES wrapper_capability_catalog(capability_key) ON DELETE RESTRICT,
  FOREIGN KEY (supersedes_grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE SET NULL,
  FOREIGN KEY (superseded_by_grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE SET NULL,
  UNIQUE (connection_id, capability_key, grant_revision)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_grants_authorize
  ON wrapper_capability_grants (
    connection_id, capability_key, state, not_before_utc, expires_at_utc, grant_revision DESC
  );
CREATE INDEX IF NOT EXISTS idx_wrapper_grants_wrapper_state
  ON wrapper_capability_grants (wrapper_id, state, updated_at_utc DESC, grant_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_wrapper_grants_one_active
  ON wrapper_capability_grants (connection_id, capability_key)
  WHERE state = 'active';

CREATE TABLE IF NOT EXISTS wrapper_dataset_scopes (
  scope_id TEXT PRIMARY KEY,
  grant_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL CHECK (scope_kind IN (
    'dataset','collection','record','tag','resource'
  )),
  scope_value TEXT NOT NULL CHECK (length(scope_value) BETWEEN 1 AND 240),
  allowed_fields_json TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(allowed_fields_json)),
  filter_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(filter_json)),
  result_policy TEXT NOT NULL CHECK (result_policy IN (
    'safe_result','metadata_only','aggregate_only','proposal_only','receipt_only'
  )),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','revoked')),
  created_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE CASCADE,
  UNIQUE (grant_id, scope_kind, scope_value)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_dataset_scopes_lookup
  ON wrapper_dataset_scopes (grant_id, state, scope_kind, scope_value);

CREATE TABLE IF NOT EXISTS wrapper_resource_limits (
  grant_id TEXT PRIMARY KEY,
  requests_per_minute INTEGER NOT NULL CHECK (requests_per_minute BETWEEN 1 AND 600),
  max_result_bytes INTEGER NOT NULL CHECK (max_result_bytes BETWEEN 1024 AND 1048576),
  max_daily_tokens INTEGER NOT NULL CHECK (max_daily_tokens BETWEEN 0 AND 100000000),
  max_concurrent_jobs INTEGER NOT NULL CHECK (max_concurrent_jobs BETWEEN 0 AND 32),
  max_queued_jobs INTEGER NOT NULL CHECK (max_queued_jobs BETWEEN 0 AND 1000),
  max_execution_seconds INTEGER NOT NULL CHECK (max_execution_seconds BETWEEN 1 AND 3600),
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS wrapper_bridge_grants (
  bridge_id TEXT PRIMARY KEY,
  source_wrapper_id TEXT NOT NULL,
  source_connection_id TEXT NOT NULL,
  target_wrapper_id TEXT NOT NULL,
  target_connection_id TEXT NOT NULL,
  capability_key TEXT NOT NULL,
  allowed_operations_json TEXT NOT NULL CHECK (json_valid(allowed_operations_json)),
  scope_kind TEXT NOT NULL CHECK (scope_kind IN (
    'dataset','collection','record','tag','resource'
  )),
  scope_value TEXT NOT NULL CHECK (length(scope_value) BETWEEN 1 AND 240),
  result_policy TEXT NOT NULL CHECK (result_policy IN (
    'safe_result','metadata_only','aggregate_only','proposal_only','receipt_only'
  )),
  approval_mode TEXT NOT NULL CHECK (approval_mode IN ('explicit')),
  state TEXT NOT NULL CHECK (state IN (
    'pending_approval','active','suspended','expired','revoked'
  )),
  issued_by_user_id TEXT NOT NULL CHECK (length(issued_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  request_hash TEXT NOT NULL CHECK (length(request_hash) = 64),
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  approved_by_user_id TEXT,
  approved_at_utc TEXT,
  revoked_by_user_id TEXT,
  revoked_at_utc TEXT,
  revocation_reason TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (source_wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (source_connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (target_wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (target_connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (capability_key) REFERENCES wrapper_capability_catalog(capability_key) ON DELETE RESTRICT,
  CHECK (source_wrapper_id <> target_wrapper_id)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_bridge_authorize
  ON wrapper_bridge_grants (
    source_connection_id, target_connection_id, capability_key, state, expires_at_utc
  );
CREATE INDEX IF NOT EXISTS idx_wrapper_bridge_target
  ON wrapper_bridge_grants (target_wrapper_id, state, updated_at_utc DESC, bridge_id);

CREATE TABLE IF NOT EXISTS wrapper_grant_approvals (
  approval_id TEXT PRIMARY KEY,
  grant_id TEXT,
  bridge_id TEXT,
  approval_action TEXT NOT NULL CHECK (approval_action IN (
    'grant_create','grant_rotate','sensitive_use','bridge_create'
  )),
  plan_hash TEXT NOT NULL CHECK (length(plan_hash) = 64),
  state TEXT NOT NULL CHECK (state IN (
    'pending','approved','rejected','expired','revoked','consumed'
  )),
  requested_by_user_id TEXT NOT NULL CHECK (length(requested_by_user_id) BETWEEN 1 AND 160),
  decided_by_user_id TEXT,
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  decided_at_utc TEXT,
  consumed_at_utc TEXT,
  reason TEXT,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE CASCADE,
  FOREIGN KEY (bridge_id) REFERENCES wrapper_bridge_grants(bridge_id) ON DELETE CASCADE,
  CHECK (
    (grant_id IS NOT NULL AND bridge_id IS NULL)
    OR (grant_id IS NULL AND bridge_id IS NOT NULL)
  ),
  UNIQUE (grant_id, approval_action, plan_hash),
  UNIQUE (bridge_id, approval_action, plan_hash)
);

CREATE INDEX IF NOT EXISTS idx_wrapper_grant_approvals_pending
  ON wrapper_grant_approvals (state, expires_at_utc, approval_action, approval_id);

CREATE TABLE IF NOT EXISTS wrapper_grant_usage_windows (
  grant_id TEXT NOT NULL,
  window_kind TEXT NOT NULL CHECK (window_kind IN ('minute','day')),
  window_start_utc TEXT NOT NULL,
  request_count INTEGER NOT NULL DEFAULT 0 CHECK (request_count >= 0),
  result_bytes INTEGER NOT NULL DEFAULT 0 CHECK (result_bytes >= 0),
  token_count INTEGER NOT NULL DEFAULT 0 CHECK (token_count >= 0),
  active_jobs INTEGER NOT NULL DEFAULT 0 CHECK (active_jobs >= 0),
  queued_jobs INTEGER NOT NULL DEFAULT 0 CHECK (queued_jobs >= 0),
  updated_at_utc TEXT NOT NULL,
  PRIMARY KEY (grant_id, window_kind, window_start_utc),
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_wrapper_grant_usage_retention
  ON wrapper_grant_usage_windows (window_kind, window_start_utc);

CREATE TABLE IF NOT EXISTS wrapper_grant_revocation_fences (
  connection_id TEXT PRIMARY KEY,
  grant_revision INTEGER NOT NULL CHECK (grant_revision >= 0),
  reason TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 500),
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS wrapper_grant_events (
  event_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  grant_id TEXT,
  bridge_id TEXT,
  event_type TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  actor_user_id TEXT,
  correlation_id TEXT,
  detail_code TEXT,
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE SET NULL,
  FOREIGN KEY (bridge_id) REFERENCES wrapper_bridge_grants(bridge_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_wrapper_grant_events_wrapper
  ON wrapper_grant_events (wrapper_id, created_at_utc DESC, event_id DESC);
CREATE INDEX IF NOT EXISTS idx_wrapper_grant_events_connection
  ON wrapper_grant_events (connection_id, created_at_utc DESC, event_id DESC);

CREATE TABLE IF NOT EXISTS wrapper_authorization_receipts (
  decision_id TEXT PRIMARY KEY,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  grant_id TEXT,
  bridge_id TEXT,
  capability_key TEXT NOT NULL,
  operation TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('allowed','denied')),
  detail_code TEXT NOT NULL,
  grant_revision INTEGER NOT NULL DEFAULT 0 CHECK (grant_revision >= 0),
  scope_hash TEXT,
  result_policy TEXT,
  correlation_id TEXT,
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE CASCADE,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE CASCADE,
  FOREIGN KEY (grant_id) REFERENCES wrapper_capability_grants(grant_id) ON DELETE SET NULL,
  FOREIGN KEY (bridge_id) REFERENCES wrapper_bridge_grants(bridge_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_wrapper_authorization_receipts_connection
  ON wrapper_authorization_receipts (connection_id, created_at_utc DESC, decision_id DESC);
CREATE INDEX IF NOT EXISTS idx_wrapper_authorization_receipts_outcome
  ON wrapper_authorization_receipts (outcome, created_at_utc DESC, decision_id DESC);

INSERT OR IGNORE INTO wrapper_capability_catalog (
  capability_key, description, risk_tier, default_approval_mode, result_mode,
  requires_scope, allowed_operations_json
) VALUES
  ('wrapper.status.read','Read the paired wrapper connection and health status.','low','none','metadata_only',0,'["read"]'),
  ('settings.read','Read explicitly shareable non-secret settings.','low','none','safe_result',1,'["read"]'),
  ('settings.update','Propose or apply an explicitly scoped non-secret setting change.','high','explicit','proposed_action',1,'["propose","update"]'),
  ('knowledge.search','Search HomeServer private knowledge and return only filtered result summaries.','medium','explicit','safe_result',1,'["search"]'),
  ('knowledge.result.read','Read a previously authorized safe knowledge result.','medium','explicit','safe_result',1,'["read"]'),
  ('model.inference.request','Request bounded model inference with authorized context only.','medium','explicit','safe_result',1,'["request"]'),
  ('agent.job.propose','Propose a bounded agent job without executing it.','medium','explicit','proposed_action',1,'["propose"]'),
  ('agent.job.read','Read safe job status and receipts for the same wrapper.','low','none','receipt_only',1,'["read"]'),
  ('action.propose','Propose a sensitive external action for explicit user approval.','critical','per_request','proposed_action',1,'["propose"]'),
  ('receipt.read','Read safe execution and authorization receipts for the same wrapper.','low','none','receipt_only',1,'["read"]');

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0021_wrapper_capability_grants');
