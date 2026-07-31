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
contract_path.write_text(contract, encoding="utf-8")

source_path = ROOT / "crates/homeserver-service/src/app/wrapper_scheduling.rs"
source = source_path.read_text(encoding="utf-8")
source = source.replace(
    "use std::{collections::BTreeSet, sync::Arc, time::Duration as StdDuration};",
    "use std::{sync::Arc, time::Duration as StdDuration};",
    1,
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
        "        .collect::<rusqlite::Result<Vec<_>>>()\n"
        "        .map_err(Into::into)?;\n"
        "    Ok(rows)\n"
        "}"
    )
    if new_tail not in block:
        if old_tail not in block:
            raise RuntimeError(f"missing owned-row collection anchor: {function_name}")
        block = block.replace(old_tail, new_tail, 1)
    source = source[:start] + block + source[end:]

source_path.write_text(source, encoding="utf-8")
print("Phase 19 native compile repairs applied")
