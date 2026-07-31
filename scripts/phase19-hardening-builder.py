from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[1]


def patch(path: str, old: str, new: str, count: int = 1) -> None:
    target = ROOT / path
    text = target.read_text(encoding="utf-8")
    if new in text:
        return
    actual = text.count(old)
    if actual < count:
        raise RuntimeError(f"{path}: missing hardening anchor ({actual} < {count}): {old[:120]!r}")
    target.write_text(text.replace(old, new, count), encoding="utf-8")


package_path = ROOT / "package.json"
package = json.loads(package_path.read_text(encoding="utf-8"))
check = package["scripts"]["check:frontend"]
if "validate-authorized-scheduling.py" not in check:
    anchor = "validate-supervised-orchestration.py"
    if anchor not in check:
        raise RuntimeError("package validator-list anchor is missing")
    check = check.replace(anchor, f"{anchor} validate-authorized-scheduling.py", 1)
package["scripts"]["check:frontend"] = check
package_path.write_text(json.dumps(package, indent=2) + "\n", encoding="utf-8")

source_path = ROOT / "crates/homeserver-service/src/app/wrapper_scheduling.rs"
source = source_path.read_text(encoding="utf-8")

cursor_old = '''    if trigger_kind == "event" {
        transaction.execute(
            "INSERT INTO agent_schedule_cursors (schedule_id,last_event_sequence,updated_at_utc) VALUES (?1,0,?2)",
            params![schedule_id, now_text],
        )?;
    }
'''
cursor_new = '''    if trigger_kind == "event" {
        let current_event_sequence: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(event_sequence),0) FROM agent_schedule_event_inbox",
            [],
            |row| row.get(0),
        )?;
        transaction.execute(
            "INSERT INTO agent_schedule_cursors (schedule_id,last_event_sequence,updated_at_utc) VALUES (?1,?2,?3)",
            params![schedule_id, current_event_sequence, now_text],
        )?;
    }
'''
if cursor_new not in source:
    if cursor_old not in source:
        raise RuntimeError("event cursor creation anchor is missing")
    source = source.replace(cursor_old, cursor_new, 1)

misfire_old = '''        create_terminal_run(
            connection,
            &schedule,
            None,
            &scheduled_for,
            "skipped",
            "misfire_skipped",
            None,
        )?;
        advance_time_schedule(connection, &schedule, now)?;
        return Ok(());
'''
misfire_new = '''        create_terminal_run(
            connection,
            &schedule,
            None,
            &scheduled_for,
            "skipped",
            "misfire_skipped",
            None,
        )?;
        if schedule.trigger_kind == "one_time" {
            complete_schedule(connection, &schedule.schedule_id, "misfire_skipped")?;
        } else {
            advance_time_schedule(connection, &schedule, now)?;
        }
        return Ok(());
'''
if misfire_new not in source:
    if misfire_old not in source:
        raise RuntimeError("one-time misfire completion anchor is missing")
    source = source.replace(misfire_old, misfire_new, 1)

overlap_old = '''    if handle_overlap(connection, &schedule, None, &scheduled_for)? {
        advance_time_schedule(connection, &schedule, now)?;
        return Ok(());
    }
'''
overlap_new = '''    if handle_overlap(connection, &schedule, None, &scheduled_for)? {
        if schedule.trigger_kind == "one_time" {
            complete_schedule(connection, &schedule.schedule_id, "overlap_skipped")?;
        } else {
            advance_time_schedule(connection, &schedule, now)?;
        }
        return Ok(());
    }
'''
if overlap_new not in source:
    if overlap_old not in source:
        raise RuntimeError("one-time overlap completion anchor is missing")
    source = source.replace(overlap_old, overlap_new, 1)

overlap_body_old = '''    let active: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_schedule_runs WHERE schedule_id=?1 AND state IN ('queued','creating_plan')",
        params![schedule.schedule_id],
        |row| row.get(0),
    )?;
    if active == 0 {
        return Ok(false);
    }
    let code = if schedule.overlap_policy == "queue_one" {
        "overlap_coalesced"
    } else {
        "overlap_skipped"
    };
    create_terminal_run_tx(
        transaction,
        schedule,
        event_id,
        scheduled_for,
        "skipped",
        code,
        None,
    )?;
    Ok(true)
'''
overlap_body_new = '''    let creating: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_schedule_runs WHERE schedule_id=?1 AND state='creating_plan'",
        params![schedule.schedule_id],
        |row| row.get(0),
    )?;
    let queued: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_schedule_runs WHERE schedule_id=?1 AND state='queued'",
        params![schedule.schedule_id],
        |row| row.get(0),
    )?;
    if creating == 0 && queued == 0 {
        return Ok(false);
    }
    if schedule.overlap_policy == "queue_one" && creating > 0 && queued == 0 {
        return Ok(false);
    }
    let code = if schedule.overlap_policy == "queue_one" {
        "overlap_coalesced"
    } else {
        "overlap_skipped"
    };
    create_terminal_run_tx(
        transaction,
        schedule,
        event_id,
        scheduled_for,
        "skipped",
        code,
        None,
    )?;
    Ok(true)
'''
if overlap_body_new not in source:
    if overlap_body_old not in source:
        raise RuntimeError("queue-one overlap anchor is missing")
    source = source.replace(overlap_body_old, overlap_body_new, 1)

source_type_anchor = '''    let source_id = bounded_text(&request.source_id, 1, 180, "event source ID")?;
'''
source_type_patch = '''    ensure!(
        source_type == expected_source_type(&topic)?,
        "safe event source type does not match its topic"
    );
    let source_id = bounded_text(&request.source_id, 1, 180, "event source ID")?;
'''
if source_type_patch not in source:
    if source_type_anchor not in source:
        raise RuntimeError("event source-type anchor is missing")
    source = source.replace(source_type_anchor, source_type_patch, 1)
source = source.replace(
    "    ensure_safe_metadata(&request.safe_metadata)?;",
    "    ensure_safe_metadata(&topic, &request.safe_metadata)?;",
    1,
)

metadata_start = source.index("fn ensure_safe_metadata(")
metadata_end = source.index("\nfn validate_choice(", metadata_start)
metadata_replacement = '''fn expected_source_type(topic: &str) -> Result<&'static str> {
    match topic {
        "wrapper.job.completed" => Ok("wrapper"),
        "runtime.plan.completed" => Ok("runtime"),
        "supervised.action.completed" => Ok("orchestration"),
        "cloud.sync.completed" => Ok("cloud"),
        _ => bail!("safe event topic has no source contract"),
    }
}

fn allowed_safe_event_fields(topic: &str) -> Result<&'static [&'static str]> {
    match topic {
        "wrapper.job.completed" => Ok(&[
            "job_id",
            "connection_id",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        "runtime.plan.completed" => Ok(&[
            "plan_id",
            "agent_id",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        "supervised.action.completed" => Ok(&[
            "checkpoint_id",
            "proposal_id",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        "cloud.sync.completed" => Ok(&[
            "connection_id",
            "operation_type",
            "outcome",
            "result_code",
            "receipt_hash",
        ]),
        _ => bail!("safe event topic has no metadata contract"),
    }
}

fn ensure_safe_metadata(topic: &str, value: &Value) -> Result<()> {
    let allowed = allowed_safe_event_fields(topic)?;
    let object = value
        .as_object()
        .context("safe event metadata must be an object")?;
    ensure!(
        object.len() <= allowed.len(),
        "safe event metadata contains too many fields"
    );
    for (key, child) in object {
        let normalized = key.to_ascii_lowercase();
        ensure!(
            allowed.iter().any(|candidate| *candidate == normalized),
            "safe event metadata field is not allowed for this topic"
        );
        ensure!(
            !FORBIDDEN_EVENT_KEYS
                .iter()
                .any(|forbidden| normalized.contains(forbidden)),
            "safe event metadata contains a forbidden private field"
        );
        match child {
            Value::String(text) => ensure!(
                text.chars().count() <= 500,
                "safe event metadata string is too long"
            ),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
            Value::Object(_) | Value::Array(_) => {
                bail!("safe event metadata values must be primitive")
            }
        }
    }
    Ok(())
}
'''
source = source[:metadata_start] + metadata_replacement + source[metadata_end:]

plan_creation_old = '''        let plan_id = wrapper_runtime::create_plan(state, plan)?;
        let connection = state.connection()?;
        let plan_hash: String = connection.query_row(
'''
plan_creation_new = '''        let plan_id = wrapper_runtime::create_plan(state, plan)?;
        let post_creation_authority = (|| -> Result<()> {
            let connection = state.connection()?;
            let current_schedule = read_schedule(&connection, &schedule.schedule_id)?;
            ensure!(
                current_schedule.state == "active",
                "schedule changed during runtime plan creation"
            );
            revalidate_authority(&connection, &current_schedule)
        })();
        if let Err(error) = post_creation_authority {
            wrapper_runtime::cancel_plan(
                state,
                wrapper_runtime::RuntimePlanReferenceRequest {
                    plan_id: plan_id.clone(),
                    actor_user_id: SCHEDULER_ACTOR.to_owned(),
                    confirmation: format!("CANCEL PLAN {plan_id}"),
                    reason: "schedule authority changed during plan creation".to_owned(),
                },
            )?;
            return Err(error).context("schedule authority changed during plan creation");
        }
        let connection = state.connection()?;
        let plan_hash: String = connection.query_row(
'''
if plan_creation_new not in source:
    if plan_creation_old not in source:
        raise RuntimeError("post-plan authority anchor is missing")
    source = source.replace(plan_creation_old, plan_creation_new, 1)

source_path.write_text(source, encoding="utf-8")

patch(
    "crates/homeserver-service/src/app/wrapper_runtime.rs",
    "fn cancel_plan(state: &AppState, request: RuntimePlanReferenceRequest) -> Result<()> {",
    "pub(crate) fn cancel_plan(state: &AppState, request: RuntimePlanReferenceRequest) -> Result<()> {",
    1,
)

contract_path = ROOT / "crates/homeserver-service/tests/phase19_authorized_scheduling_contract.rs"
contract = contract_path.read_text(encoding="utf-8")
contract_anchor = '''        "retention requires archival",
    ] {
'''
contract_patch = '''        "retention requires archival",
        "SELECT COALESCE(MAX(event_sequence),0)",
        "safe event source type does not match its topic",
        "safe event metadata field is not allowed for this topic",
        "safe event metadata values must be primitive",
        "schedule changed during runtime plan creation",
        "wrapper_runtime::cancel_plan",
        "state='creating_plan'",
        "state='queued'",
    ] {
'''
if contract_patch not in contract:
    if contract_anchor not in contract:
        raise RuntimeError("Phase 19 contract hardening anchor is missing")
    contract = contract.replace(contract_anchor, contract_patch, 1)
contract_path.write_text(contract, encoding="utf-8")

validator_path = ROOT / "scripts/validate-authorized-scheduling.py"
validator = validator_path.read_text(encoding="utf-8")
validator_extension = '''

for required in (
    "SELECT COALESCE(MAX(event_sequence),0)",
    "safe event source type does not match its topic",
    "safe event metadata field is not allowed for this topic",
    "safe event metadata values must be primitive",
    "schedule changed during runtime plan creation",
    "wrapper_runtime::cancel_plan",
):
    if required not in source:
        raise SystemExit(f"missing Phase 19 hostile-review repair: {required}")
if "VALUES (?1,0,?2)" in source:
    raise SystemExit("event schedules replay pre-creation events")
queue_one_block = source[source.index("fn handle_overlap_tx("):source.index("fn create_queued_run(")]
if "creating > 0 && queued == 0" not in queue_one_block:
    raise SystemExit("queue_one does not preserve one deferred run")
if source.count("complete_schedule(connection, &schedule.schedule_id") < 2:
    raise SystemExit("one-time skipped schedules do not terminate cleanly")
'''
if "event schedules replay pre-creation events" not in validator:
    validator += validator_extension
validator_path.write_text(validator, encoding="utf-8")

print("Phase 19 hostile-review hardening patches applied")
