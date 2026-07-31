#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def require(source: str, markers: list[str], label: str) -> None:
    for marker in markers:
        if marker not in source:
            raise SystemExit(f"Phase 21 validation failed: missing {label}: {marker}")


def forbid(source: str, markers: list[str], label: str) -> None:
    lowered = source.lower()
    for marker in markers:
        if marker.lower() in lowered:
            raise SystemExit(f"Phase 21 validation failed: forbidden {label}: {marker}")


migration = read("database/migrations/0029_tamper_evident_evidence_archive.sql")
service = read("crates/homeserver-service/src/evidence_archive.rs")
app = read("crates/homeserver-service/src/app.rs")
main = read("crates/homeserver-service/src/main.rs")
tests = read("crates/homeserver-service/tests/phase21_evidence_archive_contract.rs")
tauri = read("src-tauri/src/runtime.rs")
tauri_lib = read("src-tauri/src/lib.rs")
frontend = read("src/agent-runtime-control-center.js")
frontend_css = read("src/agent-runtime-control-center.css")
package = read("package.json")
workflow = read(".github/workflows/phase21-evidence-archive.yml")
docs = read("docs/phase-21-tamper-evident-evidence-archive.md")

require(
    migration,
    [
        "CREATE TABLE IF NOT EXISTS evidence_archive_policies",
        "CREATE TABLE IF NOT EXISTS evidence_archives",
        "CREATE TABLE IF NOT EXISTS evidence_archive_storage",
        "CREATE TABLE IF NOT EXISTS evidence_archive_members",
        "CREATE TABLE IF NOT EXISTS evidence_archive_exports",
        "CREATE TABLE IF NOT EXISTS evidence_archive_events",
        "trg_evidence_archive_policies_no_update",
        "trg_evidence_archives_authority_immutable",
        "trg_evidence_archives_terminal_immutable",
        "faeef059a975afe172c0640813d05d2331a71e48224df64d138a44e837b2c84f",
        "trg_evidence_archive_members_no_delete",
        "trg_evidence_archive_exports_no_delete",
        "trg_evidence_archive_events_no_delete",
        "export_verified_before_prune",
        "0029_tamper_evident_evidence_archive",
    ],
    "migration contract",
)
forbid(
    migration,
    [
        "DELETE FROM agent_",
        "DELETE FROM wrapper_",
        "DELETE FROM model_inference",
        "DELETE FROM private_knowledge",
        "ON DELETE CASCADE",
    ],
    "source-evidence deletion",
)

require(
    service,
    [
        'const PACKAGE_MAGIC: &[u8; 8] = b"MGHEAR01"',
        "Aes256Gcm",
        "MicrogifterHomeServerEvidenceArchive:v1",
        "evidence_archive_members",
        "evidence_archive_storage",
        "is_allowed_evidence_table",
        "REVIEWED_EVIDENCE_TABLES",
        '"wrapper_events"',
        '"wrapper_authorization_receipts"',
        '"wrapper_job_execution_receipts"',
        '"agent_action_receipts"',
        '"private_knowledge_access_receipts"',
        '"agent_runtime_audit_records"',
        '"agent_supervised_compensation_receipts"',
        '"agent_schedule_event_inbox"',
        '"model_inference_receipts"',
        '"model_inference_private_results"',
        '"future_private_events"',
        '"future_secret_receipts"',
        "source_evidence_deleted: false",
        "private_content_exposed: false",
        "private_content_included: false",
        "hash_chain",
        "manifest_sha256",
        "records_sha256",
        "chain_root_hash",
        "write_atomic",
        "verify_package_file",
        "recover_interrupted_archives",
        "create_automatic_if_due",
        "hash_policy",
        "verify_archive_chain",
        "EXISTS (SELECT 1 FROM evidence_archive_exports",
        "state='exported'",
        "state='pruned'",
        '"/v1/evidence-archives"',
        '"/v1/evidence-archives/{archive_id}/package"',
    ],
    "archive service boundary",
)
forbid(
    service,
    [
        'table.ends_with("_events")',
        'table.ends_with("_receipts")',
        'table.ends_with("_audit_records")',
        "prompt_text",
        "output_text",
        "private_result_json",
        "api_key",
        "authorization: bearer",
        "0.0.0.0",
    ],
    "private content or remote exposure",
)

require(
    app,
    [
        "evidence_archive::initialize",
        "evidence_archive::router",
        "evidence_archive::create_automatic_if_due",
    ],
    "service lifecycle integration",
)
require(main, ["mod evidence_archive;", "evidence_archive::health_check"], "service registration")
require(
    tests,
    [
        "default_policy_is_bounded_and_machine_local",
        "archive_policy_members_exports_and_events_are_immutable",
        "storage_retention_requires_exported_state_and_cannot_reverse_pruning",
        "migration_registers_archive_contract_once",
    ],
    "native mutation tests",
)
require(
    tauri,
    [
        "homeserver_evidence_archives",
        "homeserver_update_evidence_archive_policy",
        "homeserver_create_evidence_archive",
        "homeserver_verify_evidence_archive",
    ],
    "trusted Tauri commands",
)
require(
    tauri_lib,
    [
        "homeserver_export_evidence_archive",
        "MAX_EVIDENCE_ARCHIVE_BYTES",
        "application/vnd.microgifter.homeserver-evidence-archive",
        "runtime::homeserver_evidence_archives",
        "Sha256::new()",
        "package_hasher.update(&chunk)",
        "downloaded_package_sha256 != expected_package_sha256",
        "Evidence archive export hash verification failed; the incomplete file was removed.",
    ],
    "bounded desktop export",
)
require(
    frontend,
    [
        'invoke("homeserver_evidence_archives")',
        'invoke("homeserver_create_evidence_archive"',
        'invoke("homeserver_verify_evidence_archive"',
        'invoke("homeserver_export_evidence_archive"',
        "Tamper-evident evidence archives",
        "private_content_exposed",
        "source_evidence_deleted",
    ],
    "Control Center evidence archive UI",
)
forbid(
    frontend,
    ["storage_path", "prompt_text", "output_text", "source_fields", "MutationObserver"],
    "unsafe archive UI",
)
require(frontend_css, ["runtime-evidence-archive", "runtime-archive-grid"], "archive UI styles")
require(package, ["validate-evidence-archive.py"], "permanent validator registration")
require(workflow, ["Phase 21 Evidence Archive", "phase21_evidence_archive_contract", "-D warnings"], "permanent workflow")
require(docs, ["Initial audit: **4.4/10**", "Source evidence is never deleted", "Export-gated retention"], "Phase 21 documentation")

print("Phase 21 tamper-evident evidence archive contract passed.")
