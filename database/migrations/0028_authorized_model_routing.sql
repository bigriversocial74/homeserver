-- Phase 20: authorized model routing and inference governance.
-- Providers remain bounded adapters. HomeServer owns every routing decision,
-- authority snapshot, budget reservation, private result, and immutable receipt.

CREATE TABLE IF NOT EXISTS model_routing_policies (
  policy_id TEXT PRIMARY KEY,
  subject_type TEXT NOT NULL CHECK(subject_type IN ('local_control_center','agent_assignment')),
  subject_id TEXT NOT NULL CHECK(length(subject_id) BETWEEN 1 AND 180),
  agent_id TEXT,
  agent_revision INTEGER,
  assignment_id TEXT,
  assignment_revision INTEGER,
  wrapper_id TEXT,
  connection_id TEXT,
  connection_authority_revision INTEGER,
  purpose TEXT NOT NULL CHECK(length(purpose) BETWEEN 1 AND 500),
  purpose_hash TEXT NOT NULL CHECK(length(purpose_hash)=64),
  allowed_data_classes_json TEXT NOT NULL CHECK(json_valid(allowed_data_classes_json)),
  provider_order_json TEXT NOT NULL CHECK(json_valid(provider_order_json)),
  allowed_models_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(allowed_models_json)),
  allow_fallback INTEGER NOT NULL DEFAULT 0 CHECK(allow_fallback IN (0,1)),
  remote_context_mode TEXT NOT NULL DEFAULT 'deny' CHECK(remote_context_mode IN ('deny','public_only','approved_selector')),
  require_zdr INTEGER NOT NULL DEFAULT 1 CHECK(require_zdr IN (0,1)),
  max_input_chars INTEGER NOT NULL CHECK(max_input_chars BETWEEN 1 AND 30000),
  max_output_tokens INTEGER NOT NULL CHECK(max_output_tokens BETWEEN 16 AND 4096),
  window_seconds INTEGER NOT NULL CHECK(window_seconds BETWEEN 60 AND 2592000),
  max_requests INTEGER NOT NULL CHECK(max_requests BETWEEN 1 AND 1000000),
  max_total_tokens INTEGER NOT NULL CHECK(max_total_tokens BETWEEN 16 AND 1000000000),
  max_spend_microusd INTEGER NOT NULL CHECK(max_spend_microusd BETWEEN 0 AND 1000000000000),
  policy_revision INTEGER NOT NULL CHECK(policy_revision >= 1),
  policy_hash TEXT NOT NULL CHECK(length(policy_hash)=64),
  state TEXT NOT NULL CHECK(state IN ('active','superseded','suspended','revoked','expired')),
  created_by_user_id TEXT NOT NULL CHECK(length(created_by_user_id) BETWEEN 1 AND 160),
  reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 500),
  not_before_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  revoked_at_utc TEXT,
  FOREIGN KEY(agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY(assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  CHECK(
    (subject_type='local_control_center' AND subject_id='local_control_center' AND agent_id IS NULL AND assignment_id IS NULL AND wrapper_id IS NULL AND connection_id IS NULL)
    OR
    (subject_type='agent_assignment' AND agent_id IS NOT NULL AND assignment_id IS NOT NULL AND wrapper_id IS NOT NULL AND connection_id IS NOT NULL AND agent_revision >= 1 AND assignment_revision >= 1 AND connection_authority_revision >= 0)
  )
);
CREATE INDEX IF NOT EXISTS idx_model_routing_policy_subject
  ON model_routing_policies(subject_type,subject_id,state,expires_at_utc,policy_revision DESC,policy_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_model_routing_one_active_policy
  ON model_routing_policies(subject_type,subject_id,purpose_hash)
  WHERE state='active';

CREATE TABLE IF NOT EXISTS model_inference_requests (
  request_id TEXT PRIMARY KEY,
  idempotency_key TEXT NOT NULL UNIQUE CHECK(length(idempotency_key) BETWEEN 16 AND 240),
  request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
  subject_type TEXT NOT NULL CHECK(subject_type IN ('local_control_center','agent_assignment')),
  subject_id TEXT NOT NULL,
  agent_id TEXT,
  agent_revision INTEGER,
  assignment_id TEXT,
  assignment_revision INTEGER,
  wrapper_id TEXT,
  connection_id TEXT,
  connection_authority_revision INTEGER,
  policy_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL CHECK(policy_revision >= 1),
  policy_hash TEXT NOT NULL CHECK(length(policy_hash)=64),
  purpose TEXT NOT NULL,
  purpose_hash TEXT NOT NULL CHECK(length(purpose_hash)=64),
  data_classification TEXT NOT NULL,
  provider_order_json TEXT NOT NULL CHECK(json_valid(provider_order_json)),
  requested_model TEXT,
  privacy_selector_id TEXT,
  prompt_hash TEXT NOT NULL CHECK(length(prompt_hash)=64),
  context_hash TEXT NOT NULL CHECK(length(context_hash)=64),
  authority_hash TEXT NOT NULL CHECK(length(authority_hash)=64),
  input_chars INTEGER NOT NULL CHECK(input_chars BETWEEN 1 AND 30000),
  max_output_tokens INTEGER NOT NULL CHECK(max_output_tokens BETWEEN 16 AND 4096),
  state TEXT NOT NULL CHECK(state IN ('reserved','running','completed','failed','cancelled','interrupted')),
  selected_provider TEXT,
  selected_model TEXT,
  attempt_count INTEGER NOT NULL DEFAULT 0 CHECK(attempt_count BETWEEN 0 AND 8),
  result_hash TEXT,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  started_at_utc TEXT,
  completed_at_utc TEXT,
  FOREIGN KEY(policy_id) REFERENCES model_routing_policies(policy_id) ON DELETE RESTRICT,
  FOREIGN KEY(agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY(assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  FOREIGN KEY(privacy_selector_id) REFERENCES private_resource_selectors(selector_id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_model_inference_queue
  ON model_inference_requests(state,created_at_utc,request_id);
CREATE INDEX IF NOT EXISTS idx_model_inference_subject
  ON model_inference_requests(subject_type,subject_id,created_at_utc DESC,request_id DESC);
CREATE INDEX IF NOT EXISTS idx_model_inference_policy_window
  ON model_inference_requests(policy_id,created_at_utc,state,request_id);

CREATE TABLE IF NOT EXISTS model_inference_attempts (
  attempt_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL,
  attempt_sequence INTEGER NOT NULL CHECK(attempt_sequence BETWEEN 1 AND 8),
  provider_key TEXT NOT NULL CHECK(provider_key IN ('ollama','openrouter')),
  model_id TEXT NOT NULL CHECK(length(model_id) BETWEEN 1 AND 240),
  authority_hash TEXT NOT NULL CHECK(length(authority_hash)=64),
  decision_hash TEXT NOT NULL CHECK(length(decision_hash)=64),
  state TEXT NOT NULL CHECK(state IN ('running','succeeded','failed','cancelled','interrupted')),
  prompt_tokens INTEGER NOT NULL DEFAULT 0 CHECK(prompt_tokens >= 0),
  completion_tokens INTEGER NOT NULL DEFAULT 0 CHECK(completion_tokens >= 0),
  total_tokens INTEGER NOT NULL DEFAULT 0 CHECK(total_tokens >= 0),
  reported_cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK(reported_cost_microusd >= 0),
  output_hash TEXT,
  failure_code TEXT,
  started_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY(request_id) REFERENCES model_inference_requests(request_id) ON DELETE RESTRICT,
  UNIQUE(request_id,attempt_sequence)
);
CREATE INDEX IF NOT EXISTS idx_model_inference_attempt_request
  ON model_inference_attempts(request_id,attempt_sequence);

CREATE TABLE IF NOT EXISTS model_inference_private_results (
  request_id TEXT PRIMARY KEY,
  classification TEXT NOT NULL DEFAULT 'private' CHECK(classification='private'),
  output_text TEXT NOT NULL,
  output_bytes INTEGER NOT NULL CHECK(output_bytes BETWEEN 1 AND 4194304),
  output_hash TEXT NOT NULL CHECK(length(output_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY(request_id) REFERENCES model_inference_requests(request_id) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS model_inference_receipts (
  receipt_id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  subject_type TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  agent_id TEXT,
  assignment_id TEXT,
  wrapper_id TEXT,
  connection_id TEXT,
  policy_id TEXT NOT NULL,
  policy_revision INTEGER NOT NULL,
  purpose_hash TEXT NOT NULL CHECK(length(purpose_hash)=64),
  data_classification TEXT NOT NULL,
  provider_key TEXT,
  model_id TEXT,
  outcome TEXT NOT NULL CHECK(outcome IN ('completed','failed','cancelled','interrupted')),
  result_code TEXT NOT NULL CHECK(length(result_code) BETWEEN 1 AND 160),
  request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
  authority_hash TEXT NOT NULL CHECK(length(authority_hash)=64),
  prompt_hash TEXT NOT NULL CHECK(length(prompt_hash)=64),
  context_hash TEXT NOT NULL CHECK(length(context_hash)=64),
  result_hash TEXT,
  prompt_tokens INTEGER NOT NULL DEFAULT 0 CHECK(prompt_tokens >= 0),
  completion_tokens INTEGER NOT NULL DEFAULT 0 CHECK(completion_tokens >= 0),
  total_tokens INTEGER NOT NULL DEFAULT 0 CHECK(total_tokens >= 0),
  reported_cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK(reported_cost_microusd >= 0),
  receipt_hash TEXT NOT NULL CHECK(length(receipt_hash)=64),
  completed_at_utc TEXT NOT NULL,
  FOREIGN KEY(request_id) REFERENCES model_inference_requests(request_id) ON DELETE RESTRICT,
  FOREIGN KEY(policy_id) REFERENCES model_routing_policies(policy_id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_model_inference_receipt_subject
  ON model_inference_receipts(subject_type,subject_id,completed_at_utc DESC,receipt_id DESC);

CREATE TABLE IF NOT EXISTS model_inference_events (
  event_id TEXT PRIMARY KEY,
  request_id TEXT,
  policy_id TEXT,
  event_type TEXT NOT NULL CHECK(length(event_type) BETWEEN 1 AND 160),
  outcome TEXT NOT NULL CHECK(outcome IN ('success','warning','error','denied')),
  actor_type TEXT NOT NULL CHECK(actor_type IN ('local_user','agent','system','mcp_client')),
  actor_id TEXT NOT NULL CHECK(length(actor_id) BETWEEN 1 AND 180),
  detail_code TEXT NOT NULL CHECK(length(detail_code) BETWEEN 1 AND 160),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(metadata_json)),
  event_hash TEXT NOT NULL CHECK(length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY(request_id) REFERENCES model_inference_requests(request_id) ON DELETE RESTRICT,
  FOREIGN KEY(policy_id) REFERENCES model_routing_policies(policy_id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_model_inference_events
  ON model_inference_events(created_at_utc DESC,event_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_model_routing_policy_authority_immutable
BEFORE UPDATE ON model_routing_policies
WHEN NEW.subject_type IS NOT OLD.subject_type
  OR NEW.subject_id IS NOT OLD.subject_id
  OR NEW.agent_id IS NOT OLD.agent_id
  OR NEW.agent_revision IS NOT OLD.agent_revision
  OR NEW.assignment_id IS NOT OLD.assignment_id
  OR NEW.assignment_revision IS NOT OLD.assignment_revision
  OR NEW.wrapper_id IS NOT OLD.wrapper_id
  OR NEW.connection_id IS NOT OLD.connection_id
  OR NEW.connection_authority_revision IS NOT OLD.connection_authority_revision
  OR NEW.purpose IS NOT OLD.purpose
  OR NEW.purpose_hash IS NOT OLD.purpose_hash
  OR NEW.allowed_data_classes_json IS NOT OLD.allowed_data_classes_json
  OR NEW.provider_order_json IS NOT OLD.provider_order_json
  OR NEW.allowed_models_json IS NOT OLD.allowed_models_json
  OR NEW.allow_fallback IS NOT OLD.allow_fallback
  OR NEW.remote_context_mode IS NOT OLD.remote_context_mode
  OR NEW.require_zdr IS NOT OLD.require_zdr
  OR NEW.max_input_chars IS NOT OLD.max_input_chars
  OR NEW.max_output_tokens IS NOT OLD.max_output_tokens
  OR NEW.window_seconds IS NOT OLD.window_seconds
  OR NEW.max_requests IS NOT OLD.max_requests
  OR NEW.max_total_tokens IS NOT OLD.max_total_tokens
  OR NEW.max_spend_microusd IS NOT OLD.max_spend_microusd
  OR NEW.policy_revision IS NOT OLD.policy_revision
  OR NEW.policy_hash IS NOT OLD.policy_hash
  OR NEW.created_by_user_id IS NOT OLD.created_by_user_id
  OR NEW.not_before_utc IS NOT OLD.not_before_utc
  OR NEW.expires_at_utc IS NOT OLD.expires_at_utc
  OR NEW.created_at_utc IS NOT OLD.created_at_utc
BEGIN
  SELECT RAISE(ABORT,'model routing policy authority is immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_routing_policy_no_delete
BEFORE DELETE ON model_routing_policies
BEGIN
  SELECT RAISE(ABORT,'model routing policies are retained evidence');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_request_terminal_immutable
BEFORE UPDATE ON model_inference_requests
WHEN OLD.state IN ('completed','failed','cancelled','interrupted')
BEGIN
  SELECT RAISE(ABORT,'terminal model inference requests are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_request_no_delete
BEFORE DELETE ON model_inference_requests
BEGIN
  SELECT RAISE(ABORT,'model inference requests are retained evidence');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_attempt_terminal_immutable
BEFORE UPDATE ON model_inference_attempts
WHEN OLD.state IN ('succeeded','failed','cancelled','interrupted')
BEGIN
  SELECT RAISE(ABORT,'terminal model inference attempts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_attempt_no_delete
BEFORE DELETE ON model_inference_attempts
BEGIN
  SELECT RAISE(ABORT,'model inference attempts are retained evidence');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_private_result_no_update
BEFORE UPDATE ON model_inference_private_results
BEGIN
  SELECT RAISE(ABORT,'private model inference results are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_private_result_no_delete
BEFORE DELETE ON model_inference_private_results
BEGIN
  SELECT RAISE(ABORT,'private model inference results require archival');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_receipt_no_update
BEFORE UPDATE ON model_inference_receipts
BEGIN
  SELECT RAISE(ABORT,'model inference receipts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_receipt_no_delete
BEFORE DELETE ON model_inference_receipts
BEGIN
  SELECT RAISE(ABORT,'model inference receipts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_event_no_update
BEFORE UPDATE ON model_inference_events
BEGIN
  SELECT RAISE(ABORT,'model inference events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS trg_model_inference_event_no_delete
BEFORE DELETE ON model_inference_events
BEGIN
  SELECT RAISE(ABORT,'model inference events are append-only');
END;

INSERT OR IGNORE INTO model_routing_policies (
  policy_id,subject_type,subject_id,purpose,purpose_hash,
  allowed_data_classes_json,provider_order_json,allowed_models_json,
  allow_fallback,remote_context_mode,require_zdr,max_input_chars,max_output_tokens,
  window_seconds,max_requests,max_total_tokens,max_spend_microusd,
  policy_revision,policy_hash,state,created_by_user_id,reason,
  not_before_utc,expires_at_utc,created_at_utc,updated_at_utc
) VALUES (
  '00000000-0000-4000-8000-000000000020',
  'local_control_center','local_control_center','agent_workspace',
  lower(hex(sha3('agent_workspace',256))),
  '["public","safe_receipt","security_metadata","wrapper_owned","shared_approved","private_derived","private_source"]',
  '["ollama"]','[]',0,'deny',1,30000,1024,86400,10000,10000000,0,1,
  lower(hex(sha3('homeserver.phase20.local-control.default.v1',256))),
  'active','system','Default local-only inference policy. Remote providers require an explicit policy.',
  strftime('%Y-%m-%dT%H:%M:%fZ','now'),'2099-12-31T23:59:59.999Z',
  strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')
);

INSERT OR IGNORE INTO schema_migrations(migration_key)
VALUES('0028_authorized_model_routing');
