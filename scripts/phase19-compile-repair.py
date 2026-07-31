from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

contract_path = ROOT / "crates/homeserver-service/tests/phase19_authorized_scheduling_contract.rs"
contract = contract_path.read_text(encoding="utf-8")
old_privacy = "../../../database/migrations/0024_private_context_and_result_egress.sql"
new_privacy = "../../../database/migrations/0024_private_knowledge_boundary.sql"
if new_privacy not in contract:
    if old_privacy not in contract:
        raise RuntimeError("Phase 19 privacy migration test anchor is missing")
    contract = contract.replace(old_privacy, new_privacy, 1)

vault_constant = '''const VAULT_MIGRATION: &str =
    include_str!("../../../database/migrations/0005_knowledge_vault.sql");
'''
privacy_constant = '''const PRIVACY_MIGRATION: &str =
    include_str!("../../../database/migrations/0024_private_knowledge_boundary.sql");
'''
if vault_constant not in contract:
    if privacy_constant not in contract:
        raise RuntimeError("Phase 19 privacy constant anchor is missing")
    contract = contract.replace(privacy_constant, vault_constant + privacy_constant, 1)

migration_order = '''        AGENT_MIGRATION,
        PRIVACY_MIGRATION,
'''
migration_order_with_vault = '''        AGENT_MIGRATION,
        VAULT_MIGRATION,
        PRIVACY_MIGRATION,
'''
if migration_order_with_vault not in contract:
    if migration_order not in contract:
        raise RuntimeError("Phase 19 migration order anchor is missing")
    contract = contract.replace(migration_order, migration_order_with_vault, 1)
contract_path.write_text(contract, encoding="utf-8")

source_path = ROOT / "crates/homeserver-service/src/app/wrapper_scheduling.rs"
source = source_path.read_text(encoding="utf-8")
source = source.replace(
    "use std::{collections::BTreeSet, sync::Arc, time::Duration as StdDuration};",
    "use std::{sync::Arc, time::Duration as StdDuration};",
    1,
)

api_result_anchor = "type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;\n"
trigger_alias = "type NormalizedTrigger = (Option<String>, Option<i64>, Option<String>, Option<String>, Option<String>);\n"
if trigger_alias not in source:
    if api_result_anchor not in source:
        raise RuntimeError("Phase 19 trigger type alias anchor is missing")
    source = source.replace(api_result_anchor, api_result_anchor + trigger_alias, 1)

complex_signature = ''') -> Result<(
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<String>,
)> {
'''
simple_signature = ") -> Result<NormalizedTrigger> {\n"
if simple_signature not in source:
    if complex_signature not in source:
        raise RuntimeError("Phase 19 normalized trigger signature anchor is missing")
    source = source.replace(complex_signature, simple_signature, 1)

source = source.replace("template_json.as_bytes().len()", "template_json.len()")
source = source.replace("metadata_json.as_bytes().len()", "metadata_json.len()")
source = source.replace(
    "allowed.iter().any(|candidate| *candidate == value.as_str())",
    "allowed.contains(&value.as_str())",
)

commit_ending = "    transaction.commit()\n}"
commit_replacement = "    transaction.commit()?;\n    Ok(())\n}"
commit_count = source.count(commit_ending)
if commit_count == 0 and commit_replacement not in source:
    raise RuntimeError("Phase 19 transaction completion anchors are missing")
source = source.replace(commit_ending, commit_replacement)

source = source.replace(
    "            schedule_id: schedule.schedule_id.clone(),\n",
    "",
)
source = source.replace(
    "            scheduled_for_utc: scheduled_for.to_owned(),\n",
    "",
)

for function_name in ("read_schedules", "read_runs", "read_events", "read_receipts"):
    marker = f"fn {function_name}("
    start = source.find(marker)
    if start < 0:
        raise RuntimeError(f"missing Phase 19 query function: {function_name}")
    end = source.find("\nfn ", start + len(marker))
    if end < 0:
        raise RuntimeError(f"unable to bound Phase 19 query function: {function_name}")
    block = source[start:end]
    if "    let rows = statement\n" not in block:
        if "    statement\n" not in block:
            raise RuntimeError(f"missing statement lifetime anchor: {function_name}")
        block = block.replace("    statement\n", "    let rows = statement\n", 1)
    old_tail = (
        "        .collect::<rusqlite::Result<Vec<_>>>()\n"
        "        .map_err(Into::into)\n"
        "}"
    )
    new_tail = (
        "        .collect::<rusqlite::Result<Vec<_>>>()?;\n"
        "    Ok(rows)\n"
        "}"
    )
    ambiguous_tail = (
        "        .collect::<rusqlite::Result<Vec<_>>>()\n"
        "        .map_err(Into::into)?;\n"
        "    Ok(rows)\n"
        "}"
    )
    if new_tail not in block:
        if ambiguous_tail in block:
            block = block.replace(ambiguous_tail, new_tail, 1)
        elif old_tail in block:
            block = block.replace(old_tail, new_tail, 1)
        else:
            raise RuntimeError(f"missing owned-row collection anchor: {function_name}")
    source = source[:start] + block + source[end:]

source_path.write_text(source, encoding="utf-8")
print("Phase 19 native compile repairs applied")
