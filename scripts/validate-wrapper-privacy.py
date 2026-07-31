#!/usr/bin/env python3
from __future__ import annotations
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATION = ROOT / "database/migrations/0024_private_knowledge_boundary.sql"
SOURCE = ROOT / "crates/homeserver-service/src/app/wrapper_privacy.rs"
APP = ROOT / "crates/homeserver-service/src/app.rs"
JOBS = ROOT / "crates/homeserver-service/src/app/wrapper_jobs.rs"
SUBMIT = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_submit.rs"
COMPLETE = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_completion.rs"
RECONCILE = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_reconcile.rs"
DELIVERY = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_delivery.rs"
READ = ROOT / "crates/homeserver-service/src/app/wrapper_jobs_read.rs"
DOC = ROOT / "docs/phase-16e-private-knowledge-egress.md"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(f"Phase 16E validation failed: {message}")

migration = MIGRATION.read_text(encoding="utf-8")
source = SOURCE.read_text(encoding="utf-8")
app = APP.read_text(encoding="utf-8")
jobs = JOBS.read_text(encoding="utf-8")
submit = SUBMIT.read_text(encoding="utf-8")
complete = COMPLETE.read_text(encoding="utf-8")
reconcile = RECONCILE.read_text(encoding="utf-8")
delivery = DELIVERY.read_text(encoding="utf-8")
read = READ.read_text(encoding="utf-8")
doc = DOC.read_text(encoding="utf-8")

required_tables = [
    "data_classification_catalog", "private_resource_catalog",
    "private_resource_classifications", "private_resource_selectors",
    "private_selector_resources", "private_resource_aliases",
    "wrapper_job_privacy_bindings", "private_knowledge_access_receipts",
    "egress_decisions", "wrapper_resource_projections", "egress_redactions",
    "egress_approvals", "private_evidence_records", "privacy_boundary_incidents",
    "projection_cache_entries", "deletion_propagation_jobs",
]
for table in required_tables:
    require(f"CREATE TABLE IF NOT EXISTS {table}" in migration, f"missing table {table}")
for phrase in [
    "private_source", "private_derived", "shared_approved", "safe_receipt",
    "wrapper_owned", "security_metadata", "trg_privacy_vault_update",
    "trg_privacy_vault_delete", "0024_private_knowledge_boundary",
]:
    require(phrase in migration, f"missing migration boundary {phrase}")

for integration in [
    '#[path = "app/wrapper_privacy.rs"]',
    "wrapper_privacy::initialize(&connection)?;",
    ".merge(wrapper_privacy::router(state.clone()))",
    "wrapper_privacy::maintain_history(&connection)",
]:
    require(integration in app, f"missing app integration {integration}")
require("use super::wrapper_privacy;" in jobs, "wrapper jobs do not import privacy authority")
for phrase in ["private_selector_id", "output_schema", "remote_model_provider"]:
    require(phrase in jobs, f"job envelope missing {phrase}")
for phrase in ["validate_job_privacy_submission", "bind_job_privacy_tx"]:
    require(phrase in submit, f"job submission is not privacy-bound: {phrase}")
require("evaluate_egress_tx" in complete, "completion does not call egress engine")
require("job_privacy_authority_is_current_tx" in reconcile, "job reconciliation does not enforce selector authority")
require("delivery_egress_is_current_tx" in delivery, "delivery does not recheck egress authority")
require("safe_result_is_visible" in read, "pending/denied projections are visible")

for phrase in [
    "pairing_implies_private_authority", "private_sources_exposed: false",
    "destination_specific_aliases: true", "cross_wrapper_sentinel",
    "credential_material", "private_knowledge_access_receipts",
    "fresh_egress_approval_required", "selector forbids remote model context",
    "source_identifiers_included", "private_source_content_included",
]:
    require(phrase in source or phrase in complete, f"missing runtime boundary {phrase}")

for forbidden in ["knowledge.all", "private_source.read_raw", "wrapper.cross_read", "selector.*"]:
    require(forbidden not in source, f"unsafe private capability present: {forbidden}")

conn = sqlite3.connect(":memory:")
conn.execute("PRAGMA foreign_keys=ON")
conn.executescript("""
CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);
CREATE TABLE wrapper_identities (wrapper_id TEXT PRIMARY KEY);
CREATE TABLE wrapper_connections (connection_id TEXT PRIMARY KEY,wrapper_id TEXT NOT NULL,lifecycle_state TEXT NOT NULL,grant_revision INTEGER NOT NULL DEFAULT 0,FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id));
CREATE TABLE wrapper_capability_catalog (capability_key TEXT PRIMARY KEY);
CREATE TABLE wrapper_capability_grants (grant_id TEXT PRIMARY KEY,wrapper_id TEXT NOT NULL,connection_id TEXT NOT NULL,capability_key TEXT NOT NULL,grant_revision INTEGER NOT NULL,state TEXT NOT NULL,expires_at_utc TEXT NOT NULL,FOREIGN KEY(wrapper_id) REFERENCES wrapper_identities(wrapper_id),FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id),FOREIGN KEY(capability_key) REFERENCES wrapper_capability_catalog(capability_key));
CREATE TABLE homeserver_agents (agent_id TEXT PRIMARY KEY,state TEXT NOT NULL,revision INTEGER NOT NULL,expires_at_utc TEXT NOT NULL);
CREATE TABLE wrapper_jobs (job_id TEXT PRIMARY KEY,connection_id TEXT NOT NULL,grant_id TEXT NOT NULL,capability_key TEXT NOT NULL,state TEXT NOT NULL,FOREIGN KEY(connection_id) REFERENCES wrapper_connections(connection_id),FOREIGN KEY(grant_id) REFERENCES wrapper_capability_grants(grant_id));
CREATE TABLE wrapper_job_deliveries (delivery_id TEXT PRIMARY KEY,job_id TEXT NOT NULL,state TEXT NOT NULL,updated_at_utc TEXT NOT NULL,FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id));
CREATE TABLE wrapper_job_safe_results (job_id TEXT PRIMARY KEY,result_policy TEXT NOT NULL,safe_result_json TEXT NOT NULL,safe_result_hash TEXT NOT NULL,provenance_summary_json TEXT NOT NULL,provenance_summary_hash TEXT NOT NULL,filter_version TEXT NOT NULL,result_bytes INTEGER NOT NULL,created_at_utc TEXT NOT NULL,FOREIGN KEY(job_id) REFERENCES wrapper_jobs(job_id));
CREATE TABLE vault_documents (document_id TEXT PRIMARY KEY,title TEXT NOT NULL,sha256 TEXT NOT NULL,state TEXT NOT NULL,created_at_utc TEXT NOT NULL,updated_at_utc TEXT NOT NULL);
""")
conn.executescript(migration)
tables = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'")}
require(set(required_tables).issubset(tables), "SQLite did not create all privacy tables")
classes = {row[0] for row in conn.execute("SELECT class_key FROM data_classification_catalog")}
require(classes == {"secret","private_source","private_derived","private_selector","shared_approved","wrapper_owned","public","safe_receipt","security_metadata"}, "data classes are not exact")
conn.execute("INSERT INTO vault_documents VALUES ('doc-1','Local Secret','a','indexed','2026-01-01','2026-01-01')")
row = conn.execute("SELECT resource_id,state,resource_revision FROM private_resource_catalog").fetchone()
require(row == ("vault:doc-1","active",1), "vault insert did not create private resource")
conn.execute("UPDATE vault_documents SET sha256='b',updated_at_utc='2026-01-02' WHERE document_id='doc-1'")
require(conn.execute("SELECT resource_revision FROM private_resource_catalog").fetchone()[0] == 2, "vault revision did not propagate")
conn.execute("DELETE FROM vault_documents WHERE document_id='doc-1'")
require(conn.execute("SELECT state FROM private_resource_catalog").fetchone()[0] == "deleted", "vault deletion did not tombstone resource")
require(conn.execute("SELECT state FROM deletion_propagation_jobs").fetchone()[0] == "queued", "deletion propagation was not queued")

for phrase in [
    "Initial current-state score: **7.1/10**", "Authority model", "Data classifications",
    "Private selectors", "Local-only knowledge access", "Result-egress pipeline",
    "Egress approval", "Revocation and deletion", "10/10 certification gates",
]:
    require(phrase in doc, f"missing documentation section {phrase}")

print("Phase 16E private knowledge and egress validation passed.")
