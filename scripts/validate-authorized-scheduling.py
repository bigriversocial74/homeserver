#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
migration = (ROOT / "database/migrations/0027_authorized_agent_scheduling.sql").read_text(encoding="utf-8")
source = (ROOT / "crates/homeserver-service/src/app/wrapper_scheduling.rs").read_text(encoding="utf-8")
runtime = (ROOT / "crates/homeserver-service/src/app/wrapper_runtime.rs").read_text(encoding="utf-8")
app = (ROOT / "crates/homeserver-service/src/app.rs").read_text(encoding="utf-8")
tauri_runtime = (ROOT / "src-tauri/src/runtime.rs").read_text(encoding="utf-8")
tauri_lib = (ROOT / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
frontend = (ROOT / "src/agent-runtime-control-center.js").read_text(encoding="utf-8")
package = (ROOT / "package.json").read_text(encoding="utf-8")

for table in (
    "agent_schedule_definitions",
    "agent_schedule_private_templates",
    "agent_schedule_event_inbox",
    "agent_schedule_cursors",
    "agent_schedule_runs",
    "agent_schedule_receipts",
    "agent_schedule_audit_events",
    "agent_scheduler_state",
):
    if f"CREATE TABLE IF NOT EXISTS {table}" not in migration:
        raise SystemExit(f"missing Phase 19 table: {table}")

for trigger in (
    "trg_agent_schedule_event_inbox_no_update",
    "trg_agent_schedule_event_inbox_no_delete",
    "trg_agent_schedule_receipts_no_update",
    "trg_agent_schedule_receipts_no_delete",
    "trg_agent_schedule_audit_no_update",
    "trg_agent_schedule_audit_no_delete",
):
    if trigger not in migration:
        raise SystemExit(f"missing Phase 19 immutable trigger: {trigger}")

for required in (
    "capture_authority",
    "revalidate_authority",
    "wrapper_runtime::create_plan",
    "reconcile_interrupted_runs",
    "runtime_plan_recovered",
    "misfire_policy",
    "overlap_policy",
    "debounce_seconds",
    "trigger_token",
    "agent_scheduler:",
    "private_templates_exposed: false",
    "private_event_payloads_exposed: false",
    "direct_execution_allowed: false",
    "phase17_runtime_required: true",
    "phase18_supervision_required: true",
    "retention requires archival",
    "safe event metadata contains a forbidden private field",
):
    if required not in source:
        raise SystemExit(f"missing Phase 19 boundary: {required}")

for forbidden in (
    "execute_adapter(",
    "execute_proposal_as_orchestrator(",
    "tokio::process::Command",
    "std::process::Command",
    "cmd.exe",
    "powershell",
    "webhook",
):
    if forbidden in source:
        raise SystemExit(f"forbidden Phase 19 execution path: {forbidden}")

for topic in (
    "wrapper.job.completed",
    "runtime.plan.completed",
    "supervised.action.completed",
    "cloud.sync.completed",
):
    if topic not in source:
        raise SystemExit(f"missing closed event topic: {topic}")

if "Serialize, Deserialize" not in runtime and "Deserialize, Serialize" not in runtime:
    raise SystemExit("runtime plan templates are not serializable")
for required in (
    "wrapper_scheduling::initialize",
    "wrapper_scheduling::run",
    ".merge(wrapper_scheduling::router",
    "agent_schedule_worker.abort",
):
    if required not in app:
        raise SystemExit(f"service integration missing: {required}")

for required in (
    "homeserver_agent_schedules",
    "homeserver_run_agent_scheduler_once",
    "homeserver_pause_agent_schedule",
    "homeserver_resume_agent_schedule",
    "homeserver_cancel_agent_schedule",
):
    if required not in tauri_runtime or required not in tauri_lib:
        raise SystemExit(f"trusted desktop bridge missing: {required}")

for required in (
    "scheduling: null",
    "renderSchedules",
    "homeserver_agent_schedules",
    "data-schedule-pause",
    "data-schedule-resume",
    "data-schedule-cancel",
    "Phase 17/18 plan creation",
):
    if required not in frontend:
        raise SystemExit(f"Control Center scheduling contract missing: {required}")

if "validate-authorized-scheduling.py" not in package:
    raise SystemExit("frontend validation does not retain the Phase 19 validator")

if "DELETE FROM agent_schedule_event_inbox" in source:
    raise SystemExit("safe event inbox has a deletion path")
if "DELETE FROM agent_schedule_receipts" in source:
    raise SystemExit("schedule receipts have a deletion path")
if "DELETE FROM agent_schedule_audit_events" in source:
    raise SystemExit("schedule audit events have a deletion path")

print("Phase 19 authorized scheduling validation passed")
