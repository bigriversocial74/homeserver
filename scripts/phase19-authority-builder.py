from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

runtime_path = ROOT / "crates/homeserver-service/src/app/wrapper_runtime.rs"
runtime = runtime_path.read_text(encoding="utf-8")
old_signature = "fn cancel_plan(state: &AppState, request: RuntimePlanReferenceRequest) -> Result<()> {"
new_signature = '''fn cancel_plan(state: &AppState, request: RuntimePlanReferenceRequest) -> Result<()> {
    cancel_plan_with_actor(state, request, "local_user")
}

pub(crate) fn cancel_plan_as_system(
    state: &AppState,
    request: RuntimePlanReferenceRequest,
) -> Result<()> {
    cancel_plan_with_actor(state, request, "system")
}

fn cancel_plan_with_actor(
    state: &AppState,
    request: RuntimePlanReferenceRequest,
    actor_type: &'static str,
) -> Result<()> {'''
if "pub(crate) fn cancel_plan_as_system" not in runtime:
    if old_signature not in runtime:
        raise RuntimeError("runtime cancellation authority anchor is missing")
    runtime = runtime.replace(old_signature, new_signature, 1)
section_start = runtime.index("fn cancel_plan_with_actor(")
section_end = runtime.index("\nfn process_cycle(", section_start)
section = runtime[section_start:section_end]
if 'actor_type,' not in section:
    marker = '            actor_type: "local_user",\n'
    if marker not in section:
        raise RuntimeError("runtime cancellation event actor anchor is missing")
    section = section.replace(marker, "            actor_type,\n", 1)
runtime = runtime[:section_start] + section + runtime[section_end:]
runtime_path.write_text(runtime, encoding="utf-8")

scheduling_path = ROOT / "crates/homeserver-service/src/app/wrapper_scheduling.rs"
scheduling = scheduling_path.read_text(encoding="utf-8")
scheduling = scheduling.replace(
    "wrapper_runtime::cancel_plan(\n",
    "wrapper_runtime::cancel_plan_as_system(\n",
    1,
)

capture_anchor = '''    )?;
    let mut bindings = Vec::with_capacity(steps.len());
'''
capture_patch = '''    )?;
    ensure_no_emergency_stop(connection, agent_id, &row.3, connection_id, &now)?;
    let mut bindings = Vec::with_capacity(steps.len());
'''
if "ensure_no_emergency_stop(connection, agent_id, &row.3" not in scheduling:
    if capture_anchor not in scheduling:
        raise RuntimeError("schedule authority capture emergency-stop anchor is missing")
    scheduling = scheduling.replace(capture_anchor, capture_patch, 1)

revalidate_anchor = '''    ensure!(
        authority_count == 1,
        "schedule agent, assignment, or connection authority changed"
    );
    for binding in &document.bindings {
'''
revalidate_patch = '''    ensure!(
        authority_count == 1,
        "schedule agent, assignment, or connection authority changed"
    );
    ensure_no_emergency_stop(
        connection,
        &schedule.agent_id,
        &schedule.wrapper_id,
        &schedule.connection_id,
        &now,
    )?;
    for binding in &document.bindings {
'''
if "&schedule.connection_id,\n        &now,\n    )?;" not in scheduling:
    if revalidate_anchor not in scheduling:
        raise RuntimeError("schedule authority revalidation emergency-stop anchor is missing")
    scheduling = scheduling.replace(revalidate_anchor, revalidate_patch, 1)

function_anchor = "fn pause_schedule(state: &AppState, request: ScheduleReferenceRequest) -> Result<()> {"
emergency_function = '''fn ensure_no_emergency_stop(
    connection: &Connection,
    agent_id: &str,
    wrapper_id: &str,
    connection_id: &str,
    now: &str,
) -> Result<()> {
    let active_stops: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_emergency_stops WHERE state='active' AND (expires_at_utc IS NULL OR expires_at_utc>?4) AND (scope_type='global' OR (scope_type='agent' AND agent_id=?1) OR (scope_type='wrapper' AND wrapper_id=?2) OR (scope_type='connection' AND connection_id=?3))",
        params![agent_id, wrapper_id, connection_id, now],
        |row| row.get(0),
    )?;
    ensure!(
        active_stops == 0,
        "schedule authority is blocked by an active emergency stop"
    );
    Ok(())
}

'''
if "fn ensure_no_emergency_stop(" not in scheduling:
    if function_anchor not in scheduling:
        raise RuntimeError("emergency-stop helper insertion anchor is missing")
    scheduling = scheduling.replace(function_anchor, emergency_function + function_anchor, 1)
scheduling_path.write_text(scheduling, encoding="utf-8")

contract_path = ROOT / "crates/homeserver-service/tests/phase19_authorized_scheduling_contract.rs"
contract = contract_path.read_text(encoding="utf-8")
anchor = '''        "state='queued'",
    ] {
'''
replacement = '''        "state='queued'",
        "schedule authority is blocked by an active emergency stop",
        "wrapper_runtime::cancel_plan_as_system",
    ] {
'''
if replacement not in contract:
    if anchor not in contract:
        raise RuntimeError("Phase 19 authority test anchor is missing")
    contract = contract.replace(anchor, replacement, 1)
contract_path.write_text(contract, encoding="utf-8")

validator_path = ROOT / "scripts/validate-authorized-scheduling.py"
validator = validator_path.read_text(encoding="utf-8")
extension = '''

for required in (
    "schedule authority is blocked by an active emergency stop",
    "wrapper_runtime::cancel_plan_as_system",
):
    if required not in source:
        raise SystemExit(f"missing Phase 19 emergency or cancellation authority: {required}")
if "pub(crate) fn cancel_plan_as_system" not in runtime:
    raise SystemExit("runtime does not expose a system-only cancellation path")
'''
if "runtime does not expose a system-only cancellation path" not in validator:
    validator += extension
validator_path.write_text(validator, encoding="utf-8")

print("Phase 19 emergency-stop and system-cancellation patches applied")
