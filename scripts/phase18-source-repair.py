from pathlib import Path

root = Path(__file__).resolve().parents[1]
path = root / "crates/homeserver-service/src/app/wrapper_orchestration.rs"
text = path.read_text(encoding="utf-8")

text = text.replace(
    "use chrono::{DateTime, Duration, SecondsFormat, Utc};",
    "use chrono::{DateTime, SecondsFormat, Utc};",
    1,
)

scan = """        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };"""
owned = """        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };"""
if text.count(scan) != 2:
    raise RuntimeError(f"unexpected checkpoint scan count: {text.count(scan)}")
text = text.replace(scan, owned)

pair_scan = """        statement
            .query_map(params![plan_id, sequence], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };"""
pair_owned = """        let rows = statement
            .query_map(params![plan_id, sequence], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };"""
if text.count(pair_scan) != 1:
    raise RuntimeError(f"unexpected execution receipt scan count: {text.count(pair_scan)}")
text = text.replace(pair_scan, pair_owned, 1)

old = """    if transaction
        .query_row(
            "SELECT compensation_receipt_id FROM agent_supervised_compensation_receipts WHERE checkpoint_id=?1",
            params![&checkpoint_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some()
    {"""
new = """    let existing_compensation_receipt: Option<String> = transaction
        .query_row(
            "SELECT compensation_receipt_id FROM agent_supervised_compensation_receipts WHERE checkpoint_id=?1",
            params![&checkpoint_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if existing_compensation_receipt.is_some() {"""
if old not in text:
    raise RuntimeError("missing compensation receipt ownership anchor")
text = text.replace(old, new, 1)

anchor = """fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}
"""
parser = anchor + """
fn parse_utc(value: &str, label: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("{label} is invalid"))
        .map(|value| value.with_timezone(&Utc))
}
"""
if "fn parse_utc(" not in text:
    if anchor not in text:
        raise RuntimeError("missing timestamp parser anchor")
    text = text.replace(anchor, parser, 1)

path.write_text(text, encoding="utf-8")

migration_path = root / "database/migrations/0026_supervised_action_orchestration.sql"
migration = migration_path.read_text(encoding="utf-8")
for table, label in (
    ("agent_supervised_action_receipts", "supervised action receipts are immutable"),
    ("agent_supervised_compensation_receipts", "supervised compensation receipts are immutable"),
    ("agent_supervised_action_events", "supervised action events are append-only"),
):
    update_trigger = f"""CREATE TRIGGER IF NOT EXISTS trg_{table.removeprefix('agent_')}_no_update
BEFORE UPDATE ON {table}
BEGIN
  SELECT RAISE(ABORT, '{label}');
END;
"""
    delete_trigger = update_trigger + f"""
CREATE TRIGGER IF NOT EXISTS trg_{table.removeprefix('agent_')}_no_delete
BEFORE DELETE ON {table}
BEGIN
  SELECT RAISE(ABORT, '{label}');
END;
"""
    if f"trg_{table.removeprefix('agent_')}_no_delete" not in migration:
        if update_trigger not in migration:
            raise RuntimeError(f"missing update trigger anchor for {table}")
        migration = migration.replace(update_trigger, delete_trigger, 1)
migration_path.write_text(migration, encoding="utf-8")

contract_path = root / "crates/homeserver-service/tests/phase18_supervised_orchestration_contract.rs"
contract = contract_path.read_text(encoding="utf-8")
connection_anchor = """    let connection = initialize_contract_database();
    connection
        .execute_batch(
"""
connection_patch = """    let connection = initialize_contract_database();
    // This disposable fixture validates immutable triggers only. Referential authority
    // is covered independently by the migration schema and orchestration API tests.
    connection
        .pragma_update(None, "foreign_keys", false)
        .expect("disable foreign keys for trigger-only fixtures");
    connection
        .execute_batch(
"""
if "disable foreign keys for trigger-only fixtures" not in contract:
    if connection_anchor not in contract:
        raise RuntimeError("missing Phase 18 trigger fixture connection anchor")
    contract = contract.replace(connection_anchor, connection_patch, 1)

final_anchor = """    assert!(connection
        .execute(
            "UPDATE agent_supervised_compensation_receipts SET result_code='changed' WHERE compensation_receipt_id=?1",
            params!["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
        )
        .is_err());
}
"""
final_patch = """    assert!(connection
        .execute(
            "UPDATE agent_supervised_compensation_receipts SET result_code='changed' WHERE compensation_receipt_id=?1",
            params!["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM agent_supervised_action_events WHERE event_id=?1",
            params!["11111111-1111-4111-8111-111111111111"],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM agent_supervised_action_receipts WHERE receipt_id=?1",
            params!["22222222-2222-4222-8222-222222222222"],
        )
        .is_err());
    assert!(connection
        .execute(
            "DELETE FROM agent_supervised_compensation_receipts WHERE compensation_receipt_id=?1",
            params!["aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
        )
        .is_err());
}
"""
if "DELETE FROM agent_supervised_action_events" not in contract:
    if final_anchor not in contract:
        raise RuntimeError("missing Phase 18 immutability assertion anchor")
    contract = contract.replace(final_anchor, final_patch, 1)
contract_path.write_text(contract, encoding="utf-8")

validator_path = root / "scripts/validate-supervised-orchestration.py"
validator = validator_path.read_text(encoding="utf-8")
validator_extension = """

migration_contract = (ROOT / "database/migrations/0026_supervised_action_orchestration.sql").read_text(encoding="utf-8")
for immutable_trigger in (
    "trg_supervised_action_receipts_no_delete",
    "trg_supervised_compensation_receipts_no_delete",
    "trg_supervised_action_events_no_delete",
):
    if immutable_trigger not in migration_contract:
        raise SystemExit(f"missing immutable delete trigger: {immutable_trigger}")
"""
if "trg_supervised_action_receipts_no_delete" not in validator:
    validator += validator_extension
validator_path.write_text(validator, encoding="utf-8")
