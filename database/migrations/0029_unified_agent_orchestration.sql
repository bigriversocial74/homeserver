-- Phase 22: unified HomeServer Agent orchestration.
-- HomeServer remains the authority for local context, engagement, external MCP
-- credentials, tool discovery, and invocation evidence.

CREATE TABLE IF NOT EXISTS agent_site_integrations (
  connection_id TEXT PRIMARY KEY,
  provider_key TEXT NOT NULL CHECK(length(provider_key) BETWEEN 2 AND 80),
  resource_uri TEXT NOT NULL CHECK(length(resource_uri) BETWEEN 12 AND 500),
  authorization_server TEXT NOT NULL CHECK(length(authorization_server) BETWEEN 12 AND 500),
  client_id TEXT NOT NULL CHECK(length(client_id) BETWEEN 1 AND 240),
  redirect_uri TEXT NOT NULL CHECK(length(redirect_uri) BETWEEN 12 AND 500),
  scopes_json TEXT NOT NULL CHECK(json_valid(scopes_json)),
  credential_key TEXT NOT NULL UNIQUE CHECK(length(credential_key) BETWEEN 16 AND 300),
  state TEXT NOT NULL CHECK(state IN ('configured','authorization_pending','connected','degraded','revoked')),
  token_expires_at_utc TEXT,
  pending_state_hash TEXT CHECK(pending_state_hash IS NULL OR length(pending_state_hash)=64),
  pending_expires_at_utc TEXT,
  tool_catalog_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(tool_catalog_json)),
  last_tool_sync_utc TEXT,
  last_success_utc TEXT,
  last_error TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY(connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_agent_site_integration_state
  ON agent_site_integrations(state,provider_key,updated_at_utc DESC,connection_id);

CREATE TABLE IF NOT EXISTS agent_engagement_state (
  singleton_id INTEGER PRIMARY KEY CHECK(singleton_id=1),
  onboarding_started_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  onboarding_completed_at_utc TEXT,
  last_user_prompt_at_utc TEXT,
  last_agent_prompt_key TEXT,
  dismissed_prompt_keys_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(dismissed_prompt_keys_json)),
  engagement_revision INTEGER NOT NULL DEFAULT 1 CHECK(engagement_revision >= 1),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
INSERT OR IGNORE INTO agent_engagement_state(singleton_id) VALUES(1);

CREATE TABLE IF NOT EXISTS agent_context_receipts (
  receipt_id TEXT PRIMARY KEY,
  thread_id TEXT,
  prompt_hash TEXT NOT NULL CHECK(length(prompt_hash)=64),
  source_keys_json TEXT NOT NULL CHECK(json_valid(source_keys_json)),
  knowledge_hit_count INTEGER NOT NULL DEFAULT 0 CHECK(knowledge_hit_count >= 0),
  operational_record_count INTEGER NOT NULL DEFAULT 0 CHECK(operational_record_count >= 0),
  mcp_tool_names_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(mcp_tool_names_json)),
  context_hash TEXT NOT NULL CHECK(length(context_hash)=64),
  inference_state TEXT NOT NULL CHECK(inference_state IN ('not_started','completed','unavailable','failed')),
  failure_code TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY(thread_id) REFERENCES agent_threads(thread_id) ON DELETE SET NULL
);
CREATE INDEX IF NOT EXISTS idx_agent_context_receipts_created
  ON agent_context_receipts(created_at_utc DESC,receipt_id DESC);

CREATE TABLE IF NOT EXISTS agent_mcp_invocation_receipts (
  receipt_id TEXT PRIMARY KEY,
  connection_id TEXT NOT NULL,
  tool_name TEXT NOT NULL CHECK(length(tool_name) BETWEEN 2 AND 240),
  operation_class TEXT NOT NULL CHECK(operation_class IN ('read','draft','action_request','unknown')),
  request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
  result_hash TEXT,
  outcome TEXT NOT NULL CHECK(outcome IN ('completed','denied','failed')),
  result_code TEXT NOT NULL CHECK(length(result_code) BETWEEN 2 AND 160),
  duration_ms INTEGER NOT NULL DEFAULT 0 CHECK(duration_ms >= 0),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY(connection_id) REFERENCES agent_site_integrations(connection_id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_agent_mcp_receipts_connection
  ON agent_mcp_invocation_receipts(connection_id,created_at_utc DESC,receipt_id DESC);

CREATE TRIGGER IF NOT EXISTS trg_agent_context_receipts_no_update
BEFORE UPDATE ON agent_context_receipts
BEGIN
  SELECT RAISE(ABORT,'Agent context receipts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS trg_agent_context_receipts_no_delete
BEFORE DELETE ON agent_context_receipts
BEGIN
  SELECT RAISE(ABORT,'Agent context receipts require archival');
END;
CREATE TRIGGER IF NOT EXISTS trg_agent_mcp_receipts_no_update
BEFORE UPDATE ON agent_mcp_invocation_receipts
BEGIN
  SELECT RAISE(ABORT,'Agent MCP receipts are immutable');
END;
CREATE TRIGGER IF NOT EXISTS trg_agent_mcp_receipts_no_delete
BEFORE DELETE ON agent_mcp_invocation_receipts
BEGIN
  SELECT RAISE(ABORT,'Agent MCP receipts require archival');
END;

INSERT OR IGNORE INTO schema_migrations(migration_key)
VALUES('0029_unified_agent_orchestration');
