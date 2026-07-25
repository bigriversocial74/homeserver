#!/usr/bin/env python3
"""Fail CI when audited HomeServer security boundaries regress."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def text(path: str) -> str:
    file_path = ROOT / path
    if not file_path.is_file():
        ERRORS.append(f"required file is missing: {path}")
        return ""
    return file_path.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in text(path):
        ERRORS.append(message)


def forbid(path: str, marker: str, message: str) -> None:
    if marker in text(path):
        ERRORS.append(message)


# Release metadata must remain synchronized.
cargo = text("Cargo.toml")
match = re.search(r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"', cargo)
if not match:
    ERRORS.append("workspace release version is missing")
else:
    version = match.group(1)
    for path in ("package.json", "src-tauri/tauri.conf.json"):
        try:
            value = json.loads(text(path)).get("version")
        except json.JSONDecodeError as error:
            ERRORS.append(f"invalid JSON in {path}: {error}")
            continue
        if value != version:
            ERRORS.append(f"{path} version {value!r} does not match workspace {version!r}")

# Machine secrets must be machine-scoped on Windows and fail closed elsewhere.
for path in (
    "crates/machine-keyring/src/lib.rs",
    "crates/homeserver-service/src/backup_key.rs",
):
    require(path, "CRYPTPROTECT_LOCAL_MACHINE", f"{path} is not using machine-scoped DPAPI")
    forbid(path, "#[cfg(not(windows))]\nfn protect(value", f"{path} may expose a plaintext non-Windows protector")

forbid(
    "crates/machine-keyring/src/lib.rs",
    "#[cfg(not(windows))]\nfn protect(value: &[u8]",
    "machine keyring non-Windows protection must fail closed",
)
forbid(
    "crates/homeserver-service/src/backup_key.rs",
    "#[cfg(not(windows))]\nfn protect(value: &[u8]",
    "backup key non-Windows protection must fail closed",
)

# Every merged loopback route must be wrapped by the same anti-browser boundary.
require(
    "crates/homeserver-service/src/app.rs",
    "http::secure(http::router(state.clone()).merge(cloud_connector::router(state)))",
    "the fully merged local API router is not wrapped by the security layer",
)
for marker in (
    "LOCAL_CLIENT_HEADER",
    "LOCAL_CLIENT_VALUE",
    "headers.contains_key(header::ORIGIN)",
    'headers.contains_key("sec-fetch-site")',
    "LOCAL_API_HOST",
    'path.starts_with("/v1/")',
    'headers.contains_key("x-forwarded-host")',
    "StatusCode::FORBIDDEN",
):
    require("crates/homeserver-service/src/http.rs", marker, f"local API boundary is missing {marker}")
require(
    "src-tauri/src/lib.rs",
    "default_headers",
    "Control Center HTTP client does not install trusted local headers",
)
forbid(
    "src-tauri/tauri.conf.json",
    "unsafe-inline",
    "Control Center CSP still permits unsafe inline content",
)

# Recovery packages and restore activation must be bounded and fail-safe.
for marker in (
    "entry.header().entry_type().is_file()",
    "MAX_DATABASE_BYTES + 1",
    "archive contains duplicate",
    "archive contains too many entries",
):
    require("crates/homeserver-service/src/backup.rs", marker, f"recovery extraction is missing guard: {marker}")
for marker in ("activate_staged_database", "reactivate the previous HomeServer database", "delete_restore_request"):
    require("crates/homeserver-service/src/backup.rs", marker, f"restore fail-safe is missing {marker}")

# SQLite durability must prefer committed data over throughput for the local authority.
require(
    "crates/homeserver-service/src/database.rs",
    'pragma_update(None, "synchronous", "FULL")',
    "SQLite synchronous mode is not FULL",
)

# Cloud synchronization must not grow without bounds or trust unbounded replies.
for marker in (
    "MAX_CLOUD_RESPONSE_BYTES",
    "MAX_PENDING_SYNC_OPERATIONS",
    "MAX_SYNC_ATTEMPTS",
    "maintain_sync_history",
    "Microgifter response exceeds the HomeServer size limit",
):
    require("crates/homeserver-service/src/cloud_connector.rs", marker, f"cloud boundary is missing {marker}")

# The updater may operate only within canonical managed directories.
for path in (
    "crates/homeserver-service/src/update_apply.rs",
    "crates/homeserver-updater/src/main.rs",
):
    require(path, "canonicalize", f"{path} does not canonicalize update paths")
    require(path, "starts_with", f"{path} does not enforce managed path containment")

# Per-machine data must be access controlled during installation.
for marker in ("icacls", "S-1-5-18", "S-1-5-32-544", "/inheritance:r"):
    require("src-tauri/windows/hooks.nsh", marker, f"installer ACL hardening is missing {marker}")

# Production logging and synchronization histories must have retention.
require("crates/homeserver-service/src/main.rs", "prune_old_service_logs", "service log retention is missing")

# Supply-chain workflows must be read-only and every external action pinned by SHA.
workflow_dir = ROOT / ".github" / "workflows"
obsolete = {
    "audit-source-snapshot.yml",
    "cloud-connector-compile-fix.yml",
    "verified-v012-installer.yml",
}
for name in obsolete:
    if (workflow_dir / name).exists():
        ERRORS.append(f"obsolete or temporary workflow remains: {name}")

uses_pattern = re.compile(r"^\s*-?\s*uses:\s*([^\s#]+)@([^\s#]+)", re.MULTILINE)
sha_pattern = re.compile(r"^[0-9a-f]{40}$")
for workflow in sorted(workflow_dir.glob("*.yml")):
    content = workflow.read_text(encoding="utf-8")
    if re.search(r"(?m)^\s*contents:\s*write\s*$", content):
        ERRORS.append(f"workflow has unnecessary contents write permission: {workflow.name}")
    for action, ref in uses_pattern.findall(content):
        if action.startswith("./"):
            continue
        if not sha_pattern.fullmatch(ref):
            ERRORS.append(f"workflow action is not pinned by full SHA: {workflow.name}: {action}@{ref}")

# Dependency audit and automated update coverage are mandatory release gates.
quality = text(".github/workflows/phase-1-foundation.yml")
for marker in ("cargo audit", "npm audit --audit-level=high", "verify-installer-release.ps1"):
    if marker not in quality:
        ERRORS.append(f"production workflow is missing release gate: {marker}")
if not (ROOT / ".github" / "dependabot.yml").is_file():
    ERRORS.append("Dependabot configuration is missing")

if ERRORS:
    print("HomeServer security-boundary validation failed:", file=sys.stderr)
    for error in ERRORS:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print("HomeServer security-boundary validation passed.")
