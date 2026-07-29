#!/usr/bin/env python3
"""Fail CI when audited HomeServer security boundaries regress."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

from validation_support import (
    base_router_is_secured,
    merged_value_is_secured,
    router_component_is_secured,
)

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

# Every merged loopback route must remain inside the same anti-browser boundary.
app_source = text("crates/homeserver-service/src/app.rs")
if not base_router_is_secured(app_source):
    ERRORS.append("the base local API router is not wrapped by http::secure")

for component in (
    "activity",
    "cloud_connector",
    "cloud_pairing_v2",
    "microgifter_connection",
    "pod_provider_runtime",
    "knowledge_vault",
    "model_center",
    "semantic_vault",
    "operational_data",
    "review_intelligence",
    "agent_runtime",
    "mcp_runtime",
):
    if not router_component_is_secured(app_source, component):
        ERRORS.append(
            f"the fully merged local API router is missing secured component: {component}::router"
        )

if not merged_value_is_secured(app_source, "registry_router"):
    ERRORS.append("the cloud registry router is not merged inside the secured local API")

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

# Local model management must remain on a fixed, non-redirecting loopback boundary.
for marker in (
    'const OLLAMA_API_BASE: &str = "http://127.0.0.1:11434"',
    "redirect(Policy::none())",
    "approved_model(&request.model)",
    "MAX_PULL_STREAM_BYTES",
    "model_operations",
):
    require("crates/homeserver-service/src/model_center.rs", marker, f"Model Center boundary is missing {marker}")
for marker in (
    "MAX_EMBED_INPUTS",
    "MAX_EMBED_TOTAL_CHARS",
    "validate_embedding_model(&model)",
    "configured_embedding_model_from_connection",
    'post(format!("{OLLAMA_API_BASE}/api/embed"))',
):
    require(
        "crates/homeserver-service/src/model_center.rs",
        marker,
        f"Model Center semantic embedding boundary is missing {marker}",
    )

for marker in (
    "0007_semantic_vault.sql",
    "MAX_SEMANTIC_CHARS_PER_DOCUMENT",
    "MAX_CHUNKS_PER_DOCUMENT",
    "MAX_SEARCH_CHUNKS",
    "cosine_similarity",
    "local_only: true",
    "vault_semantic_operations",
):
    require(
        "crates/homeserver-service/src/semantic_vault.rs",
        marker,
        f"semantic Knowledge Vault boundary is missing {marker}",
    )
for forbidden in ("https://", "0.0.0.0", "cloud_connector"):
    forbid(
        "crates/homeserver-service/src/semantic_vault.rs",
        forbidden,
        f"semantic Knowledge Vault contains a disallowed external boundary: {forbidden}",
    )

for forbidden in ("https://ollama.com/api", "0.0.0.0:11434", "OLLAMA_HOST"):
    forbid(
        "crates/homeserver-service/src/model_center.rs",
        forbidden,
        f"Model Center contains a disallowed runtime boundary: {forbidden}",
    )
forbid(
    "src-tauri/tauri.conf.json",
    "unsafe-inline",
    "Control Center CSP still permits unsafe inline content",
)

# MCP must remain fixed-loopback, client-scoped, request-only, and audited.
for marker in (
    'const MCP_ENDPOINT: &str = "http://127.0.0.1:47831/mcp"',
    'const MAX_MCP_BODY_BYTES: usize = 128 * 1024',
    'const MAX_MCP_REQUESTS_PER_MINUTE: i64 = 120',
    'hash_token(&token)',
    'readOnlyHint": true',
    'destructiveHint": false',
    'mcp_audit_receipts',
    'homeserver_world_mission_draft',
    'homeserver_agent_plan_submit',
    'requestOnly',
    '"world.request"',
    '"agents.request"',
):
    require("crates/homeserver-service/src/mcp_runtime.rs", marker, f"local MCP boundary is missing {marker}")
for marker in ("0.0.0.0", "https://", "MG_HOMESERVER_MCP_URL"):
    forbid(
        "crates/homeserver-service/src/mcp_runtime.rs",
        marker,
        f"local MCP runtime contains a disallowed network boundary: {marker}",
    )
require(
    "crates/homeserver-mcp/src/main.rs",
    'const MCP_ENDPOINT: &str = "http://127.0.0.1:47831/mcp"',
    "MCP stdio bridge endpoint is not fixed to loopback",
)
require(
    "src-tauri/tauri.conf.json",
    "resources/microgifter-homeserver-mcp.exe",
    "MCP stdio bridge is not packaged",
)

# Recovery packages and restore activation must be bounded and fail-safe.
for marker in (
    "entry.header().entry_type().is_file()",
    "MAX_DATABASE_BYTES + 1",
    "archive contains duplicate",
    "archive contains too many entries",
):
    require("crates/homeserver-service/src/backup.rs", marker, f"recovery extraction is missing guard: {marker}")
for marker in (
    "activate_staged_database",
    "reactivate the previous HomeServer database",
    "delete_restore_request",
):
    require("crates/homeserver-service/src/backup.rs", marker, f"restore fail-safe is missing {marker}")

# SQLite durability must prefer committed data over throughput.
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

# Supply-chain workflows must default to read-only and pin external actions.
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
    has_contents_write = re.search(r"(?m)^\s*contents:\s*write\s*$", content) is not None
    if has_contents_write:
        if workflow.name != "release-v013.yml":
            ERRORS.append(f"workflow has unnecessary contents write permission: {workflow.name}")
        else:
            required_release_markers = (
                "permissions:\n  contents: read",
                "permissions:\n      contents: write",
                "environment: production-release",
                "if: github.event_name != 'pull_request'",
                "tags:\n      - 'v*.*.*'",
                "gh release create",
                "--verify-tag",
            )
            for marker in required_release_markers:
                if marker not in content:
                    ERRORS.append(
                        f"protected release workflow write permission is missing guard: {marker}"
                    )
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
