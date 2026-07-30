#!/usr/bin/env python3
from __future__ import annotations

import json
import sqlite3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATION = ROOT / "database/migrations/0021_wrapper_capability_grants.sql"
RUNTIME_DIR = ROOT / "crates/homeserver-service/src/app"
RUNTIME = RUNTIME_DIR / "wrapper_grants.rs"
APP = ROOT / "crates/homeserver-service/src/app.rs"
DOC = ROOT / "docs/phase-16b-wrapper-capability-grants.md"

REQUIRED_TABLES = {
    "wrapper_capability_catalog",
    "wrapper_capability_grants",
    "wrapper_dataset_scopes",
    "wrapper_resource_limits",
    "wrapper_grant_approvals",
    "wrapper_bridge_grants",
    "wrapper_grant_usage_windows",
    "wrapper_grant_revocation_fences",
    "wrapper_grant_events",
    "wrapper_authorization_receipts",
}

REQUIRED_CAPABILITIES = {
    "wrapper.status.read",
    "settings.read",
    "settings.update",
    "knowledge.search",
    "knowledge.result.read",
    "model.inference.request",
    "agent.job.propose",
    "agent.job.read",
    "action.propose",
    "receipt.read",
}

FORBIDDEN_CAPABILITIES = {
    "admin",
    "knowledge.all",
    "tools.all",
    "agent.execute_any",
    "cross_wrapper.read",
}

REQUIRED_ENDPOINTS = {
    "/v1/wrapper-grants",
    "/v1/wrapper-grants/create",
    "/v1/wrapper-grants/rotate",
    "/v1/wrapper-grants/revoke",
    "/v1/wrapper-grants/approvals/request",
    "/v1/wrapper-grants/approvals/decide",
    "/v1/wrapper-grants/authorize",
    "/v1/wrapper-bridges/create",
    "/v1/wrapper-bridges/revoke",
    "/v1/wrapper-bridges/authorize",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SystemExit(message)


def validate_sql() -> None:
    sql = MIGRATION.read_text(encoding="utf-8")
    database = sqlite3.connect(":memory:")
    database.execute("PRAGMA foreign_keys=ON")
    database.executescript(
        """
        CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);
        CREATE TABLE wrapper_identities (
          wrapper_id TEXT PRIMARY KEY,
          state TEXT NOT NULL DEFAULT 'active'
        );
        CREATE TABLE wrapper_connections (
          connection_id TEXT PRIMARY KEY,
          wrapper_id TEXT NOT NULL,
          grant_revision INTEGER NOT NULL DEFAULT 0,
          lifecycle_state TEXT NOT NULL DEFAULT 'active',
          updated_at_utc TEXT NOT NULL DEFAULT '',
          FOREIGN KEY (wrapper_id) REFERENCES wrapper_identities(wrapper_id)
        );
        """
    )
    database.executescript(sql)

    tables = {
        row[0]
        for row in database.execute(
            "SELECT name FROM sqlite_master WHERE type='table'"
        )
    }
    require(REQUIRED_TABLES <= tables, f"missing Phase 16B tables: {REQUIRED_TABLES - tables}")

    capabilities = {
        row[0]
        for row in database.execute(
            "SELECT capability_key FROM wrapper_capability_catalog"
        )
    }
    require(REQUIRED_CAPABILITIES <= capabilities, "capability catalog is incomplete")
    require(not (FORBIDDEN_CAPABILITIES & capabilities), "forbidden broad capability is seeded")
    require(
        not any(value.endswith(".all") for value in capabilities),
        "wildcard capability is seeded",
    )

    bridge_sql = database.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='wrapper_bridge_grants'"
    ).fetchone()[0]
    require("source_wrapper_id <> target_wrapper_id" in bridge_sql, "bridge isolation check is missing")

    grant_sql = database.execute(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='wrapper_capability_grants'"
    ).fetchone()[0]
    require("expires_at_utc TEXT NOT NULL" in grant_sql, "grant expiration is not mandatory")
    require("connection_id TEXT NOT NULL" in grant_sql, "grant is not connection-bound")


def validate_runtime() -> None:
    runtime_files = sorted(RUNTIME_DIR.glob("wrapper_grants*.rs"))
    require(runtime_files, "wrapper grant runtime files are missing")
    source = "\n".join(path.read_text(encoding="utf-8") for path in runtime_files)
    app = APP.read_text(encoding="utf-8")
    documentation = DOC.read_text(encoding="utf-8")

    for endpoint in REQUIRED_ENDPOINTS:
        require(endpoint in source, f"missing endpoint {endpoint}")

    for token in [
        "pairing_implies_authority: false",
        "broad or administrative capability keys are forbidden",
        "unscoped or wildcard authority is forbidden",
        "approval mode cannot weaken",
        "grant request rate limit exceeded",
        "grant daily token limit exceeded",
        "wrapper_authorization_receipts",
        "wrapper_grant_revocation_fences",
        "REVOKE GRANT",
        "REVOKE BRIDGE",
        "per_request",
        "source.wrapper_id != target.wrapper_id",
    ]:
        require(token in source, f"missing runtime boundary: {token}")

    require('mod wrapper_grants;' in app, "wrapper grant module is not registered")
    require(
        "wrapper_grants::initialize(&connection)?" in app,
        "wrapper grant migration is not initialized",
    )
    require(
        ".merge(wrapper_grants::router(state.clone()))" in app,
        "wrapper grant routes are not mounted",
    )
    require(
        "wrapper_grants::maintain_history(&connection)" in app,
        "wrapper grant retention is not scheduled",
    )

    for heading in [
        "Initial score: 4.6/10",
        "Final target: 10/10",
        "Pairing grants zero authority",
        "Cross-wrapper bridge policy",
        "Revocation and queued-work fences",
        "Security test matrix",
    ]:
        require(heading in documentation, f"missing design evidence: {heading}")


def main() -> None:
    for path in [MIGRATION, RUNTIME, APP, DOC]:
        require(path.exists(), f"missing required file: {path.relative_to(ROOT)}")
    validate_sql()
    validate_runtime()
    print("Phase 16B wrapper capability-grant boundaries validated.")


if __name__ == "__main__":
    main()
