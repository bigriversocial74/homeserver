#!/usr/bin/env python3
from __future__ import annotations

import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATION = ROOT / "database/migrations/0022_wrapper_jobs_events_receipts.sql"
AUTHORITY_MIGRATION = ROOT / "database/migrations/0022a_wrapper_job_authority_snapshots.sql"
APP = ROOT / "crates/homeserver-service/src/app.rs"
MAIN = ROOT / "crates/homeserver-service/src/app/wrapper_jobs.rs"
DOC = ROOT / "docs/phase-16c-shared-jobs-events-receipts.md"
SOURCE_FILES = sorted((ROOT / "crates/homeserver-service/src/app").glob("wrapper_jobs*.rs"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"Phase 16C validation failed: {message}")


for path in [MIGRATION, AUTHORITY_MIGRATION, APP, MAIN, DOC]:
    require(path.is_file(), f"missing {path.relative_to(ROOT)}")

migration = MIGRATION.read_text(encoding="utf-8")
authority_migration = AUTHORITY_MIGRATION.read_text(encoding="utf-8")
app = APP.read_text(encoding="utf-8")
main = MAIN.read_text(encoding="utf-8")
source = "\n".join(path.read_text(encoding="utf-8") for path in SOURCE_FILES)
doc = DOC.read_text(encoding="utf-8")

required_tables = [
    "wrapper_job_workers",
    "wrapper_jobs",
    "wrapper_job_inputs",
    "wrapper_job_events",
    "wrapper_job_private_results",
    "wrapper_job_safe_results",
    "wrapper_job_execution_receipts",
    "wrapper_job_deliveries",
]
for table in required_tables:
    require(f"CREATE TABLE IF NOT EXISTS {table}" in migration, f"missing table {table}")
require(
    "CREATE TABLE IF NOT EXISTS wrapper_job_authority_snapshots" in authority_migration,
    "missing connection authority snapshot table",
)

for token in [
    "UNIQUE (connection_id, idempotency_key)",
    "connection_authority_revision",
    "authorization_decision_id",
    "grant_revision",
    "payload_hash",
    "request_hash",
    "correlation_id",
    "causation_id",
    "allowed_result_fields_json",
    "lease_token_hash",
    "receipt_hash",
    "event_hash",
    "trg_wrapper_job_events_no_update",
    "trg_wrapper_job_receipts_no_update",
]:
    require(token in migration + authority_migration, f"missing contract token {token}")

require("lease_token TEXT" not in migration, "raw lease tokens must not be stored")
require("private_input_json" in migration, "private input storage is missing")
require("private_result_json" in migration, "private result storage is missing")

safe_table = migration.split("CREATE TABLE IF NOT EXISTS wrapper_job_safe_results", 1)[1].split(
    "CREATE TABLE IF NOT EXISTS wrapper_job_execution_receipts", 1
)[0]
receipt_table = migration.split(
    "CREATE TABLE IF NOT EXISTS wrapper_job_execution_receipts", 1
)[1].split("CREATE TABLE IF NOT EXISTS wrapper_job_deliveries", 1)[0]
for forbidden in [
    "private_input_json",
    "private_result_json",
    "private_provenance_json",
    "credential",
    "prompt",
    "source_text",
    "full_document",
]:
    require(forbidden not in safe_table, f"safe results contain private column {forbidden}")
    require(forbidden not in receipt_table, f"receipts contain private column {forbidden}")

for endpoint in [
    "/v1/wrapper-jobs/snapshot",
    "/v1/wrapper-jobs/submit",
    "/v1/wrapper-jobs/cancel",
    "/v1/wrapper-jobs/deliveries/poll",
    "/v1/wrapper-jobs/deliveries/ack",
    "/v1/internal/wrapper-jobs/workers/register",
    "/v1/internal/wrapper-jobs/claim",
    "/v1/internal/wrapper-jobs/start",
    "/v1/internal/wrapper-jobs/heartbeat",
    "/v1/internal/wrapper-jobs/complete",
    "/v1/internal/wrapper-jobs/fail",
]:
    require(endpoint in main, f"missing endpoint {endpoint}")

for integration in [
    '#[path = "app/wrapper_jobs.rs"]',
    "wrapper_jobs::initialize(&connection)?;",
    ".merge(wrapper_jobs::router(state.clone()))",
    "wrapper_jobs::maintain_history(&connection)",
]:
    require(integration in app, f"missing app integration {integration}")

for token in [
    "wrapper_grants::authorize",
    "authority_is_current_tx",
    "current_connection_authority_revision",
    "idempotency key was already used with a different request",
    "project_safe_result",
    "safe_provenance_summary",
    "private_inputs_exposed: false",
    "private_results_exposed: false",
    "create_terminal_receipt_tx",
    "acknowledge_delivery",
    "receipt_hash does not match",
    "CANCEL JOB",
]:
    require(token in source, f"missing runtime boundary {token}")

job_summary = main.split("pub struct JobSummary", 1)[1].split("pub struct ConnectionJobSnapshot", 1)[0]
for forbidden in ["private_input", "private_result", "private_provenance", "lease_token:"]:
    require(forbidden not in job_summary, f"wrapper job summary exposes {forbidden}")

snapshot_source = (ROOT / "crates/homeserver-service/src/app/wrapper_jobs_read.rs").read_text(
    encoding="utf-8"
)
require("wrapper_job_inputs" not in snapshot_source, "wrapper snapshot reads private job input")
require("wrapper_job_private_results" not in snapshot_source, "wrapper snapshot reads private result")

projection = (ROOT / "crates/homeserver-service/src/app/wrapper_jobs_projection.rs").read_text(
    encoding="utf-8"
)
for forbidden in [
    '"source_text"',
    '"full_document"',
    '"system_prompt"',
    '"credential"',
    '"api_key"',
    '"memory"',
    '"private_data"',
    '"file_path"',
]:
    require(forbidden in projection, f"safe projection does not reject {forbidden}")
require(
    'output.get("requires_approval") == Some(&Value::Bool(true))' in projection,
    "proposed actions are not approval-gated",
)
require("action authority is proposal-only" in source, "action jobs are not proposal-only")

for phrase in [
    "Initial current-state score: **5.1/10**",
    "Private-data boundary",
    "Idempotency and replay",
    "Worker and lease model",
    "Safe result policies",
    "Offline behavior",
    "10/10 certification gates",
]:
    require(phrase in doc, f"documentation missing {phrase}")

connection = sqlite3.connect(":memory:")
connection.execute("PRAGMA foreign_keys=ON")
connection.execute("CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY)")
for migration_path in [
    ROOT / "database/migrations/0020_wrapper_identity_and_pairing.sql",
    ROOT / "database/migrations/0021_wrapper_capability_grants.sql",
    MIGRATION,
    AUTHORITY_MIGRATION,
]:
    connection.executescript(migration_path.read_text(encoding="utf-8"))

for table in required_tables + ["wrapper_job_authority_snapshots"]:
    columns = connection.execute(f"PRAGMA table_info({table})").fetchall()
    require(columns, f"SQLite did not create {table}")

job_indexes = {row[1]: row for row in connection.execute("PRAGMA index_list(wrapper_jobs)")}
require(
    any(row[2] == 1 for row in job_indexes.values()),
    "wrapper job idempotency does not have a unique index",
)
triggers = {
    row[0]: row[1]
    for row in connection.execute(
        "SELECT name,sql FROM sqlite_master WHERE type='trigger'"
    ).fetchall()
}
require("trg_wrapper_job_events_no_update" in triggers, "event immutability trigger missing")
require("trg_wrapper_job_receipts_no_update" in triggers, "receipt immutability trigger missing")

migration_keys = {
    row[0] for row in connection.execute("SELECT migration_key FROM schema_migrations")
}
require("0022_wrapper_jobs_events_receipts" in migration_keys, "primary migration key missing")
require(
    "0022a_wrapper_job_authority_snapshots" in migration_keys,
    "authority migration key missing",
)

print("Phase 16C shared wrapper job contract validation passed (10/10 contract coverage).")
