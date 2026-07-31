#!/usr/bin/env python3
from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATION = ROOT / "database/migrations/0025_authorized_agent_tool_runtime.sql"
SOURCE = ROOT / "crates/homeserver-service/src/app/wrapper_runtime.rs"
POLICY = ROOT / "crates/homeserver-service/src/app/wrapper_runtime_policy.rs"
APP = ROOT / "crates/homeserver-service/src/app.rs"
JOBS = ROOT / "crates/homeserver-service/src/app/wrapper_jobs.rs"
SUBMIT = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_submit.rs"
WORKERS = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_workers.rs"
COMPLETION = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_completion.rs"
AGENTS = ROOT / "crates/homeserver-service/src/app/wrapper_agents.rs"
PRIVACY = ROOT / "crates/homeserver-service/src/app/wrapper_privacy.rs"
DOC = ROOT / "docs/phase-17-authorized-agent-tool-runtime.md"
TAURI_RUNTIME = ROOT / "src-tauri/src/runtime.rs"
TAURI_LIB = ROOT / "src-tauri/src/lib.rs"
CONTROL_CENTER = ROOT / "src/agent-runtime-control-center.js"
INDEX = ROOT / "index.html"
PACKAGE = ROOT / "package.json"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"Phase 17 validation failed: {message}")


migration = MIGRATION.read_text(encoding="utf-8")
source = SOURCE.read_text(encoding="utf-8")
policy = POLICY.read_text(encoding="utf-8")
app = APP.read_text(encoding="utf-8")
jobs = JOBS.read_text(encoding="utf-8")
submit = SUBMIT.read_text(encoding="utf-8")
workers = WORKERS.read_text(encoding="utf-8")
completion = COMPLETION.read_text(encoding="utf-8")
agents = AGENTS.read_text(encoding="utf-8")
privacy = PRIVACY.read_text(encoding="utf-8")
doc = DOC.read_text(encoding="utf-8")
tauri_runtime = TAURI_RUNTIME.read_text(encoding="utf-8")
tauri_lib = TAURI_LIB.read_text(encoding="utf-8")
control_center = CONTROL_CENTER.read_text(encoding="utf-8")
index = INDEX.read_text(encoding="utf-8")
package = PACKAGE.read_text(encoding="utf-8")
job_sources = "\n".join([jobs, submit, workers, completion])

required_tables = [
    "agent_tool_catalog",
    "agent_runtime_plans",
    "agent_runtime_plan_steps",
    "agent_runtime_attempts",
    "agent_runtime_receipts",
    "agent_runtime_events",
    "agent_runtime_audit_records",
    "agent_runtime_state",
]
for table in required_tables:
    require(f"CREATE TABLE IF NOT EXISTS {table}" in migration, f"missing table {table}")

for phrase in [
    "agent runtime receipts are immutable",
    "agent runtime events are append-only",
    "FOREIGN KEY (job_id) REFERENCES wrapper_jobs(job_id)",
    "FOREIGN KEY (agent_id) REFERENCES homeserver_agents(agent_id)",
    "FOREIGN KEY (job_receipt_id) REFERENCES wrapper_job_execution_receipts(receipt_id)",
    "'read_only','reversible','external_side_effect','high_risk'",
    "0025_authorized_agent_tool_runtime",
]:
    require(phrase in migration, f"missing migration boundary {phrase}")

for tool in [
    "wrapper.status.read",
    "receipt.read",
    "audit.record",
    "result.compose",
]:
    require(tool in migration, f"missing runtime tool {tool}")

for integration in [
    '#[path = "app/wrapper_runtime.rs"]',
    '#[path = "app/wrapper_runtime_policy.rs"]',
    "wrapper_runtime::initialize(&connection)?;",
    "wrapper_runtime::run(state.clone(), shutdown.clone())",
    ".merge(wrapper_runtime::router(state.clone()))",
    ".merge(wrapper_runtime_policy::router(state.clone()))",
    "wrapper_runtime::maintain_history(&connection)",
]:
    require(integration in app, f"missing service integration {integration}")

for retained in [
    "pub fn submit_job",
    "pub fn claim_jobs",
    "pub fn start_job",
    "pub fn complete_job",
    "pub fn fail_job",
    "pub fn cancel_job",
]:
    require(retained in job_sources, f"missing retained job operation {retained}")

for boundary in [
    "wrapper_jobs::submit_job",
    "wrapper_jobs::claim_jobs",
    "wrapper_jobs::start_job",
    "wrapper_jobs::complete_job",
    "wrapper_jobs::fail_job",
    "wrapper_jobs::cancel_job",
    "wrapper_agents::agent_job_authority_is_current_tx",
    "runtime tools requiring proposals are not executable in Phase 17",
    "Phase 17 runtime policies must be approval-free and low-risk",
    "runtime policy execution limit reached",
    "runtime plan step cannot execute before its predecessors",
    "runtime job execution limit exceeds the tool catalog limit",
    "step.job.approval_id.is_none() && step.job.plan_hash.is_none()",
    "step.job.available_at_utc = if index == 0",
    "UPDATE wrapper_jobs SET available_at_utc=?1",
    "runtime_receipt_missing",
    "direct_tool_bypass_allowed: false",
    "phase16e_egress_required: true",
    "private_inputs_exposed: false",
    "private_results_exposed: false",
    "CANCEL PLAN {plan_id}",
    "runtime_receipt_hash",
]:
    require(boundary in source, f"missing runtime boundary {boundary}")

for boundary in [
    "/v1/agent-runtime/policies/create",
    "SELECT adapter_key,risk_class,approval_requirement,state FROM agent_tool_catalog",
    "INSERT INTO agent_execution_policies",
    "Phase 17 runtime policies must be approval-free and low-risk",
    "proposal-gated tools are not executable in the Phase 17 runtime",
    "approval-free runtime policy requires scoped autonomy",
    "policy_replaced",
    "reconcile_authority",
]:
    require(boundary in policy, f"missing runtime policy boundary {boundary}")

request_start = policy.index("pub struct CreateRuntimePolicyRequest")
request_end = policy.index("pub struct RuntimePolicyResponse", request_start)
request_contract = policy[request_start:request_end]
require("tool_adapter" not in request_contract, "caller can supply a runtime adapter")
require("risk_class" not in request_contract, "caller can supply a runtime risk class")

require(
    "wrapper_privacy::evaluate_egress_tx" in completion,
    "runtime completion does not retain Phase 16E egress enforcement",
)
require(
    "agent_job_authority_is_current_tx" in agents,
    "runtime cannot revalidate Phase 16D agent authority",
)
require(
    "agent_action_approvals" not in source,
    "low-risk runtime incorrectly depends on the separate Phase 16D approval table",
)
require(
    "agent_emergency_stops" in agents,
    "retained agent emergency-stop authority is missing",
)
require(
    "evaluate_egress_tx" in privacy,
    "retained privacy egress evaluator is missing",
)


for bridge in [
    "homeserver_agent_runtime",
    "homeserver_agent_authority",
    "homeserver_run_agent_runtime_once",
    "homeserver_cancel_agent_runtime_plan",
    "/v1/agent-runtime",
    "/v1/agents",
]:
    require(bridge in tauri_runtime, f"missing trusted runtime bridge {bridge}")
    require(bridge in tauri_lib or bridge.startswith("/v1/"), f"runtime bridge is not registered {bridge}")

require(
    'const LOCAL_CONTROL_CENTER_ACTOR: &str = "local_control_center"' in tauri_runtime,
    "runtime cancellation actor is not pinned in the trusted bridge",
)
require(
    '"actor_user_id": LOCAL_CONTROL_CENTER_ACTOR' in tauri_runtime,
    "trusted runtime cancellation does not use the pinned actor",
)
require(
    "actorUserId" not in control_center,
    "runtime UI can supply its own cancellation audit actor",
)
require(
    "struct ExecutionAuthorityRow" in source
    and "execution_authority_from_row" in source,
    "runtime execution authority is not represented by a typed row",
)

for surface in [
    "data-agent-runtime-route",
    "Agent Runtime",
    "homeserver:rendered",
    "homeserver_agent_runtime",
    "homeserver_agent_authority",
    "homeserver_run_agent_runtime_once",
    "homeserver_cancel_agent_runtime_plan",
    "Phase 16 authority boundary is intact",
    "Private inputs",
    "Private results",
    "Direct tool bypass",
    "Phase 16E egress",
    "Runtime Plans",
    "Approvals & Stops",
    "Immutable Runtime Receipts",
]:
    require(surface in control_center, f"missing Control Center runtime surface {surface}")

require("MutationObserver" not in control_center, "runtime UI reintroduced an observer network")
require("fetch(" not in control_center, "runtime UI bypasses the trusted Tauri local client")
require("/src/agent-runtime-control-center.js" in index, "runtime Control Center module is not loaded")
require("node --check src/agent-runtime-control-center.js" in package, "runtime UI is not in frontend validation")

for forbidden in [
    '"shell.execute"',
    '"process.spawn"',
    '"filesystem.raw"',
    '"credential.read"',
    '"tools.all"',
    '"agent.execute_any"',
    "std::process::Command",
    "tokio::process::Command",
]:
    require(forbidden not in source, f"unsafe runtime primitive present: {forbidden}")
    require(forbidden not in policy, f"unsafe runtime policy primitive present: {forbidden}")

snapshot_start = source.index("fn snapshot(state")
snapshot_end = source.index("fn fail_interrupted_attempts", snapshot_start)
snapshot = source[snapshot_start:snapshot_end]
for private_table in [
    "wrapper_job_inputs",
    "wrapper_job_private_results",
    "agent_action_private_payloads",
    "agent_action_private_results",
]:
    require(private_table not in snapshot, f"runtime snapshot reads private table {private_table}")

reconcile_start = source.index("fn reconcile(connection")
reconcile_end = source.index("fn refresh_plan_state", reconcile_start)
reconcile_block = source[reconcile_start:reconcile_end]
require("service_restarted" not in reconcile_block, "snapshot reconciliation can fail live attempts")
require("WHEN j.state='waiting' THEN 'queued'" in reconcile_block, "waiting jobs are not mapped safely")
require("WHEN j.state='dead_letter' THEN 'failed'" in reconcile_block, "dead-letter jobs are not mapped safely")
require("fn fail_interrupted_attempts" in source, "startup interruption recovery is missing")

conn = sqlite3.connect(":memory:")
conn.execute("PRAGMA foreign_keys=ON")
conn.executescript(
    """
    CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);
    CREATE TABLE wrapper_identities (wrapper_id TEXT PRIMARY KEY);
    CREATE TABLE wrapper_connections (
      connection_id TEXT PRIMARY KEY,
      wrapper_id TEXT NOT NULL,
      FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id)
    );
    CREATE TABLE wrapper_job_workers (
      worker_id TEXT PRIMARY KEY,
      worker_kind TEXT NOT NULL
    );
    CREATE TABLE homeserver_agents (agent_id TEXT PRIMARY KEY);
    CREATE TABLE wrapper_jobs (job_id TEXT PRIMARY KEY);
    CREATE TABLE wrapper_job_execution_receipts (receipt_id TEXT PRIMARY KEY);
    """
)
conn.executescript(migration)
tables = {
    row[0]
    for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")
}
require(set(required_tables).issubset(tables), "SQLite did not create all Phase 17 tables")

conn.executescript(
    """
    INSERT INTO wrapper_identities VALUES ('11111111-1111-4111-8111-111111111111');
    INSERT INTO wrapper_connections VALUES (
      '22222222-2222-4222-8222-222222222222',
      '11111111-1111-4111-8111-111111111111'
    );
    INSERT INTO wrapper_job_workers VALUES (
      '33333333-3333-4333-8333-333333333333','tool'
    );
    INSERT INTO homeserver_agents VALUES ('44444444-4444-4444-8444-444444444444');
    INSERT INTO wrapper_jobs VALUES ('55555555-5555-4555-8555-555555555555');
    INSERT INTO agent_runtime_state (
      singleton_id,worker_id,state,created_at_utc,updated_at_utc
    ) VALUES (
      1,'33333333-3333-4333-8333-333333333333','active',
      '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
    );
    INSERT INTO agent_runtime_plans (
      plan_id,agent_id,requested_by_user_id,title,objective,state,step_count,
      correlation_id,plan_hash,expires_at_utc,created_at_utc,updated_at_utc
    ) VALUES (
      '66666666-6666-4666-8666-666666666666',
      '44444444-4444-4444-8444-444444444444','owner','Plan','Objective',
      'running',1,'77777777-7777-4777-8777-777777777777',
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      '2099-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z',
      '2026-01-01T00:00:00.000Z'
    );
    INSERT INTO agent_runtime_plan_steps (
      step_id,plan_id,sequence_number,job_id,tool_key,adapter_key,action_type,
      state,idempotency_key,argument_hash,created_at_utc,updated_at_utc
    ) VALUES (
      '88888888-8888-4888-8888-888888888888',
      '66666666-6666-4666-8666-666666666666',1,
      '55555555-5555-4555-8555-555555555555','audit.record','audit.record',
      'audit.record','running','phase17-test-key',
      'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
    );
    INSERT INTO agent_runtime_events (
      event_id,plan_id,step_id,job_id,agent_id,event_type,outcome,actor_type,
      actor_id,detail_code,metadata_json,event_hash,created_at_utc
    ) VALUES (
      '99999999-9999-4999-8999-999999999999',
      '66666666-6666-4666-8666-666666666666',
      '88888888-8888-4888-8888-888888888888',
      '55555555-5555-4555-8555-555555555555',
      '44444444-4444-4444-8444-444444444444',
      'agent.runtime_step_started','success','worker','runtime','test','{}',
      'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
      '2026-01-01T00:00:00.000Z'
    );
    INSERT INTO agent_runtime_receipts (
      receipt_id,plan_id,step_id,job_id,agent_id,wrapper_id,connection_id,
      tool_key,adapter_key,outcome,result_code,runtime_receipt_hash,
      completed_at_utc,created_at_utc
    ) VALUES (
      'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
      '66666666-6666-4666-8666-666666666666',
      '88888888-8888-4888-8888-888888888888',
      '55555555-5555-4555-8555-555555555555',
      '44444444-4444-4444-8444-444444444444',
      '11111111-1111-4111-8111-111111111111',
      '22222222-2222-4222-8222-222222222222',
      'audit.record','audit.record','failed','test_failure',
      'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
      '2026-01-01T00:00:00.000Z','2026-01-01T00:00:00.000Z'
    );
    """
)
for statement, label in [
    (
        "UPDATE agent_runtime_events SET detail_code='changed' "
        "WHERE event_id='99999999-9999-4999-8999-999999999999'",
        "runtime event update",
    ),
    (
        "UPDATE agent_runtime_receipts SET result_code='changed' "
        "WHERE receipt_id='aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa'",
        "runtime receipt update",
    ),
]:
    try:
        conn.execute(statement)
    except sqlite3.DatabaseError:
        pass
    else:
        raise SystemExit(f"Phase 17 validation failed: {label} was not blocked")

for phrase in [
    "Initial current-state audit: **5.8/10**",
    "Authority chain",
    "Tool registry",
    "Runtime plan lifecycle",
    "Private result boundary",
    "Failure, cancellation, and restart behavior",
    "Immutable evidence",
    "Control Center visibility",
    "10/10 certification gates",
    "explicit merge approval",
]:
    require(phrase in doc, f"missing documentation section {phrase}")

print("Phase 17 authorized agent tool runtime validation passed.")
