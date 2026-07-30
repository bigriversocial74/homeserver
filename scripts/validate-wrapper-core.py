#!/usr/bin/env python3
"""Permanent Phase 16A multi-wrapper core contract validator."""

from __future__ import annotations

import sqlite3
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MIGRATION = ROOT / "database/migrations/0020_wrapper_identity_and_pairing.sql"
MODULE = ROOT / "crates/homeserver-service/src/app/wrapper_core.rs"
APP = ROOT / "crates/homeserver-service/src/app.rs"
PACKAGE = ROOT / "package.json"
DOC = ROOT / "docs/phase-16a-multi-wrapper-core-v1.md"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def require_text(path: Path, values: list[str]) -> str:
    require(path.is_file(), f"missing required file: {path.relative_to(ROOT)}")
    text = path.read_text(encoding="utf-8")
    for value in values:
        require(value in text, f"{path.relative_to(ROOT)} is missing required contract text: {value}")
    return text


def validate_migration() -> None:
    sql = require_text(
        MIGRATION,
        [
            "CREATE TABLE IF NOT EXISTS wrapper_identities",
            "CREATE TABLE IF NOT EXISTS wrapper_connections",
            "CREATE TABLE IF NOT EXISTS wrapper_devices",
            "CREATE TABLE IF NOT EXISTS wrapper_pairing_attempts",
            "CREATE TABLE IF NOT EXISTS wrapper_credential_references",
            "CREATE TABLE IF NOT EXISTS wrapper_events",
            "0020_wrapper_identity_and_pairing",
        ],
    )
    forbidden = [
        "credential_secret",
        "bearer_token TEXT",
        "private_key TEXT",
        "pairing_code TEXT",
        "prompt TEXT",
        "document_content",
    ]
    lowered = sql.lower()
    for value in forbidden:
        require(value.lower() not in lowered, f"migration stores forbidden secret/private field: {value}")

    with tempfile.TemporaryDirectory() as directory:
        database = Path(directory) / "wrapper-core.sqlite"
        connection = sqlite3.connect(database)
        connection.executescript(
            """
            PRAGMA foreign_keys=ON;
            CREATE TABLE schema_migrations (
              migration_key TEXT PRIMARY KEY,
              applied_at_utc TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE homeserver_settings (
              setting_key TEXT PRIMARY KEY,
              setting_value TEXT NOT NULL
            );
            CREATE TABLE cloud_connections (
              connection_id TEXT PRIMARY KEY,
              provider_key TEXT NOT NULL,
              display_name TEXT NOT NULL,
              cloud_base_url TEXT NOT NULL,
              tenant_id TEXT,
              site_id TEXT,
              device_id TEXT NOT NULL,
              public_key_base64 TEXT NOT NULL,
              credential_key TEXT NOT NULL UNIQUE,
              state TEXT NOT NULL,
              scopes_json TEXT NOT NULL DEFAULT '[]',
              is_default INTEGER NOT NULL DEFAULT 0,
              paired_at_utc TEXT NOT NULL,
              last_success_utc TEXT,
              last_error TEXT,
              created_at_utc TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
              updated_at_utc TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            """
        )
        connection.executescript(sql)
        connection.executescript(sql)
        tables = {
            row[0]
            for row in connection.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            )
        }
        for table in {
            "wrapper_identities",
            "wrapper_connections",
            "wrapper_devices",
            "wrapper_pairing_attempts",
            "wrapper_credential_references",
            "wrapper_events",
        }:
            require(table in tables, f"migration failed to create {table}")
        count = connection.execute(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key='0020_wrapper_identity_and_pairing'"
        ).fetchone()[0]
        require(count == 1, "migration key must be registered exactly once")
        connection.close()


def validate_runtime() -> None:
    module = require_text(
        MODULE,
        [
            "pub fn initialize(connection: &Connection)",
            "backfill_legacy_connections",
            "wrapper.pairing.started",
            "wrapper.pairing.completed",
            "wrapper.connection.revoked",
            '"/v1/wrappers"',
            '"/v1/wrappers/register"',
            '"/v1/wrappers/pairing/start"',
            '"/v1/wrappers/pairing/complete"',
            '"/v1/wrappers/connections/revoke"',
            "DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES)",
            "REVOKE WRAPPER",
            "delete_vault_credential",
            "request_hash",
            "requested_capabilities",
            "local_only: true",
        ],
    )
    require("pairing_code" not in module, "runtime must not retain pairing codes")
    require("credential_reference" in module, "runtime must use credential references")
    require("wrapper.wrapper_key == legacy.provider_key" in module, "pairing completion must bind provider identity")
    require("normalize_origin_for_compare" in module, "pairing completion must bind the approved origin")

    require_text(
        APP,
        [
            '#[path = "app/wrapper_core.rs"]',
            "wrapper_core::initialize(&connection)?;",
            ".merge(wrapper_core::router(state.clone()))",
            "wrapper_core::maintain_history(&connection)",
        ],
    )


def validate_delivery() -> None:
    require_text(PACKAGE, ["validate-wrapper-core.py"])
    require_text(
        DOC,
        [
            "RSS-POD is one authorized wrapper",
            "Microgifter is one authorized wrapper",
            "Secrets remain in the operating-system credential vault",
            "Migration `0020_wrapper_identity_and_pairing.sql`",
            "10/10",
        ],
    )


def main() -> int:
    validate_migration()
    validate_runtime()
    validate_delivery()
    print("Phase 16A multi-wrapper core contract validation passed.")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except AssertionError as error:
        print(f"Phase 16A validation failed: {error}", file=sys.stderr)
        raise SystemExit(1)
