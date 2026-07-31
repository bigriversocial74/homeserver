-- Phase 19: authority-bound scheduling and safe event triggers.
-- Schedules never execute tools or approvals directly. Every trigger revalidates
-- the captured Phase 16 authority and creates a fresh Phase 17/18 runtime plan.

CREATE TABLE IF NOT EXISTS agent_schedule_definitions (
  schedule_id TEXT PRIMARY KEY,
  agent_id TEXT NOT NULL,
  agent_revision INTEGER NOT NULL CHECK (agent_revision >= 1),
  assignment_id TEXT NOT NULL,
  assignment_revision INTEGER NOT NULL CHECK (assignment_revision >= 1),
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  connection_authority_revision INTEGER NOT NULL CHECK (connection_authority_revision >= 0),
  created_by_user_id TEXT NOT NULL CHECK (length(created_by_user_id) BETWEEN 1 AND 160),
  title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 180),
  description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 2000),
  state TEXT NOT NULL CHECK (state IN (
    'active','paused','completed','failed','cancelled','expired'
  )),
  trigger_kind TEXT NOT NULL CHECK (trigger_kind IN ('one_time','interval','event')),
  run_at_utc TEXT,
  interval_seconds INTEGER CHECK (interval_seconds BETWEEN 60 AND 2592000),
  event_topic TEXT,
  event_source_id TEXT,
  misfire_policy TEXT NOT NULL CHECK (misfire_policy IN ('skip','fire_once','fail')),
  overlap_policy TEXT NOT NULL CHECK (overlap_policy IN ('skip','queue_one')),
  debounce_seconds INTEGER NOT NULL DEFAULT 0 CHECK (debounce_seconds BETWEEN 0 AND 86400),
  max_runs INTEGER NOT NULL DEFAULT 1 CHECK (max_runs BETWEEN 1 AND 100000),
  run_count INTEGER NOT NULL DEFAULT 0 CHECK (run_count BETWEEN 0 AND max_runs),
  template_hash TEXT NOT NULL CHECK (length(template_hash)=64),
  authority_snapshot_json TEXT NOT NULL CHECK (json_valid(authority_snapshot_json)),
  authority_hash TEXT NOT NULL CHECK (length(authority_hash)=64),
  next_fire_at_utc TEXT,
  last_fired_at_utc TEXT,
  expires_at_utc TEXT NOT NULL,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL,
  completed_at_utc TEXT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT,
  CHECK (
    (trigger_kind='one_time' AND run_at_utc IS NOT NULL AND interval_seconds IS NULL AND event_topic IS NULL)
    OR (trigger_kind='interval' AND interval_seconds IS NOT NULL AND event_topic IS NULL)
    OR (trigger_kind='event' AND event_topic IS NOT NULL AND run_at_utc IS NULL AND interval_seconds IS NULL)
  )
);

CREATE INDEX IF NOT EXISTS idx_agent_schedules_due
  ON agent_schedule_definitions (state, trigger_kind, next_fire_at_utc, schedule_id);
CREATE INDEX IF NOT EXISTS idx_agent_schedules_event
  ON agent_schedule_definitions (state, event_topic, event_source_id, schedule_id);
CREATE INDEX IF NOT EXISTS idx_agent_schedules_agent
  ON agent_schedule_definitions (agent_id, state, updated_at_utc DESC, schedule_id);

CREATE TABLE IF NOT EXISTS agent_schedule_private_templates (
  schedule_id TEXT PRIMARY KEY,
  classification TEXT NOT NULL DEFAULT 'private' CHECK (classification='private'),
  template_json TEXT NOT NULL CHECK (json_valid(template_json)),
  template_bytes INTEGER NOT NULL CHECK (template_bytes BETWEEN 2 AND 1048576),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (schedule_id) REFERENCES agent_schedule_definitions(schedule_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_schedule_event_inbox (
  event_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  event_id TEXT NOT NULL UNIQUE,
  topic TEXT NOT NULL CHECK (length(topic) BETWEEN 1 AND 120),
  source_type TEXT NOT NULL CHECK (source_type IN (
    'wrapper','runtime','orchestration','cloud','system'
  )),
  source_id TEXT NOT NULL CHECK (length(source_id) BETWEEN 1 AND 180),
  event_key TEXT NOT NULL UNIQUE CHECK (length(event_key) BETWEEN 1 AND 240),
  safe_metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(safe_metadata_json)),
  payload_hash TEXT NOT NULL CHECK (length(payload_hash)=64),
  occurred_at_utc TEXT NOT NULL,
  received_at_utc TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_agent_schedule_event_topic
  ON agent_schedule_event_inbox (topic, event_sequence);
CREATE INDEX IF NOT EXISTS idx_agent_schedule_event_source
  ON agent_schedule_event_inbox (source_type, source_id, event_sequence);

CREATE TABLE IF NOT EXISTS agent_schedule_cursors (
  schedule_id TEXT PRIMARY KEY,
  last_event_sequence INTEGER NOT NULL DEFAULT 0 CHECK (last_event_sequence >= 0),
  updated_at_utc TEXT NOT NULL,
  FOREIGN KEY (schedule_id) REFERENCES agent_schedule_definitions(schedule_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS agent_schedule_runs (
  run_id TEXT PRIMARY KEY,
  schedule_id TEXT NOT NULL,
  trigger_kind TEXT NOT NULL CHECK (trigger_kind IN ('one_time','interval','event')),
  trigger_token TEXT NOT NULL UNIQUE CHECK (length(trigger_token)=64),
  event_id TEXT,
  scheduled_for_utc TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN (
    'queued','creating_plan','completed','skipped','failed','interrupted'
  )),
  authority_hash TEXT NOT NULL CHECK (length(authority_hash)=64),
  template_hash TEXT NOT NULL CHECK (length(template_hash)=64),
  plan_id TEXT,
  plan_hash TEXT,
  outcome TEXT,
  result_code TEXT,
  failure_code TEXT,
  created_at_utc TEXT NOT NULL,
  started_at_utc TEXT,
  completed_at_utc TEXT,
  FOREIGN KEY (schedule_id) REFERENCES agent_schedule_definitions(schedule_id) ON DELETE RESTRICT,
  CHECK ((plan_id IS NULL AND plan_hash IS NULL) OR (plan_id IS NOT NULL AND length(plan_hash)=64))
);

CREATE INDEX IF NOT EXISTS idx_agent_schedule_runs_queue
  ON agent_schedule_runs (state, created_at_utc, run_id);
CREATE INDEX IF NOT EXISTS idx_agent_schedule_runs_schedule
  ON agent_schedule_runs (schedule_id, created_at_utc DESC, run_id DESC);

CREATE TABLE IF NOT EXISTS agent_schedule_receipts (
  receipt_id TEXT PRIMARY KEY,
  schedule_id TEXT NOT NULL,
  run_id TEXT NOT NULL UNIQUE,
  agent_id TEXT NOT NULL,
  assignment_id TEXT NOT NULL,
  wrapper_id TEXT NOT NULL,
  connection_id TEXT NOT NULL,
  trigger_kind TEXT NOT NULL,
  trigger_token TEXT NOT NULL CHECK (length(trigger_token)=64),
  event_id TEXT,
  outcome TEXT NOT NULL CHECK (outcome IN ('completed','skipped','failed','interrupted')),
  result_code TEXT NOT NULL,
  authority_hash TEXT NOT NULL CHECK (length(authority_hash)=64),
  template_hash TEXT NOT NULL CHECK (length(template_hash)=64),
  plan_id TEXT,
  plan_hash TEXT,
  receipt_hash TEXT NOT NULL CHECK (length(receipt_hash)=64),
  completed_at_utc TEXT NOT NULL,
  FOREIGN KEY (schedule_id) REFERENCES agent_schedule_definitions(schedule_id) ON DELETE RESTRICT,
  FOREIGN KEY (run_id) REFERENCES agent_schedule_runs(run_id) ON DELETE RESTRICT,
  FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id) ON DELETE RESTRICT,
  FOREIGN KEY (assignment_id) REFERENCES wrapper_agent_assignments(assignment_id) ON DELETE RESTRICT,
  FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id) ON DELETE RESTRICT,
  FOREIGN KEY (connection_id) REFERENCES wrapper_connections(connection_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_schedule_receipts_schedule
  ON agent_schedule_receipts (schedule_id, completed_at_utc DESC, receipt_id DESC);

CREATE TABLE IF NOT EXISTS agent_schedule_audit_events (
  audit_event_id TEXT PRIMARY KEY,
  schedule_id TEXT,
  run_id TEXT,
  event_type TEXT NOT NULL CHECK (length(event_type) BETWEEN 1 AND 160),
  outcome TEXT NOT NULL CHECK (outcome IN ('success','warning','error','denied')),
  actor_type TEXT NOT NULL CHECK (actor_type IN ('local_user','scheduler','system','event_source')),
  actor_id TEXT NOT NULL CHECK (length(actor_id) BETWEEN 1 AND 180),
  detail_code TEXT NOT NULL CHECK (length(detail_code) BETWEEN 1 AND 160),
  metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json)),
  event_hash TEXT NOT NULL CHECK (length(event_hash)=64),
  created_at_utc TEXT NOT NULL,
  FOREIGN KEY (schedule_id) REFERENCES agent_schedule_definitions(schedule_id) ON DELETE RESTRICT,
  FOREIGN KEY (run_id) REFERENCES agent_schedule_runs(run_id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_agent_schedule_audit
  ON agent_schedule_audit_events (schedule_id, created_at_utc DESC, audit_event_id DESC);

CREATE TABLE IF NOT EXISTS agent_scheduler_state (
  singleton_id INTEGER PRIMARY KEY CHECK (singleton_id=1),
  state TEXT NOT NULL CHECK (state IN ('active','degraded')),
  scheduler_revision INTEGER NOT NULL DEFAULT 1 CHECK (scheduler_revision >= 1),
  last_cycle_at_utc TEXT,
  last_error_code TEXT,
  created_at_utc TEXT NOT NULL,
  updated_at_utc TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_definitions_immutable_fields
BEFORE UPDATE ON agent_schedule_definitions
WHEN NEW.agent_id IS NOT OLD.agent_id
  OR NEW.agent_revision IS NOT OLD.agent_revision
  OR NEW.assignment_id IS NOT OLD.assignment_id
  OR NEW.assignment_revision IS NOT OLD.assignment_revision
  OR NEW.wrapper_id IS NOT OLD.wrapper_id
  OR NEW.connection_id IS NOT OLD.connection_id
  OR NEW.connection_authority_revision IS NOT OLD.connection_authority_revision
  OR NEW.created_by_user_id IS NOT OLD.created_by_user_id
  OR NEW.title IS NOT OLD.title
  OR NEW.description IS NOT OLD.description
  OR NEW.trigger_kind IS NOT OLD.trigger_kind
  OR NEW.run_at_utc IS NOT OLD.run_at_utc
  OR NEW.interval_seconds IS NOT OLD.interval_seconds
  OR NEW.event_topic IS NOT OLD.event_topic
  OR NEW.event_source_id IS NOT OLD.event_source_id
  OR NEW.misfire_policy IS NOT OLD.misfire_policy
  OR NEW.overlap_policy IS NOT OLD.overlap_policy
  OR NEW.debounce_seconds IS NOT OLD.debounce_seconds
  OR NEW.max_runs IS NOT OLD.max_runs
  OR NEW.template_hash IS NOT OLD.template_hash
  OR NEW.authority_snapshot_json IS NOT OLD.authority_snapshot_json
  OR NEW.authority_hash IS NOT OLD.authority_hash
  OR NEW.expires_at_utc IS NOT OLD.expires_at_utc
  OR NEW.created_at_utc IS NOT OLD.created_at_utc
BEGIN
  SELECT RAISE(ABORT, 'agent schedule authority and trigger fields are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_definitions_no_delete
BEFORE DELETE ON agent_schedule_definitions
BEGIN
  SELECT RAISE(ABORT, 'agent schedule definitions are retained evidence');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_private_templates_no_update
BEFORE UPDATE ON agent_schedule_private_templates
BEGIN
  SELECT RAISE(ABORT, 'agent schedule private templates are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_private_templates_no_delete
BEFORE DELETE ON agent_schedule_private_templates
BEGIN
  SELECT RAISE(ABORT, 'agent schedule private templates are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_runs_terminal_no_update
BEFORE UPDATE ON agent_schedule_runs
WHEN OLD.state IN ('completed','skipped','failed','interrupted')
BEGIN
  SELECT RAISE(ABORT, 'terminal agent schedule runs are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_runs_no_delete
BEFORE DELETE ON agent_schedule_runs
BEGIN
  SELECT RAISE(ABORT, 'agent schedule runs are retained evidence');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_event_inbox_no_update
BEFORE UPDATE ON agent_schedule_event_inbox
BEGIN
  SELECT RAISE(ABORT, 'agent schedule event inbox is append-only');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_event_inbox_no_delete
BEFORE DELETE ON agent_schedule_event_inbox
BEGIN
  SELECT RAISE(ABORT, 'agent schedule event inbox is append-only');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_receipts_no_update
BEFORE UPDATE ON agent_schedule_receipts
BEGIN
  SELECT RAISE(ABORT, 'agent schedule receipts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_receipts_no_delete
BEFORE DELETE ON agent_schedule_receipts
BEGIN
  SELECT RAISE(ABORT, 'agent schedule receipts are immutable');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_audit_no_update
BEFORE UPDATE ON agent_schedule_audit_events
BEGIN
  SELECT RAISE(ABORT, 'agent schedule audit events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_audit_no_delete
BEFORE DELETE ON agent_schedule_audit_events
BEGIN
  SELECT RAISE(ABORT, 'agent schedule audit events are append-only');
END;

INSERT OR IGNORE INTO agent_scheduler_state (
  singleton_id,state,scheduler_revision,created_at_utc,updated_at_utc
) VALUES (
  1,'active',1,
  strftime('%Y-%m-%dT%H:%M:%fZ','now'),
  strftime('%Y-%m-%dT%H:%M:%fZ','now')
);

INSERT OR IGNORE INTO schema_migrations (migration_key)
VALUES ('0027_authorized_agent_scheduling');
