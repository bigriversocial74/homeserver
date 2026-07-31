#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected one anchor, found {count}")
    return text.replace(old, new, 1)


# Canonical default policy hash and immutable archive authority.
path = "database/migrations/0029_tamper_evident_evidence_archive.sql"
text = read(path)
text = replace_once(
    text,
    "610b795bf96f1b5a42962f2931f320034974b6cf294945d1e9c44a78159ecdf1",
    "faeef059a975afe172c0640813d05d2331a71e48224df64d138a44e837b2c84f",
    "canonical default policy hash",
)
authority_trigger = """CREATE TRIGGER IF NOT EXISTS trg_evidence_archives_authority_immutable
BEFORE UPDATE ON evidence_archives
WHEN NEW.idempotency_key IS NOT OLD.idempotency_key
  OR NEW.policy_id IS NOT OLD.policy_id
  OR NEW.policy_revision IS NOT OLD.policy_revision
  OR NEW.previous_archive_id IS NOT OLD.previous_archive_id
  OR NEW.previous_archive_hash IS NOT OLD.previous_archive_hash
  OR NEW.archive_sequence IS NOT OLD.archive_sequence
  OR NEW.file_name IS NOT OLD.file_name
  OR NEW.storage_path IS NOT OLD.storage_path
  OR NEW.encryption IS NOT OLD.encryption
  OR NEW.created_by_type IS NOT OLD.created_by_type
  OR NEW.created_by_id IS NOT OLD.created_by_id
  OR NEW.created_at_utc IS NOT OLD.created_at_utc
BEGIN
  SELECT RAISE(ABORT,'evidence archive authority and chain identity are immutable');
END;

"""
text = replace_once(
    text,
    "CREATE TRIGGER IF NOT EXISTS trg_evidence_archives_terminal_immutable\n",
    authority_trigger + "CREATE TRIGGER IF NOT EXISTS trg_evidence_archives_terminal_immutable\n",
    "archive authority immutability",
)
write(path, text)


# Explicit reviewed evidence-table allowlist and health-chain validation.
path = "crates/homeserver-service/src/evidence_archive.rs"
text = read(path)
constant_anchor = 'const ARCHIVE_DIRECTORY: &str = "evidence-archives";\n'
reviewed_tables = '''const ARCHIVE_DIRECTORY: &str = "evidence-archives";
const REVIEWED_EVIDENCE_TABLES: &[&str] = &[
    "service_events",
    "wrapper_events",
    "wrapper_grant_events",
    "wrapper_authorization_receipts",
    "wrapper_job_events",
    "wrapper_job_execution_receipts",
    "agent_action_receipts",
    "agent_lifecycle_events",
    "private_knowledge_access_receipts",
    "agent_runtime_receipts",
    "agent_runtime_events",
    "agent_runtime_audit_records",
    "agent_supervised_action_receipts",
    "agent_supervised_compensation_receipts",
    "agent_supervised_action_events",
    "agent_schedule_event_inbox",
    "agent_schedule_receipts",
    "agent_schedule_audit_events",
    "model_provider_usage_receipts",
    "model_inference_receipts",
    "model_inference_events",
];
'''
text = replace_once(text, constant_anchor, reviewed_tables, "reviewed evidence table constant")

old_allowlist = '''fn is_allowed_evidence_table(table: &str) -> bool {
    if table == "service_events" {
        return true;
    }
    if table.starts_with("evidence_archive_") || !valid_identifier(table) {
        return false;
    }
    let safe_suffix = table.ends_with("_events")
        || table.ends_with("_receipts")
        || table.ends_with("_audit_records");
    if !safe_suffix {
        return false;
    }
    let forbidden = [
        "private_results",
        "private_inputs",
        "messages",
        "documents",
        "payloads",
        "credentials",
        "secrets",
        "tokens",
        "sync_queue",
    ];
    !forbidden.iter().any(|marker| table.contains(marker))
}
'''
new_allowlist = '''fn is_allowed_evidence_table(table: &str) -> bool {
    REVIEWED_EVIDENCE_TABLES.contains(&table)
}
'''
text = replace_once(text, old_allowlist, new_allowlist, "closed reviewed evidence allowlist")

health_anchor = '''    let duplicate_members: i64 = connection.query_row(
        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",
        [],
        |row| row.get(0),
    )?;
    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");

'''
health_insert = '''    let duplicate_members: i64 = connection.query_row(
        "SELECT COUNT(*) FROM (SELECT source_table,source_key,COUNT(*) AS total FROM evidence_archive_members GROUP BY source_table,source_key HAVING total<>1)",
        [],
        |row| row.get(0),
    )?;
    ensure!(duplicate_members == 0, "evidence archive source membership is ambiguous");

    let policy = latest_policy(connection)?;
    ensure!(
        hash_policy(&policy)? == policy.policy_hash,
        "evidence archive policy hash is invalid"
    );
    verify_archive_chain(connection)?;

'''
text = replace_once(text, health_anchor, health_insert, "policy and archive chain health checks")

policy_document_anchor = '''    let document = json!({
        "schema": "homeserver.evidence-archive-policy.v1",
        "policy_revision": revision,
        "enabled": request.enabled,
        "interval_hours": request.interval_hours,
        "max_records_per_archive": request.max_records_per_archive,
        "retention_count": request.retention_count,
        "max_package_bytes": request.max_package_bytes,
        "created_by_user_id": &actor,
        "reason": &reason
    });
    let policy_hash = hash_json(&document)?;
'''
policy_hash_call = '''    let policy_record = PolicyRecord {
        policy_id: String::new(),
        policy_revision: revision,
        enabled: request.enabled,
        interval_hours: request.interval_hours,
        max_records_per_archive: request.max_records_per_archive,
        retention_count: request.retention_count,
        max_package_bytes: request.max_package_bytes,
        policy_hash: String::new(),
        created_by_user_id: actor.clone(),
        reason: reason.clone(),
        created_at_utc: String::new(),
    };
    let policy_hash = hash_policy(&policy_record)?;
'''
text = replace_once(text, policy_document_anchor, policy_hash_call, "canonical policy hash reuse")

helper_anchor = '''fn latest_verified_archive(connection: &Connection) -> Result<Option<(String, String)>> {
'''
helpers = '''fn hash_policy(policy: &PolicyRecord) -> Result<String> {
    hash_json(&json!({
        "schema": "homeserver.evidence-archive-policy.v1",
        "policy_revision": policy.policy_revision,
        "enabled": policy.enabled,
        "interval_hours": policy.interval_hours,
        "max_records_per_archive": policy.max_records_per_archive,
        "retention_count": policy.retention_count,
        "max_package_bytes": policy.max_package_bytes,
        "created_by_user_id": policy.created_by_user_id,
        "reason": policy.reason
    }))
}

fn verify_archive_chain(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT archive_id,archive_sequence,previous_archive_id,previous_archive_hash,manifest_sha256 FROM evidence_archives WHERE state='verified' ORDER BY archive_sequence,archive_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let archives = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut previous_id = None::<String>;
    let mut previous_manifest_hash = ZERO_HASH.to_owned();
    let mut previous_sequence = 0_i64;
    for (archive_id, sequence, recorded_previous_id, recorded_previous_hash, manifest_hash) in archives {
        ensure!(sequence > previous_sequence, "evidence archive sequence is not strictly increasing");
        ensure!(recorded_previous_id == previous_id, "evidence archive predecessor identity is invalid");
        ensure!(recorded_previous_hash == previous_manifest_hash, "evidence archive predecessor hash is invalid");
        ensure!(manifest_hash.len() == 64, "evidence archive manifest hash is invalid");
        previous_sequence = sequence;
        previous_id = Some(archive_id);
        previous_manifest_hash = manifest_hash;
    }
    Ok(())
}

fn latest_verified_archive(connection: &Connection) -> Result<Option<(String, String)>> {
'''
text = replace_once(text, helper_anchor, helpers, "archive chain helper")

old_test = '''    fn evidence_allowlist_rejects_private_content_tables() {
        assert!(is_allowed_evidence_table("agent_runtime_receipts"));
        assert!(is_allowed_evidence_table("model_inference_events"));
        assert!(is_allowed_evidence_table("private_knowledge_egress_receipts"));
        assert!(!is_allowed_evidence_table("model_inference_private_results"));
        assert!(!is_allowed_evidence_table("agent_messages"));
        assert!(!is_allowed_evidence_table("wrapper_job_payloads"));
        assert!(!is_allowed_evidence_table("evidence_archive_events"));
    }
'''
new_test = '''    fn evidence_allowlist_is_explicit_and_rejects_future_suffix_matches() {
        assert!(is_allowed_evidence_table("wrapper_authorization_receipts"));
        assert!(is_allowed_evidence_table("agent_runtime_receipts"));
        assert!(is_allowed_evidence_table("model_inference_events"));
        assert!(is_allowed_evidence_table("private_knowledge_access_receipts"));
        assert!(!is_allowed_evidence_table("model_inference_private_results"));
        assert!(!is_allowed_evidence_table("agent_messages"));
        assert!(!is_allowed_evidence_table("future_private_events"));
        assert!(!is_allowed_evidence_table("future_secret_receipts"));
        assert!(!is_allowed_evidence_table("evidence_archive_events"));
    }
'''
text = replace_once(text, old_test, new_test, "explicit allowlist unit test")
write(path, text)


# Desktop independently hashes every exported byte before recording an export receipt.
path = "src-tauri/Cargo.toml"
text = read(path)
text = replace_once(text, "serde_json.workspace = true\n", "serde_json.workspace = true\nsha2.workspace = true\nhex.workspace = true\n", "desktop hashing dependencies")
write(path, text)

path = "src-tauri/src/lib.rs"
text = read(path)
text = replace_once(text, "use serde::{de::DeserializeOwned, Deserialize, Serialize};\n", "use serde::{de::DeserializeOwned, Deserialize, Serialize};\nuse sha2::{Digest, Sha256};\n", "desktop SHA-256 import")
text = replace_once(
    text,
    '''    if package_sha256.len() != 64 || !package_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Evidence archive package hash is invalid.".to_owned());
    }
''',
    '''    if package_sha256.len() != 64 || !package_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Evidence archive package hash is invalid.".to_owned());
    }
    let expected_package_sha256 = package_sha256.to_ascii_lowercase();
''',
    "normalized expected export hash",
)
text = replace_once(
    text,
    '''    let mut stream = response.bytes_stream();
    let mut total_bytes = 0_u64;
    let transfer_result = async {
''',
    '''    let mut stream = response.bytes_stream();
    let mut total_bytes = 0_u64;
    let mut package_hasher = Sha256::new();
    let transfer_result = async {
''',
    "streaming export hasher",
)
text = replace_once(
    text,
    '''            output
                .write_all(&chunk)
                .await
                .map_err(|error| error.to_string())?;
''',
    '''            package_hasher.update(&chunk);
            output
                .write_all(&chunk)
                .await
                .map_err(|error| error.to_string())?;
''',
    "hash exported archive chunks",
)
text = replace_once(
    text,
    '''    if let Err(error) = transfer_result {
        drop(output);
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err(error);
    }

    let receipt: serde_json::Value = post_json(
''',
    '''    if let Err(error) = transfer_result {
        drop(output);
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err(error);
    }
    drop(output);
    let downloaded_package_sha256 = hex::encode(package_hasher.finalize());
    if downloaded_package_sha256 != expected_package_sha256 {
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err("Evidence archive export hash verification failed; the incomplete file was removed.".to_owned());
    }

    let receipt: serde_json::Value = post_json(
''',
    "verify exported archive digest",
)
text = replace_once(
    text,
    '            "package_sha256": package_sha256,\n',
    '            "package_sha256": expected_package_sha256,\n',
    "record verified export digest",
)
write(path, text)


# Native migration tests enforce the canonical seed hash and immutable chain identity.
path = "crates/homeserver-service/tests/phase21_evidence_archive_contract.rs"
text = read(path)
text = replace_once(
    text,
    '    assert_eq!(policy.5.len(), 64);\n',
    '    assert_eq!(policy.5, "faeef059a975afe172c0640813d05d2331a71e48224df64d138a44e837b2c84f");\n',
    "canonical default policy test",
)
text = replace_once(
    text,
    '''        "UPDATE evidence_archives SET package_sha256='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' WHERE archive_sequence=1",
''',
    '''        "UPDATE evidence_archives SET package_sha256='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' WHERE archive_sequence=1",
        "UPDATE evidence_archives SET previous_archive_hash='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' WHERE archive_sequence=1",
        "UPDATE evidence_archives SET archive_sequence=2 WHERE archive_sequence=1",
        "UPDATE evidence_archives SET storage_path='C:/other.mgha' WHERE archive_sequence=1",
''',
    "archive chain identity mutation tests",
)
write(path, text)


# Hostile validator now rejects suffix-based admission and requires desktop byte hashing.
path = "scripts/validate-evidence-archive.py"
text = read(path)
text = replace_once(
    text,
    '''        'table.ends_with("_events")',
        'table.ends_with("_receipts")',
        'table.ends_with("_audit_records")',
''',
    '''        "REVIEWED_EVIDENCE_TABLES",
        '"wrapper_events"',
        '"wrapper_authorization_receipts"',
        '"wrapper_job_execution_receipts"',
        '"agent_action_receipts"',
        '"private_knowledge_access_receipts"',
        '"agent_runtime_audit_records"',
        '"agent_supervised_compensation_receipts"',
        '"agent_schedule_event_inbox"',
        '"model_inference_receipts"',
''',
    "explicit allowlist validator",
)
text = replace_once(
    text,
    '''        "recover_interrupted_archives",
        "create_automatic_if_due",
''',
    '''        "recover_interrupted_archives",
        "create_automatic_if_due",
        "hash_policy",
        "verify_archive_chain",
''',
    "archive integrity health validator",
)
text = replace_once(
    text,
    '''forbid(
    service,
    [
        "prompt_text",
''',
    '''forbid(
    service,
    [
        'table.ends_with("_events")',
        'table.ends_with("_receipts")',
        'table.ends_with("_audit_records")',
        "prompt_text",
''',
    "forbid suffix-based admission",
)
text = replace_once(
    text,
    '''        "application/vnd.microgifter.homeserver-evidence-archive",
        "runtime::homeserver_evidence_archives",
''',
    '''        "application/vnd.microgifter.homeserver-evidence-archive",
        "runtime::homeserver_evidence_archives",
        "Sha256::new()",
        "package_hasher.update(&chunk)",
        "downloaded_package_sha256 != expected_package_sha256",
        "Evidence archive export hash verification failed; the incomplete file was removed.",
''',
    "desktop export digest validator",
)
text = replace_once(
    text,
    '''        "trg_evidence_archives_terminal_immutable",
''',
    '''        "trg_evidence_archives_authority_immutable",
        "trg_evidence_archives_terminal_immutable",
        "faeef059a975afe172c0640813d05d2331a71e48224df64d138a44e837b2c84f",
''',
    "migration chain integrity validator",
)
write(path, text)


# Accurate verification scope and final hardening record.
path = "docs/phase-21-tamper-evident-evidence-archive.md"
text = read(path)
text = text.replace(
    "an independently verifiable, machine-encrypted archive and export boundary",
    "a machine-encrypted archive and export boundary that is fully verifiable by the originating HomeServer and externally verifiable by package SHA-256",
)
text = text.replace(
    "independently verifiable export",
    "origin-installation verification and external package-hash verification",
)
section = '''

## Final trust hardening

- Evidence admission is an explicit reviewed table allowlist. A future table is excluded even when its name ends in `_events`, `_receipts`, or `_audit_records` until a code review adds it.
- The seeded policy hash is the canonical SHA-256 of the complete default policy document, and every active policy is recomputed during health checks.
- Health checks verify the complete sequence of verified archive predecessor identities and manifest hashes.
- Archive idempotency, policy binding, predecessor identity, sequence, managed filename/path, encryption mode, actor, and creation timestamp are immutable at the SQLite layer.
- The desktop computes SHA-256 while streaming every exported byte, deletes a mismatched file, and records an export receipt only after the downloaded digest equals the verified package digest.
- `.mgha` contents are fully decryptable and chain-verifiable by the originating HomeServer installation. Other systems can independently verify the exported package bytes against the displayed or recorded SHA-256 without receiving the machine encryption key.
'''
if "## Final trust hardening" not in text:
    text += section
write(path, text)

print("Phase 21 closed allowlist, canonical policy, archive chain, and desktop export hashing hardening applied.")
