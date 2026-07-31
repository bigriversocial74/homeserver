from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    value = (ROOT / path).read_text(encoding="utf-8")
    if not value.strip():
        raise SystemExit(f"empty Phase 18 contract file: {path}")
    return value


migration = read("database/migrations/0026_supervised_action_orchestration.sql")
source = read("crates/homeserver-service/src/app/wrapper_orchestration.rs")
agents = read("crates/homeserver-service/src/app/wrapper_agents.rs")
runtime = read("crates/homeserver-service/src/app/wrapper_runtime.rs")
app = read("crates/homeserver-service/src/app.rs")
tauri = read("src-tauri/src/runtime.rs")
tauri_lib = read("src-tauri/src/lib.rs")
ui = read("src/agent-runtime-control-center.js")
workflow = read(".github/workflows/phase18-supervised-action-orchestration.yml")
docs = read("docs/phase-18-supervised-action-orchestration.md")

required_migration = [
    "action.supervised",
    "'external_side_effect','proposal','[\"action.propose\"]'",
    "agent_supervised_action_checkpoints",
    "agent_supervised_action_receipts",
    "agent_supervised_compensation_receipts",
    "agent_supervised_action_events",
    "trg_supervised_action_receipts_no_update",
    "trg_supervised_compensation_receipts_no_update",
    "trg_supervised_action_events_no_update",
    "0026_supervised_action_orchestration",
]
required_source = [
    "wrapper_runtime::create_plan",
    "wrapper_jobs::claim_jobs",
    "wrapper_jobs::complete_job",
    "wrapper_agents::create_proposal",
    "wrapper_agents::execute_proposal_as_orchestrator",
    "approval_payload_hash",
    "approval_connection_authority_revision",
    "supervised approval evidence changed",
    "approval_consumed_once: true",
    "phase16e_egress_required: true",
    "sensitive_runtime_bypass_allowed: false",
    "proposal_job_egress_enforced",
    "checkpoint_authority_denied",
    "runtime_plan_no_longer_active",
    "report.delete",
    "CANCEL JOB",
]
required_integration = [
    (agents, "pub(crate) fn create_proposal"),
    (agents, "pub(crate) fn execute_proposal_as_orchestrator"),
    (agents, "pub(crate) fn validate_safe_summary"),
    (runtime, "tool_key='action.supervised'"),
    (app, "mod wrapper_orchestration"),
    (app, "wrapper_orchestration::initialize"),
    (app, "wrapper_orchestration::run"),
    (app, "wrapper_orchestration::router"),
    (tauri, "/v1/action-orchestration"),
    (tauri, "/v1/action-orchestration/run-once"),
    (tauri, "/v1/action-orchestration/checkpoints/rollback"),
    (tauri_lib, "homeserver_action_orchestration"),
    (ui, "runtimeState.orchestration"),
    (ui, "renderSupervisedCheckpoints"),
    (ui, "data-runtime-rollback-checkpoint"),
    (workflow, "phase18_supervised_orchestration_contract"),
    (workflow, "cargo clippy -p microgifter-homeserver-service --all-targets -- -D warnings"),
    (docs, "Initial score: **5.6/10**"),
]

for item in required_migration:
    if item not in migration:
        raise SystemExit(f"missing Phase 18 migration contract: {item}")
for item in required_source:
    if item not in source:
        raise SystemExit(f"missing Phase 18 runtime contract: {item}")
for text, item in required_integration:
    if item not in text:
        raise SystemExit(f"missing Phase 18 integration contract: {item}")

for forbidden in [
    "std::process::Command",
    "tokio::process::Command",
    "powershell",
    "cmd.exe",
    "private_payloads_exposed: true",
    "private_results_exposed: true",
    "sensitive_runtime_bypass_allowed: true",
]:
    if forbidden in source:
        raise SystemExit(f"forbidden Phase 18 boundary: {forbidden}")

print("Phase 18 supervised action orchestration contract passed")


migration_contract = (ROOT / "database/migrations/0026_supervised_action_orchestration.sql").read_text(encoding="utf-8")
for immutable_trigger in (
    "trg_supervised_action_receipts_no_delete",
    "trg_supervised_compensation_receipts_no_delete",
    "trg_supervised_action_events_no_delete",
):
    if immutable_trigger not in migration_contract:
        raise SystemExit(f"missing immutable delete trigger: {immutable_trigger}")
