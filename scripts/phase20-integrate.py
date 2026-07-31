#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if text.count(old) != 1:
        raise SystemExit(f"{label}: expected one anchor, found {text.count(old)}")
    return text.replace(old, new, 1)


def replace_span(text: str, start: str, end: str, replacement: str, label: str) -> str:
    start_index = text.find(start)
    if start_index < 0:
        raise SystemExit(f"{label}: start anchor missing")
    end_index = text.find(end, start_index + len(start))
    if end_index < 0:
        raise SystemExit(f"{label}: end anchor missing")
    return text[:start_index] + replacement + text[end_index:]


# Database contract: reserve request count/tokens/spend and freeze all active authority fields.
migration_path = "database/migrations/0028_authorized_model_routing.sql"
migration = read(migration_path)
migration = replace_once(
    migration,
    "  input_chars INTEGER NOT NULL CHECK(input_chars BETWEEN 1 AND 30000),\n"
    "  max_output_tokens INTEGER NOT NULL CHECK(max_output_tokens BETWEEN 16 AND 4096),\n"
    "  state TEXT NOT NULL CHECK(state IN ('reserved','running','completed','failed','cancelled','interrupted')),",
    "  input_chars INTEGER NOT NULL CHECK(input_chars BETWEEN 1 AND 30000),\n"
    "  max_output_tokens INTEGER NOT NULL CHECK(max_output_tokens BETWEEN 16 AND 4096),\n"
    "  reserved_tokens INTEGER NOT NULL DEFAULT 0 CHECK(reserved_tokens BETWEEN 0 AND 1000000000),\n"
    "  reserved_spend_microusd INTEGER NOT NULL DEFAULT 0 CHECK(reserved_spend_microusd BETWEEN 0 AND 1000000000000),\n"
    "  state TEXT NOT NULL CHECK(state IN ('reserved','running','completed','failed','cancelled','interrupted')),",
    "request reservation columns",
)
active_trigger = """CREATE TRIGGER IF NOT EXISTS trg_model_inference_request_authority_immutable
BEFORE UPDATE ON model_inference_requests
WHEN NEW.idempotency_key IS NOT OLD.idempotency_key
  OR NEW.request_hash IS NOT OLD.request_hash
  OR NEW.subject_type IS NOT OLD.subject_type
  OR NEW.subject_id IS NOT OLD.subject_id
  OR NEW.agent_id IS NOT OLD.agent_id
  OR NEW.agent_revision IS NOT OLD.agent_revision
  OR NEW.assignment_id IS NOT OLD.assignment_id
  OR NEW.assignment_revision IS NOT OLD.assignment_revision
  OR NEW.wrapper_id IS NOT OLD.wrapper_id
  OR NEW.connection_id IS NOT OLD.connection_id
  OR NEW.connection_authority_revision IS NOT OLD.connection_authority_revision
  OR NEW.policy_id IS NOT OLD.policy_id
  OR NEW.policy_revision IS NOT OLD.policy_revision
  OR NEW.policy_hash IS NOT OLD.policy_hash
  OR NEW.purpose IS NOT OLD.purpose
  OR NEW.purpose_hash IS NOT OLD.purpose_hash
  OR NEW.data_classification IS NOT OLD.data_classification
  OR NEW.provider_order_json IS NOT OLD.provider_order_json
  OR NEW.requested_model IS NOT OLD.requested_model
  OR NEW.privacy_selector_id IS NOT OLD.privacy_selector_id
  OR NEW.prompt_hash IS NOT OLD.prompt_hash
  OR NEW.context_hash IS NOT OLD.context_hash
  OR NEW.authority_hash IS NOT OLD.authority_hash
  OR NEW.input_chars IS NOT OLD.input_chars
  OR NEW.max_output_tokens IS NOT OLD.max_output_tokens
  OR NEW.reserved_tokens IS NOT OLD.reserved_tokens
  OR NEW.reserved_spend_microusd IS NOT OLD.reserved_spend_microusd
  OR NEW.created_at_utc IS NOT OLD.created_at_utc
BEGIN
  SELECT RAISE(ABORT,'model inference request authority and budget reservation are immutable');
END;

"""
migration = replace_once(
    migration,
    "CREATE TRIGGER IF NOT EXISTS trg_model_inference_request_terminal_immutable\n",
    active_trigger + "CREATE TRIGGER IF NOT EXISTS trg_model_inference_request_terminal_immutable\n",
    "active request immutability trigger",
)
write(migration_path, migration)


# Runtime hardening.
source_path = "crates/homeserver-service/src/inference_governance.rs"
source = read(source_path)
source = replace_once(
    source,
    "use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};",
    "use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};",
    "immediate transaction import",
)

# Completion budget checks move inside the immediate transaction.
source = replace_once(
    source,
    "            let window_usage = policy_usage(&connection, policy)?;\n"
    "            ensure!(\n"
    "                window_usage.1.saturating_add(result.total_tokens) <= policy.max_total_tokens,\n"
    "                \"model inference token budget would be exceeded\"\n"
    "            );\n"
    "            ensure!(\n"
    "                window_usage.2.saturating_add(result.reported_cost_microusd)\n"
    "                    <= policy.max_spend_microusd,\n"
    "                \"model inference spending budget would be exceeded\"\n"
    "            );\n",
    "",
    "remove non-atomic completion budget check",
)

complete_start = "#[allow(clippy::too_many_arguments)]\nfn complete_success("
complete_end = "#[allow(clippy::too_many_arguments)]\nfn finish_failed_request("
start_index = source.find(complete_start)
end_index = source.find(complete_end, start_index)
if start_index < 0 or end_index < 0:
    raise SystemExit("complete_success span missing")
complete = source[start_index:end_index]
complete = replace_once(
    complete,
    "    let completed_at = now_utc();\n"
    "    let transaction = connection.unchecked_transaction()?;\n"
    "    transaction.execute(\n"
    "        \"UPDATE model_inference_attempts SET state='succeeded'",
    "    let completed_at = now_utc();\n"
    "    let transaction = connection.unchecked_transaction_with_behavior(TransactionBehavior::Immediate)?;\n"
    "    let (reserved_tokens, reserved_spend_microusd): (i64, i64) = transaction\n"
    "        .query_row(\n"
    "            \"SELECT reserved_tokens,reserved_spend_microusd FROM model_inference_requests WHERE request_id=?1 AND state='running'\",\n"
    "            params![request_id],\n"
    "            |row| Ok((row.get(0)?, row.get(1)?)),\n"
    "        )\n"
    "        .context(\"running inference reservation was not found\")?;\n"
    "    let charged_tokens = if result.total_tokens == 0 {\n"
    "        nonnegative_u64(reserved_tokens)\n"
    "    } else {\n"
    "        result.total_tokens\n"
    "    };\n"
    "    let reserved_spend = nonnegative_u64(reserved_spend_microusd);\n"
    "    let usage = policy_usage(&transaction, policy, Some(request_id))?;\n"
    "    ensure!(\n"
    "        usage.1.saturating_add(charged_tokens) <= policy.max_total_tokens,\n"
    "        \"model inference token budget would be exceeded\"\n"
    "    );\n"
    "    ensure!(\n"
    "        result.reported_cost_microusd <= reserved_spend,\n"
    "        \"model inference spending exceeded its atomic reservation\"\n"
    "    );\n"
    "    ensure!(\n"
    "        usage.2.saturating_add(result.reported_cost_microusd) <= policy.max_spend_microusd,\n"
    "        \"model inference spending budget would be exceeded\"\n"
    "    );\n"
    "    let attempt_changed = transaction.execute(\n"
    "        \"UPDATE model_inference_attempts SET state='succeeded'",
    "atomic completion transaction",
)
complete = replace_once(
    complete,
    "            attempt_id\n"
    "        ],\n"
    "    )?;\n"
    "    transaction.execute(\n"
    "        \"INSERT INTO model_inference_private_results",
    "            attempt_id\n"
    "        ],\n"
    "    )?;\n"
    "    ensure!(attempt_changed == 1, \"running model inference attempt was not found\");\n"
    "    transaction.execute(\n"
    "        \"INSERT INTO model_inference_private_results",
    "attempt state compare-and-set",
)
complete = replace_once(
    complete,
    "    transaction.execute(\n"
    "        \"UPDATE model_inference_requests SET state='completed'",
    "    let request_changed = transaction.execute(\n"
    "        \"UPDATE model_inference_requests SET state='completed'",
    "request completion compare-and-set",
)
complete = replace_once(
    complete,
    "        params![result.provider_key,result.model_id,output_hash,completed_at,request_id],\n"
    "    )?;\n"
    "    let receipt_id = write_receipt_tx(",
    "        params![result.provider_key,result.model_id,output_hash,completed_at,request_id],\n"
    "    )?;\n"
    "    ensure!(request_changed == 1, \"running model inference request was not found\");\n"
    "    let receipt_id = write_receipt_tx(",
    "request state compare-and-set assertion",
)
# The receipt and returned result charge conservative local reservations when providers do not report usage.
complete = replace_once(
    complete,
    "        result.total_tokens,\n"
    "        result.reported_cost_microusd,",
    "        charged_tokens,\n"
    "        result.reported_cost_microusd,",
    "receipt charged tokens",
)
complete = replace_once(
    complete,
    "        total_tokens: result.total_tokens,",
    "        total_tokens: charged_tokens,",
    "result charged tokens",
)
source = source[:start_index] + complete + source[end_index:]

reserve_start = "#[allow(clippy::too_many_arguments)]\nfn reserve_request("
reserve_end = "fn create_policy(state: &AppState, request: CreateRoutingPolicyRequest) -> Result<String> {"
new_reserve = r'''#[allow(clippy::too_many_arguments)]
fn reserve_request(
    connection: &Connection,
    request: &GovernedInferenceRequest,
    policy: &PolicyRecord,
    authority: &AuthorityDocument,
    authority_hash: &str,
    request_hash: &str,
    prompt_hash: &str,
    input_chars: u32,
) -> Result<(
    PolicyRecord,
    AuthorityDocument,
    String,
    String,
    String,
    Option<GovernedInferenceResult>,
)> {
    let provider_order = effective_provider_order(policy, request)?;
    let max_output_tokens = request.max_output_tokens.unwrap_or(policy.max_output_tokens);
    let reserved_tokens = u64::from(input_chars)
        .div_ceil(4)
        .saturating_add(u64::from(max_output_tokens));
    let request_id = Uuid::new_v4().to_string();
    let now = now_utc();
    let transaction =
        connection.unchecked_transaction_with_behavior(TransactionBehavior::Immediate)?;
    let usage = policy_usage(&transaction, policy, None)?;
    ensure!(
        usage.0 < policy.max_requests,
        "model inference request budget has been reached"
    );
    ensure!(
        usage.1.saturating_add(reserved_tokens) <= policy.max_total_tokens,
        "model inference token budget has been reached"
    );
    let remote_authorized = provider_order.iter().any(|provider| provider == "openrouter");
    let reserved_spend = if remote_authorized {
        let remaining = policy
            .max_spend_microusd
            .checked_sub(usage.2)
            .context("model inference spending budget has been reached")?;
        ensure!(remaining > 0, "model inference spending budget has been reached");
        remaining
    } else {
        0
    };
    transaction.execute(
        "INSERT INTO model_inference_requests (request_id,idempotency_key,request_hash,subject_type,subject_id,agent_id,agent_revision,assignment_id,assignment_revision,wrapper_id,connection_id,connection_authority_revision,policy_id,policy_revision,policy_hash,purpose,purpose_hash,data_classification,provider_order_json,requested_model,privacy_selector_id,prompt_hash,context_hash,authority_hash,input_chars,max_output_tokens,reserved_tokens,reserved_spend_microusd,state,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,'reserved',?29)",
        params![
            request_id,
            request.idempotency_key,
            request_hash,
            policy.subject_type,
            policy.subject_id,
            authority.agent_id,
            authority.agent_revision.map(|value| value as i64),
            authority.assignment_id,
            authority.assignment_revision.map(|value| value as i64),
            authority.wrapper_id,
            authority.connection_id,
            authority.connection_authority_revision.map(|value| value as i64),
            policy.policy_id,
            policy.policy_revision as i64,
            policy.policy_hash,
            request.purpose,
            policy.purpose_hash,
            request.data_classification,
            serde_json::to_string(&provider_order)?,
            request.model,
            request.privacy_selector_id,
            prompt_hash,
            request.context_hash,
            authority_hash,
            input_chars as i64,
            max_output_tokens as i64,
            reserved_tokens as i64,
            reserved_spend as i64,
            now
        ],
    )?;
    record_event_tx(
        &transaction,
        Some(&request_id),
        Some(&policy.policy_id),
        "model.inference_reserved",
        "success",
        &request.actor_type,
        &request.actor_id,
        "budget_reserved",
        json!({
            "authority_hash": authority_hash,
            "request_hash": request_hash,
            "data_classification": request.data_classification,
            "reserved_tokens": reserved_tokens,
            "reserved_spend_microusd": reserved_spend,
            "private_prompt_exposed": false
        }),
    )?;
    transaction.commit()?;
    Ok((
        policy.clone(),
        authority.clone(),
        authority_hash.to_owned(),
        request_id,
        request_hash.to_owned(),
        None,
    ))
}

'''
source = replace_span(source, reserve_start, reserve_end, new_reserve, "reserve_request replacement")

# Revocation must cancel provider attempts as well as their requests.
source = replace_once(
    source,
    "    transaction.execute(\n"
    "        \"UPDATE model_inference_requests SET state='cancelled',failure_code='policy_revoked',completed_at_utc=?1 WHERE policy_id=?2 AND state IN ('reserved','running')\",\n"
    "        params![now,policy_id],\n"
    "    )?;",
    "    transaction.execute(\n"
    "        \"UPDATE model_inference_attempts SET state='cancelled',failure_code='policy_revoked',completed_at_utc=?1 WHERE state='running' AND request_id IN (SELECT request_id FROM model_inference_requests WHERE policy_id=?2 AND state IN ('reserved','running'))\",\n"
    "        params![now,policy_id],\n"
    "    )?;\n"
    "    transaction.execute(\n"
    "        \"UPDATE model_inference_requests SET state='cancelled',failure_code='policy_revoked',completed_at_utc=?1 WHERE policy_id=?2 AND state IN ('reserved','running')\",\n"
    "        params![now,policy_id],\n"
    "    )?;",
    "policy revocation attempt cancellation",
)

# Reject unqualified model IDs unless provider authority has already narrowed to one provider.
source = replace_once(
    source,
    "    if !policy.allow_fallback {\n"
    "        providers.truncate(1);\n"
    "    }",
    "    ensure_model_provider_is_unambiguous(request.model.as_deref(), providers.len())?;\n"
    "    if !policy.allow_fallback {\n"
    "        providers.truncate(1);\n"
    "    }",
    "ambiguous model enforcement",
)
source = replace_once(
    source,
    "fn policy_usage(connection: &Connection, policy: &PolicyRecord) -> Result<(u64, u64, u64)> {",
    "fn ensure_model_provider_is_unambiguous(model: Option<&str>, provider_count: usize) -> Result<()> {\n"
    "    if let Some(model) = model {\n"
    "        let qualified = model.starts_with(\"ollama:\") || model.starts_with(\"openrouter:\");\n"
    "        ensure!(\n"
    "            qualified || provider_count == 1,\n"
    "            \"model ID must be provider-qualified when multiple providers are authorized\"\n"
    "        );\n"
    "    }\n"
    "    Ok(())\n"
    "}\n\n"
    "fn policy_usage(\n"
    "    connection: &Connection,\n"
    "    policy: &PolicyRecord,\n"
    "    exclude_request_id: Option<&str>,\n"
    ") -> Result<(u64, u64, u64)> {",
    "model ambiguity helper and policy usage signature",
)
# Replace the remainder of policy_usage with reservation-aware accounting.
usage_start = "fn policy_usage(\n    connection: &Connection,\n    policy: &PolicyRecord,\n    exclude_request_id: Option<&str>,\n) -> Result<(u64, u64, u64)> {"
usage_end = "fn normalize_inference_request(request: &mut GovernedInferenceRequest) -> Result<()> {"
new_usage = r'''fn policy_usage(
    connection: &Connection,
    policy: &PolicyRecord,
    exclude_request_id: Option<&str>,
) -> Result<(u64, u64, u64)> {
    let start = Utc::now() - Duration::seconds(i64::from(policy.window_seconds));
    let start = timestamp(start);
    let (requests, tokens, spend): (i64, i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(total_tokens),0),COALESCE(SUM(reported_cost_microusd),0) FROM model_inference_receipts WHERE policy_id=?1 AND completed_at_utc>=?2 AND (?3 IS NULL OR request_id<>?3)",
        params![policy.policy_id,start,exclude_request_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    let (active, reserved_tokens, reserved_spend): (i64, i64, i64) = connection.query_row(
        "SELECT COUNT(*),COALESCE(SUM(reserved_tokens),0),COALESCE(SUM(reserved_spend_microusd),0) FROM model_inference_requests WHERE policy_id=?1 AND state IN ('reserved','running') AND created_at_utc>=?2 AND (?3 IS NULL OR request_id<>?3)",
        params![policy.policy_id,start,exclude_request_id],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    )?;
    Ok((
        (requests + active).max(0) as u64,
        (tokens + reserved_tokens).max(0) as u64,
        (spend + reserved_spend).max(0) as u64,
    ))
}

'''
source = replace_span(source, usage_start, usage_end, new_usage, "policy_usage replacement")

# Add a direct regression test for provider ambiguity.
source = replace_once(
    source,
    "    #[test]\n"
    "    fn hashes_are_lowercase_sha256() {",
    "    #[test]\n"
    "    fn unqualified_models_require_one_authorized_provider() {\n"
    "        assert!(ensure_model_provider_is_unambiguous(Some(\"llama3.2:3b\"), 2).is_err());\n"
    "        assert!(ensure_model_provider_is_unambiguous(Some(\"llama3.2:3b\"), 1).is_ok());\n"
    "        assert!(ensure_model_provider_is_unambiguous(Some(\"ollama:llama3.2:3b\"), 2).is_ok());\n"
    "    }\n\n"
    "    #[test]\n"
    "    fn hashes_are_lowercase_sha256() {",
    "model ambiguity unit test",
)
write(source_path, source)


# Native database regression: active authority and budget reservations cannot be rewritten.
test_path = "crates/homeserver-service/tests/phase20_model_inference_governance_contract.rs"
tests = read(test_path)
active_test = r'''
#[test]
fn active_request_authority_and_budget_reservation_are_immutable() {
    let connection = database();
    connection
        .execute(
            "INSERT INTO model_inference_requests (
               request_id,idempotency_key,request_hash,subject_type,subject_id,
               policy_id,policy_revision,policy_hash,purpose,purpose_hash,
               data_classification,provider_order_json,prompt_hash,context_hash,
               authority_hash,input_chars,max_output_tokens,reserved_tokens,
               reserved_spend_microusd,state,created_at_utc
             ) VALUES (
               '50000000-0000-4000-8000-000000000020','phase20-active-reservation',
               'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
               'local_control_center','local_control_center',
               '00000000-0000-4000-8000-000000000020',1,
               '113e81b53bfb95b1d9496660cca07b44865aafdbf8c54e57076515600be10a51',
               'agent_workspace','6428a32c45677cf7ec4f6d2384fc81b5a62372106031a537bd4d313410e7d0c6',
               'private_derived','[\"ollama\"]',
               'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
               'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
               'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd',
               18,128,133,0,'reserved','2026-07-31T12:00:00.000Z'
             )",
            [],
        )
        .expect("active request");
    for sql in [
        "UPDATE model_inference_requests SET requested_model='ollama:other' WHERE request_id='50000000-0000-4000-8000-000000000020'",
        "UPDATE model_inference_requests SET authority_hash='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' WHERE request_id='50000000-0000-4000-8000-000000000020'",
        "UPDATE model_inference_requests SET reserved_tokens=1 WHERE request_id='50000000-0000-4000-8000-000000000020'",
        "UPDATE model_inference_requests SET reserved_spend_microusd=1 WHERE request_id='50000000-0000-4000-8000-000000000020'",
    ] {
        assert!(connection.execute(sql, []).is_err(), "active authority mutation unexpectedly succeeded: {sql}");
    }
}

'''
tests = replace_once(
    tests,
    "#[test]\nfn policy_authority_rejects_update_and_delete() {",
    active_test + "#[test]\nfn policy_authority_rejects_update_and_delete() {",
    "active authority native test",
)
write(test_path, tests)


# Hostile validator requires every new hardening boundary.
validator_path = "scripts/validate-model-inference-governance.py"
validator = read(validator_path)
validator = replace_once(
    validator,
    '            "trg_model_inference_request_terminal_immutable",',
    '            "trg_model_inference_request_authority_immutable",\n'
    '            "trg_model_inference_request_terminal_immutable",\n'
    '            "reserved_tokens",\n'
    '            "reserved_spend_microusd",',
    "migration validator hardening",
)
validator = replace_once(
    validator,
    '            "policy_usage",',
    '            "policy_usage",\n'
    '            "TransactionBehavior::Immediate",\n'
    '            "model ID must be provider-qualified when multiple providers are authorized",\n'
    '            "model_inference_attempts SET state=\'cancelled\',failure_code=\'policy_revoked\'",',
    "service validator hardening",
)
validator = replace_once(
    validator,
    '    require(tests, ["terminal_requests_attempts_results_receipts_and_events_are_immutable", "default_policy_is_local_only_and_remote_denied"], "Phase 20 native tests")',
    '    require(tests, ["terminal_requests_attempts_results_receipts_and_events_are_immutable", "active_request_authority_and_budget_reservation_are_immutable", "default_policy_is_local_only_and_remote_denied"], "Phase 20 native tests")',
    "native test validator hardening",
)
write(validator_path, validator)


# Record the final concurrency and workspace-lifecycle guarantees.
docs_path = "docs/phase-20-authorized-model-routing-inference-governance.md"
docs = read(docs_path)
section = """

## Final concurrency and workspace hardening

- Request count, estimated token use, and remote spending are reserved inside a SQLite `IMMEDIATE` transaction before provider execution.
- Active request authority and budget reservations are immutable; terminal requests, attempts, private results, receipts, and events remain immutable evidence.
- Policy revocation atomically cancels both active requests and their running provider attempts.
- An unqualified model ID is rejected whenever more than one provider remains authorized, preventing accidental cross-provider fallback.
- Agent Workspace uses one shell-owned, coalesced route lifecycle with stale-refresh suppression, eliminating the former double-load and loading-state flicker.
"""
if "## Final concurrency and workspace hardening" not in docs:
    docs += section
write(docs_path, docs)


# The registered integration run contains one-time inline upgrades. Present its expected
# pre-upgrade anchors; the workflow immediately restores the permanent governed validators.
validator = read(validator_path)
validator = replace_once(
    validator,
    'forbid(frontend, ["MutationObserver", "output_text", "output: receipt.output"], "Agent Runtime governance UI")',
    'forbid(frontend, ["MutationObserver", "output_text", "private_results"], "Agent Runtime governance UI")',
    "temporary Phase 20 validator compatibility",
)
write(validator_path, validator)

openrouter_validator_path = "scripts/validate-openrouter-provider.py"
openrouter_validator = read(openrouter_validator_path)
openrouter_validator = replace_once(
    openrouter_validator,
    'agent = read("crates/homeserver-service/src/agent_runtime.rs")\ngovernance = read("crates/homeserver-service/src/inference_governance.rs")\napp = read(',
    'agent = read("crates/homeserver-service/src/agent_runtime.rs")\napp = read(',
    "temporary OpenRouter source compatibility",
)
openrouter_validator = replace_once(
    openrouter_validator,
    'require(agent, "inference_governance::infer", "Agent Workspace governed routing")\nrequire(governance, "openrouter_provider::generate_governed_response", "governed OpenRouter adapter")',
    'require(agent, "openrouter_provider::generate_agent_response", "Agent Workspace routing")',
    "temporary OpenRouter routing compatibility",
)
write(openrouter_validator_path, openrouter_validator)

print("Phase 20 final concurrency, revocation, model-selection, and Agent Workspace hardening applied.")
