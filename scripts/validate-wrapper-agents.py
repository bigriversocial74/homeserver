#!/usr/bin/env python3
from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATION = ROOT / "database/migrations/0023_wrapper_agents_and_action_approvals.sql"
SOURCE = ROOT / "crates/homeserver-service/src/app/wrapper_agents.rs"
APP = ROOT / "crates/homeserver-service/src/app.rs"
JOBS = ROOT / "crates/homeserver-service/src/app/wrapper_jobs.rs"
SUBMIT = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_submit.rs"
RECONCILE = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_reconcile.rs"
DOC = ROOT / "docs/phase-16d-agent-lifecycle-action-policy.md"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"Phase 16D validation failed: {message}")


migration = MIGRATION.read_text(encoding="utf-8")
source = SOURCE.read_text(encoding="utf-8")
app = APP.read_text(encoding="utf-8")
jobs = JOBS.read_text(encoding="utf-8")
submit = SUBMIT.read_text(encoding="utf-8")
reconcile = RECONCILE.read_text(encoding="utf-8")
doc = DOC.read_text(encoding="utf-8")

required_tables = [
    "homeserver_agents",
    "wrapper_agent_assignments",
    "agent_capability_bindings",
    "agent_execution_policies",
    "agent_job_bindings",
    "agent_action_proposals",
    "agent_action_private_payloads",
    "agent_action_approvals",
    "agent_action_attempts",
    "agent_action_private_results",
    "agent_action_receipts",
    "agent_lifecycle_events",
    "agent_emergency_stops",
]
for table in required_tables:
    require(f"CREATE TABLE IF NOT EXISTS {table}" in migration, f"missing table {table}")

for phrase in [
    "agent action receipts are immutable",
    "agent lifecycle events are append-only",
    "autonomy_level BETWEEN 0 AND 4",
    "'read_only','reversible','external_side_effect','high_risk'",
    "classification='private'",
    "connection_authority_revision",
    "agent_revision",
    "assignment_revision",
    "policy_revision",
]:
    require(phrase in migration, f"missing migration boundary {phrase}")

for integration in [
    '#[path = "app/wrapper_agents.rs"]',
    "wrapper_agents::initialize(&connection)?;",
    ".merge(wrapper_agents::router(state.clone()))",
    "wrapper_agents::maintain_history(&connection)",
]:
    require(integration in app, f"missing app integration {integration}")

require("use super::wrapper_agents;" in jobs, "wrapper jobs do not import agent authority")
for phrase in [
    "validate_agent_job_submission",
    "bind_agent_job_tx",
]:
    require(phrase in submit, f"agent job submission is not bound: {phrase}")
require(
    "agent_job_authority_is_current_tx" in reconcile,
    "job reconciliation does not enforce agent revision and stop authority",
)

for phrase in [
    "pairing_implies_agent_authority: false",
    "private_payloads_exposed: false",
    "private_results_exposed: false",
    "sensitive actions always require approval",
    "suggest-only agents cannot receive executable adapters",
    "approval plan hash mismatch",
    "approval payload hash mismatch",
    "emergency_stop_active_tx",
    "proposal-only policy cannot execute",
    "receipt_hash",
    "agent.action_executed",
]:
    require(phrase in source, f"missing runtime boundary {phrase}")

for forbidden in [
    '"shell.execute"',
    '"process.spawn"',
    '"filesystem.raw"',
    '"credential.read"',
    '"tools.all"',
    '"agent.execute_any"',
]:
    require(forbidden not in source, f"unsafe adapter or capability present: {forbidden}")

snapshot_start = source.index("fn snapshot_with_connection")
snapshot_end = source.index("fn read_agents", snapshot_start)
snapshot = source[snapshot_start:snapshot_end]
require("agent_action_private_payloads" not in snapshot, "snapshot reads private action payloads")
require("agent_action_private_results" not in snapshot, "snapshot reads private action results")

conn = sqlite3.connect(":memory:")
conn.execute("PRAGMA foreign_keys=ON")
conn.executescript(
    """
    CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);
    CREATE TABLE wrapper_identities (wrapper_id TEXT PRIMARY KEY);
    CREATE TABLE wrapper_connections (
      connection_id TEXT PRIMARY KEY,
      wrapper_id TEXT NOT NULL,
      lifecycle_state TEXT NOT NULL,
      grant_revision INTEGER NOT NULL DEFAULT 0,
      FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id)
    );
    CREATE TABLE wrapper_capability_catalog (capability_key TEXT PRIMARY KEY);
    CREATE TABLE wrapper_capability_grants (
      grant_id TEXT PRIMARY KEY,
      wrapper_id TEXT NOT NULL,
      connection_id TEXT NOT NULL,
      capability_key TEXT NOT NULL,
      grant_revision INTEGER NOT NULL,
      FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id),
      FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id),
      FOREIGN KEY(capability_key) REFERENCES wrapper_capability_catalog(capability_key)
    );
    CREATE TABLE wrapper_jobs (
      job_id TEXT PRIMARY KEY,
      connection_id TEXT NOT NULL,
      FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id)
    );
    CREATE TABLE agent_reports (
      report_id TEXT PRIMARY KEY,
      plan_id TEXT,
      title TEXT NOT NULL,
      content_markdown TEXT NOT NULL,
      connection_ids_json TEXT NOT NULL,
      dataset_keys_json TEXT NOT NULL,
      created_at_utc TEXT NOT NULL
    );
    """
)
conn.executescript(migration)
tables = {
    row[0]
    for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
}
require(set(required_tables).issubset(tables), "SQLite did not create all Phase 16D tables")

conn.executescript(
    """
    INSERT INTO wrapper_identities VALUES ('11111111-1111-4111-8111-111111111111');
    INSERT INTO wrapper_connections VALUES (
      '22222222-2222-4222-8222-222222222222',
      '11111111-1111-4111-8111-111111111111','active',1
    );
    INSERT INTO wrapper_capability_catalog VALUES ('action.propose');
    INSERT INTO wrapper_capability_grants VALUES (
      '33333333-3333-4333-8333-333333333333',
      '11111111-1111-4111-8111-111111111111',
      '22222222-2222-4222-8222-222222222222',
      'action.propose',1
    );
    INSERT INTO wrapper_jobs VALUES (
      '44444444-4444-4444-8444-444444444444',
      '22222222-2222-4222-8222-222222222222'
    );
    INSERT INTO homeserver_agents (
      agent_id,owner_user_id,display_name,purpose,state,autonomy_level,revision,
      allowed_job_types_json,model_restrictions_json,tool_restrictions_json,
      expires_at_utc,created_at_utc,updated_at_utc
    ) VALUES (
      '55555555-5555-4555-8555-555555555555','owner','Agent','test','active',2,1,
      '["action.proposal"]','{}','{}','2099-01-01T00:00:00.000Z',
      '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
    );
    INSERT INTO agent_lifecycle_events (
      event_id,agent_id,event_type,outcome,actor_type,actor_id,detail_code,
      metadata_json,event_hash,created_at_utc
    ) VALUES (
      '66666666-6666-4666-8666-666666666666',
      '55555555-5555-4555-8555-555555555555',
      'agent.created','success','local_user','owner','test','{}',
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      '2026-01-01T00:00:00.000Z'
    );
    """
)
try:
    conn.execute(
        "UPDATE agent_lifecycle_events SET detail_code='changed' "
        "WHERE event_id='66666666-6666-4666-8666-666666666666'"
    )
except sqlite3.DatabaseError:
    pass
else:
    raise SystemExit("Phase 16D validation failed: lifecycle event update was not blocked")

for phrase in [
    "Initial current-state score: **5.4/10**",
    "Authority model",
    "Autonomy levels",
    "Sensitive action flow",
    "Private-data boundary",
    "Emergency stop",
    "10/10 certification gates",
]:
    require(phrase in doc, f"missing documentation section {phrase}")

print("Phase 16D agent lifecycle and action authority validation passed.")
