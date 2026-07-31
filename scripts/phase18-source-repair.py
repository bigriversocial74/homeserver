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

migration = root / "database/migrations/0026_supervised_action_orchestration.sql"
for number, line in enumerate(migration.read_text(encoding="utf-8").splitlines(), start=1):
    if 1 <= number <= 260:
        print(f"PHASE18_SCHEMA {number:04d}: {line}")

contract = root / "crates/homeserver-service/tests/phase18_supervised_orchestration_contract.rs"
for number, line in enumerate(contract.read_text(encoding="utf-8").splitlines(), start=1):
    if 100 <= number <= 240:
        print(f"PHASE18_FIXTURE {number:04d}: {line}")

raise RuntimeError("Phase 18 schema diagnostic complete")
