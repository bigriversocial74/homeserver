#!/usr/bin/env python3
"""Validate the Phase 5B multi-cloud connection foundation."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        ERRORS.append(f"required multi-connection file is missing: {path}")
        return ""
    return target.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in read(path):
        ERRORS.append(message)


MIGRATION = "database/migrations/0010_multi_cloud_connections.sql"
SERVICE = "crates/homeserver-service/src/cloud_registry.rs"
PAIRING = "crates/homeserver-service/src/cloud_pairing_v2.rs"
APP = "crates/homeserver-service/src/app.rs"
TAURI = "src-tauri/src/cloud.rs"
TAURI_LIB = "src-tauri/src/lib.rs"
UI = "src/cloud-connections.js"
INDEX = "index.html"

for marker in (
    "CREATE TABLE IF NOT EXISTS cloud_connections",
    "credential_key TEXT NOT NULL UNIQUE",
    "UNIQUE (connection_id, idempotency_key)",
    "FOREIGN KEY (connection_id) REFERENCES cloud_connections(connection_id) ON DELETE CASCADE",
    "CREATE TABLE IF NOT EXISTS cloud_connection_events",
    "0010_multi_cloud_connections",
):
    require(MIGRATION, marker, f"multi-connection migration boundary is missing: {marker}")

for marker in (
    'const ALLOWED_PROVIDERS: &[&str] = &["microgifter"]',
    "cloud_connections_snapshot",
    "disconnect_cloud_connection",
    "enqueue_connection_sync",
    "sync_cloud_connection",
    "sync_all_cloud_connections",
    "credential_key",
    "cloud_sync_queue",
    "cloud_sync_receipts",
    "connection_id",
    "tenant_id",
    "site_id",
    "local_only",
    "migrate_legacy_connection",
    "MAX_PENDING_SYNC_OPERATIONS_PER_CONNECTION",
    "validate_receipts",
    "canonical_request",
):
    require(SERVICE, marker, f"multi-connection service boundary is missing: {marker}")

for marker in (
    'installation_id: connection_id',
    'const LEGACY_PAIR_PATH: &str = "/v1/cloud/connections/pair"',
    '"/v1/cloud/connections/pair-v2"',
    "reject_legacy_pairing",
    "credential_key",
    "cloud_installation_id",
    "MicrogifterPairingClient",
):
    require(PAIRING, marker, f"connection-scoped pairing boundary is missing: {marker}")

for marker in (
    "cloud_registry::initialize(&connection)",
    "cloud_registry::run(state.clone(), shutdown.clone())",
    "let registry_router = cloud_registry::router(state.clone()).layer(",
    "cloud_pairing_v2::reject_legacy_pairing",
    ".merge(cloud_pairing_v2::router(state.clone()))",
):
    require(APP, marker, f"multi-connection app integration is missing: {marker}")

for marker in (
    "homeserver_cloud_connections",
    "homeserver_pair_cloud_connection",
    "homeserver_disconnect_cloud_connection",
    "homeserver_sync_cloud_connection",
    "homeserver_sync_all_cloud_connections",
):
    require(TAURI, marker, f"multi-connection Tauri command is missing: {marker}")
    require(TAURI_LIB, f"cloud::{marker}", f"multi-connection command is not registered: {marker}")

require(TAURI, "/v1/cloud/connections/pair-v2", "Tauri does not use connection-scoped pairing v2")

for marker in (
    "Cloud Connection Registry",
    "Pair a Site",
    "HomeServer remains usable with zero cloud connections",
    "homeserver_pair_cloud_connection",
    "homeserver_sync_cloud_connection",
    "homeserver_disconnect_cloud_connection",
):
    require(UI, marker, f"multi-connection Control Center boundary is missing: {marker}")

require(INDEX, "/src/cloud-connections.js", "multi-connection Control Center module is not loaded")
require("package.json", "validate-multi-cloud-connections.py", "multi-connection validation is not part of the frontend gate")

service_runtime = read(SERVICE).split("#[cfg(test)]", 1)[0] + read(PAIRING).split("#[cfg(test)]", 1)[0]
for marker in (
    "commerce.order.create",
    "payment.",
    "claim.",
    "redemption.",
    "ownership.",
    "shell.execute",
):
    if marker in service_runtime:
        ERRORS.append(f"multi-connection foundation contains disallowed domain authority: {marker}")

for marker in ("0.0.0.0", "MG_HOMESERVER_CLOUD_URL"):
    if marker in service_runtime:
        ERRORS.append(f"multi-connection runtime contains a disallowed network boundary: {marker}")

if ERRORS:
    print("Phase 5B multi-cloud connection validation failed:", file=sys.stderr)
    for error in ERRORS:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print("Phase 5B multi-cloud connection boundaries validated.")
