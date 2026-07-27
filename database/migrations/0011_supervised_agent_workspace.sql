-- Phase 5B supervised Agent Workspace, approvals, receipts, and World Mission contracts.

CREATE TABLE IF NOT EXISTS agent_goals (
  goal_id TEXT PRIMARY KEY,
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 160),
  description TEXT NOT NULL DEFAULT '',
  target_metric TEXT,
  target_value TEXT,
  target_date TEXT,
  connection_ids_json TEXT NOT NULL DEFAULT '[]',
  dataset_keys_json TEXT NOT NULL DEFAULT '[]',
  constraints_json TEXT NOT NULL DEFAULT '{}',
  allowed_actions_json TEXT NOT NULL DEFAULT '[]',
  approval_policy TEXT NOT NULL DEFAULT 'always' CHECK (approval_policy IN ('always','read_only','disabled')),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','paused','completed','archived')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_agent_goals_state_updated
  ON agent_goals (state, updated_at_utc DESC, goal_id DESC);

CREATE TABLE IF NOT EXISTS agent_threads (
  thread_id TEXT PRIMARY KEY,
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 160),
  state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active','archived')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);

CREATE INDEX IF NOT EXISTS idx_agent_threads_updated
  ON agent_threads (state, updated_at_utc DESC, thread_id DESC);

CREATE TABLE IF NOT EXISTS agent_messages (
  message_id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('user','assistant','system')),
  mode TEXT NOT NULL CHECK (mode IN ('ask','analyze','plan','dispatch','execute')),
  content TEXT NOT NULL CHECK (length(content) BETWEEN 1 AND 20000),
  context_json TEXT NOT NULL DEFAULT '{}',
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (thread_id) REFERENCES agent_threads(thread_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_agent_messages_thread_created
  ON agent_messages (thread_id, created_at_utc, message_id);

CREATE TABLE IF NOT EXISTS agent_plans (
  plan_id TEXT PRIMARY KEY,
  thread_id TEXT,
  goal_id TEXT,
  requested_by_type TEXT NOT NULL CHECK (requested_by_type IN ('local_user','mcp_client','system')),
  requested_by_id TEXT NOT NULL,
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  rationale TEXT NOT NULL CHECK (length(rationale) BETWEEN 1 AND 4000),
  action_type TEXT NOT NULL,
  arguments_json TEXT NOT NULL DEFAULT '{}',
  connection_id TEXT,
  dataset_keys_json TEXT NOT NULL DEFAULT '[]',
  risk_level TEXT NOT NULL CHECK (risk_level IN ('low','medium','high')),
  state TEXT NOT NULL CHECK (state IN ('draft','awaiting_approval','approved','executing','completed','failed','rejected','cancelled','expired')),
  plan_hash TEXT NOT NULL UNIQUE,
  fresh_state_token TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  completed_at_utc TEXT,
  FOREIGN KEY (thread_id) REFERENCES agent_threads(thread_id) ON DELETE SET NULL,
  FOREIGN KEY (goal_id) REFERENCES agent_goals(goal_id) ON DELETE SET NULL,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_plans_state_updated
  ON agent_plans (state, updated_at_utc DESC, plan_id DESC);
CREATE INDEX IF NOT EXISTS idx_agent_plans_requester
  ON agent_plans (requested_by_type, requested_by_id, created_at_utc DESC);

CREATE TABLE IF NOT EXISTS agent_plan_steps (
  step_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  step_index INTEGER NOT NULL CHECK (step_index >= 0),
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  action_type TEXT NOT NULL,
  arguments_json TEXT NOT NULL DEFAULT '{}',
  state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','executing','completed','failed','cancelled')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (plan_id) REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
  UNIQUE (plan_id, step_index)
);

CREATE TABLE IF NOT EXISTS agent_approval_requests (
  approval_request_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL UNIQUE,
  plan_hash TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('pending','approved','rejected','consumed','expired','cancelled')),
  risk_summary TEXT NOT NULL CHECK (length(risk_summary) BETWEEN 1 AND 2000),
  requested_at_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  decided_at_utc TEXT,
  decision_reason TEXT,
  FOREIGN KEY (plan_id) REFERENCES agent_plans(plan_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_agent_approval_requests_state
  ON agent_approval_requests (state, expires_at_utc, requested_at_utc DESC);

CREATE TABLE IF NOT EXISTS agent_approvals (
  approval_id TEXT PRIMARY KEY,
  approval_request_id TEXT NOT NULL UNIQUE,
  plan_id TEXT NOT NULL UNIQUE,
  plan_hash TEXT NOT NULL,
  approved_by TEXT NOT NULL,
  approved_at_utc TEXT NOT NULL,
  expires_at_utc TEXT NOT NULL,
  consumed_at_utc TEXT,
  FOREIGN KEY (approval_request_id) REFERENCES agent_approval_requests(approval_request_id) ON DELETE CASCADE,
  FOREIGN KEY (plan_id) REFERENCES agent_plans(plan_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_action_idempotency (
  idempotency_key TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('executing','completed','failed')),
  receipt_id TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (plan_id) REFERENCES agent_plans(plan_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_execution_receipts (
  receipt_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL UNIQUE,
  approval_id TEXT NOT NULL,
  plan_hash TEXT NOT NULL,
  action_type TEXT NOT NULL,
  connection_id TEXT,
  idempotency_key TEXT NOT NULL UNIQUE,
  state TEXT NOT NULL CHECK (state IN ('completed','failed')),
  result_code TEXT NOT NULL,
  result_summary TEXT NOT NULL CHECK (length(result_summary) BETWEEN 1 AND 4000),
  result_json TEXT NOT NULL DEFAULT '{}',
  started_at_utc TEXT NOT NULL,
  completed_at_utc TEXT NOT NULL,
  FOREIGN KEY (plan_id) REFERENCES agent_plans(plan_id) ON DELETE CASCADE,
  FOREIGN KEY (approval_id) REFERENCES agent_approvals(approval_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_execution_receipts_completed
  ON agent_execution_receipts (completed_at_utc DESC, receipt_id DESC);

CREATE TABLE IF NOT EXISTS agent_reports (
  report_id TEXT PRIMARY KEY,
  plan_id TEXT,
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  content_markdown TEXT NOT NULL CHECK (length(content_markdown) BETWEEN 1 AND 30000),
  connection_ids_json TEXT NOT NULL DEFAULT '[]',
  dataset_keys_json TEXT NOT NULL DEFAULT '[]',
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (plan_id) REFERENCES agent_plans(plan_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_reports_created
  ON agent_reports (created_at_utc DESC, report_id DESC);

CREATE TABLE IF NOT EXISTS world_missions (
  mission_id TEXT PRIMARY KEY,
  thread_id TEXT,
  goal_id TEXT,
  connection_id TEXT,
  world_agent_id TEXT NOT NULL CHECK (length(world_agent_id) BETWEEN 1 AND 160),
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  objective TEXT NOT NULL CHECK (length(objective) BETWEEN 1 AND 4000),
  allowed_operations_json TEXT NOT NULL DEFAULT '[]',
  prohibited_operations_json TEXT NOT NULL DEFAULT '[]',
  limits_json TEXT NOT NULL DEFAULT '{}',
  disclosure_policy_json TEXT NOT NULL DEFAULT '{}',
  state TEXT NOT NULL DEFAULT 'draft' CHECK (state IN ('draft','awaiting_approval','ready_for_dispatch','dispatched','active','waiting','recommendation_ready','completed','cancelled','expired','failed')),
  expires_at_utc TEXT NOT NULL,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (thread_id) REFERENCES agent_threads(thread_id) ON DELETE SET NULL,
  FOREIGN KEY (goal_id) REFERENCES agent_goals(goal_id) ON DELETE SET NULL,
  FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_world_missions_state_updated
  ON world_missions (state, updated_at_utc DESC, mission_id DESC);

CREATE TABLE IF NOT EXISTS world_tasks (
  task_id TEXT PRIMARY KEY,
  mission_id TEXT NOT NULL,
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  description TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL DEFAULT 'draft' CHECK (state IN ('draft','queued','active','completed','failed','cancelled')),
  due_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (mission_id) REFERENCES world_missions(mission_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS world_conversations (
  conversation_id TEXT PRIMARY KEY,
  mission_id TEXT,
  external_conversation_id TEXT,
  participant_type TEXT NOT NULL CHECK (participant_type IN ('avatar','merchant_agent','campaign_agent','community_agent','human')),
  participant_id TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('discovered','introduced','engaged','qualified','needs_identified','information_exchanged','recommendation_prepared','action_proposed','awaiting_approval','committed','completed','closed','follow_up_scheduled','reopened','expired')),
  summary TEXT NOT NULL DEFAULT '',
  closure_reason TEXT,
  visibility TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private','group','merchant','campaign','public_world','transaction')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  updated_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  closed_at_utc TEXT,
  FOREIGN KEY (mission_id) REFERENCES world_missions(mission_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_world_conversations_state_updated
  ON world_conversations (state, updated_at_utc DESC, conversation_id DESC);

CREATE TABLE IF NOT EXISTS world_conversation_commitments (
  commitment_id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  owner_type TEXT NOT NULL CHECK (owner_type IN ('world_agent','merchant_agent','avatar','human','system')),
  owner_id TEXT NOT NULL,
  action TEXT NOT NULL CHECK (length(action) BETWEEN 1 AND 1000),
  state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open','completed','cancelled','overdue')),
  due_at_utc TEXT,
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  completed_at_utc TEXT,
  FOREIGN KEY (conversation_id) REFERENCES world_conversations(conversation_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS world_follow_ups (
  follow_up_id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  scheduled_for_utc TEXT NOT NULL,
  purpose TEXT NOT NULL CHECK (length(purpose) BETWEEN 1 AND 1000),
  state TEXT NOT NULL DEFAULT 'scheduled' CHECK (state IN ('scheduled','completed','cancelled','expired')),
  created_at_utc TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  completed_at_utc TEXT,
  FOREIGN KEY (conversation_id) REFERENCES world_conversations(conversation_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_world_follow_ups_due
  ON world_follow_ups (state, scheduled_for_utc, follow_up_id);

CREATE TABLE IF NOT EXISTS world_mission_events (
  event_id TEXT PRIMARY KEY,
  mission_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error')),
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (mission_id) REFERENCES world_missions(mission_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_world_mission_events_recent
  ON world_mission_events (mission_id, created_at_utc DESC, event_id DESC);

CREATE TABLE IF NOT EXISTS world_receipts (
  receipt_id TEXT PRIMARY KEY,
  mission_id TEXT NOT NULL,
  event_type TEXT NOT NULL,
  disposition TEXT NOT NULL CHECK (disposition IN ('accepted','rejected','review','completed','failed')),
  summary TEXT NOT NULL CHECK (length(summary) BETWEEN 1 AND 4000),
  metadata_json TEXT NOT NULL DEFAULT '{}',
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (mission_id) REFERENCES world_missions(mission_id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0011_supervised_agent_workspace');
