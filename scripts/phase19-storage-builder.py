from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

migration_path = ROOT / "database/migrations/0027_authorized_agent_scheduling.sql"
migration = migration_path.read_text(encoding="utf-8")
trigger_anchor = "CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_event_inbox_no_update\n"
storage_triggers = '''CREATE TRIGGER IF NOT EXISTS trg_agent_schedule_definitions_immutable_fields
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

'''
if "trg_agent_schedule_definitions_immutable_fields" not in migration:
    if trigger_anchor not in migration:
        raise RuntimeError("Phase 19 storage trigger insertion anchor is missing")
    migration = migration.replace(trigger_anchor, storage_triggers + trigger_anchor, 1)
migration_path.write_text(migration, encoding="utf-8")

source_path = ROOT / "crates/homeserver-service/src/app/wrapper_scheduling.rs"
source = source_path.read_text(encoding="utf-8")
failure_update_old = '''        "UPDATE agent_schedule_definitions SET state='failed',next_fire_at_utc=NULL,failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE schedule_id=?3",
        params![failure_code, now, schedule.schedule_id],
'''
failure_update_new = '''        "UPDATE agent_schedule_definitions SET state='failed',next_fire_at_utc=NULL,failure_code=?1,completed_at_utc=?2,updated_at_utc=?2 WHERE schedule_id=?3 AND state='active'",
        params![failure_code, now, schedule.schedule_id],
'''
if failure_update_new not in source:
    if failure_update_old not in source:
        raise RuntimeError("schedule terminal-authority preservation anchor is missing")
    source = source.replace(failure_update_old, failure_update_new, 1)
source_path.write_text(source, encoding="utf-8")

contract_path = ROOT / "crates/homeserver-service/tests/phase19_authorized_scheduling_contract.rs"
contract = contract_path.read_text(encoding="utf-8")
trigger_list_anchor = '''        "trg_agent_schedule_audit_no_update",
        "trg_agent_schedule_audit_no_delete",
    ] {
'''
trigger_list_patch = '''        "trg_agent_schedule_audit_no_update",
        "trg_agent_schedule_audit_no_delete",
        "trg_agent_schedule_definitions_immutable_fields",
        "trg_agent_schedule_definitions_no_delete",
        "trg_agent_schedule_private_templates_no_update",
        "trg_agent_schedule_private_templates_no_delete",
        "trg_agent_schedule_runs_terminal_no_update",
        "trg_agent_schedule_runs_no_delete",
    ] {
'''
if trigger_list_patch not in contract:
    if trigger_list_anchor not in contract:
        raise RuntimeError("Phase 19 storage trigger test anchor is missing")
    contract = contract.replace(trigger_list_anchor, trigger_list_patch, 1)

assertion_anchor = '''        "DELETE FROM agent_schedule_audit_events WHERE audit_event_id='99999999-9999-4999-8999-999999999999'",
    ] {
'''
assertion_patch = '''        "DELETE FROM agent_schedule_audit_events WHERE audit_event_id='99999999-9999-4999-8999-999999999999'",
        "UPDATE agent_schedule_definitions SET authority_hash='dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd' WHERE schedule_id='11111111-1111-4111-8111-111111111111'",
        "DELETE FROM agent_schedule_definitions WHERE schedule_id='11111111-1111-4111-8111-111111111111'",
        "UPDATE agent_schedule_private_templates SET template_json='{\"changed\":true}' WHERE schedule_id='11111111-1111-4111-8111-111111111111'",
        "DELETE FROM agent_schedule_private_templates WHERE schedule_id='11111111-1111-4111-8111-111111111111'",
        "UPDATE agent_schedule_runs SET result_code='changed' WHERE run_id='77777777-7777-4777-8777-777777777777'",
        "DELETE FROM agent_schedule_runs WHERE run_id='77777777-7777-4777-8777-777777777777'",
    ] {
'''
if assertion_patch not in contract:
    if assertion_anchor not in contract:
        raise RuntimeError("Phase 19 storage mutation assertion anchor is missing")
    contract = contract.replace(assertion_anchor, assertion_patch, 1)
contract_path.write_text(contract, encoding="utf-8")

validator_path = ROOT / "scripts/validate-authorized-scheduling.py"
validator = validator_path.read_text(encoding="utf-8")
extension = '''

for immutable_trigger in (
    "trg_agent_schedule_definitions_immutable_fields",
    "trg_agent_schedule_definitions_no_delete",
    "trg_agent_schedule_private_templates_no_update",
    "trg_agent_schedule_private_templates_no_delete",
    "trg_agent_schedule_runs_terminal_no_update",
    "trg_agent_schedule_runs_no_delete",
):
    if immutable_trigger not in migration:
        raise SystemExit(f"missing Phase 19 immutable storage trigger: {immutable_trigger}")
if "WHERE schedule_id=?3 AND state='active'" not in source:
    raise SystemExit("schedule cancellation or pause can be overwritten by a failed plan creation")
'''
if "missing Phase 19 immutable storage trigger" not in validator:
    validator += extension
validator_path.write_text(validator, encoding="utf-8")

print("Phase 19 immutable storage patches applied")
