#!/usr/bin/env python3
"""Validate the coordinated HomeServer and Microgifter cloud connector contract."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path


def read(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        raise RuntimeError(f"unable to read {path}: {exc}") from exc


def require(condition: bool, message: str, failures: list[str]) -> None:
    if not condition:
        failures.append(message)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--homeserver-root", default=".")
    parser.add_argument("--cloud-root", required=True)
    args = parser.parse_args()

    homeserver = Path(args.homeserver_root).resolve()
    cloud = Path(args.cloud_root).resolve()
    failures: list[str] = []

    connector = read(homeserver / "crates/homeserver-service/src/cloud_connector.rs")
    local_migration = read(homeserver / "database/migrations/0004_cloud_pairing_sync.sql")
    control_bridge = read(homeserver / "src-tauri/src/cloud.rs")
    control_ui = read(homeserver / "src/main.js")

    cloud_foundation = read(cloud / "api/homeserver/_homeserver.php")
    cloud_pair = read(cloud / "api/homeserver/pair.php")
    cloud_status = read(cloud / "api/homeserver/status.php")
    cloud_sync = read(cloud / "api/homeserver/sync.php")
    cloud_migration = read(cloud / "database/20260724_homeserver_cloud_pairing_sync_v1.sql")

    path_contracts = {
        "/api/homeserver/pair.php": cloud_pair,
        "/api/homeserver/status.php": cloud_status,
        "/api/homeserver/sync.php": cloud_sync,
    }
    for path, cloud_source in path_contracts.items():
        require(path in connector, f"HomeServer client path is missing: {path}", failures)
        require("mg_require_method" in cloud_source, f"Cloud endpoint lacks a method gate: {path}", failures)

    allowed_operations = [
        "device.heartbeat",
        "local.settings.snapshot",
        "cache.refresh.request",
    ]
    for operation in allowed_operations:
        require(operation in connector, f"HomeServer allowlist is missing {operation}", failures)
        require(operation in cloud_foundation, f"Cloud disposition is missing {operation}", failures)

    rejected_prefixes = ["commerce.", "payment.", "claim.", "redemption.", "ownership."]
    for prefix in rejected_prefixes:
        require(prefix in cloud_foundation, f"Cloud authority rejection is missing {prefix}", failures)
        require(prefix not in re.findall(r'"([a-z.]+)"', connector.split("ALLOWED_LOCAL_OPERATIONS", 1)[1].split("];", 1)[0]),
                f"HomeServer allowlist unexpectedly contains {prefix}", failures)

    canonical_body = '{"x":1}'
    expected_hash = "5041bf1f713df204784353e82f6a4a535931cb64f1f4b4a5aeaffcb720918b22"
    require(hashlib.sha256(canonical_body.encode()).hexdigest() == expected_hash,
            "canonical SHA-256 vector changed", failures)
    require(expected_hash in connector, "HomeServer signature test vector is missing", failures)
    require(expected_hash in read(cloud / "scripts/validate_homeserver_pairing_sync_v1.php"),
            "Cloud signature test vector is missing", failures)
    require("METHOD" not in connector or "canonical_request" in connector,
            "HomeServer canonical request builder is missing", failures)
    require("sodium_crypto_sign_verify_detached" in cloud_foundation,
            "Cloud Ed25519 verification is missing", failures)
    require("X-MG-Nonce" in cloud_foundation and "X-MG-Signature" in cloud_foundation,
            "Cloud signed-request headers are incomplete", failures)
    require("X-MG-Nonce" in connector and "X-MG-Signature" in connector,
            "HomeServer signed-request headers are incomplete", failures)

    require("0004_cloud_pairing_sync" in local_migration,
            "HomeServer SQLite migration key is missing", failures)
    require("cloud_connection" in local_migration and "sync_receipts" in local_migration,
            "HomeServer SQLite connector tables are incomplete", failures)
    for table in [
        "homeserver_devices",
        "homeserver_pairing_codes",
        "homeserver_request_nonces",
        "homeserver_sync_receipts",
    ]:
        require(table in cloud_migration, f"Cloud migration is missing {table}", failures)

    for command in [
        "homeserver_cloud_status",
        "homeserver_pair_cloud",
        "homeserver_disconnect_cloud",
        "homeserver_cloud_vault_self_test",
        "homeserver_enqueue_cloud_sync",
        "homeserver_sync_cloud",
    ]:
        require(command in control_bridge, f"Control Center bridge is missing {command}", failures)
        require(command in control_ui, f"Control Center UI is missing {command}", failures)

    require("keyring" in connector and "zeroize" in connector,
            "HomeServer credential-vault protection is incomplete", failures)
    require("token_hash" in cloud_migration and "device_token" not in cloud_migration,
            "Cloud migration must store only hashed device tokens", failures)
    require("homeserver_request_nonces" in cloud_foundation,
            "Cloud replay-protection persistence is missing", failures)
    require("idempotency_conflict" in cloud_sync and "FOR UPDATE" in cloud_sync,
            "Cloud idempotency conflict handling is incomplete", failures)

    if failures:
        print("Cloud connector contract validation failed:", file=sys.stderr)
        for failure in failures:
            print(f"- {failure}", file=sys.stderr)
        return 1

    print("HomeServer and Microgifter cloud connector contract is aligned.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
