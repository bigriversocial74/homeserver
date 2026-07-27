#!/usr/bin/env python3
"""Validate local MCP read and request-only security and packaging boundaries."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        ERRORS.append(f"required HomeServer MCP file is missing: {path}")
        return ""
    return target.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in read(path):
        ERRORS.append(message)


def forbid(path: str, marker: str, message: str) -> None:
    if marker in read(path):
        ERRORS.append(message)

SERVICE = "crates/homeserver-service/src/mcp_runtime.rs"
BRIDGE = "crates/homeserver-mcp/src/main.rs"
MIGRATION = "database/migrations/0009_mcp_runtime.sql"
TAURI = "src-tauri/src/mcp.rs"
UI = "src/main.js"

for marker in (
    'const MCP_ENDPOINT: &str = "http://127.0.0.1:47831/mcp"',
    'const MAX_MCP_BODY_BYTES: usize = 128 * 1024',
    'const MAX_MCP_RESPONSE_BYTES: usize = 1024 * 1024',
    'const MAX_MCP_REQUESTS_PER_MINUTE: i64 = 120',
    'hash_token(&token)',
    'token_hash TEXT NOT NULL UNIQUE',
    'readOnlyHint": true',
    'destructiveHint": false',
    'openWorldHint": false',
    'homeserver_knowledge_search',
    'homeserver_knowledge_document',
    'mcp_audit_receipts',
    'requestOnly',
    'homeserver_agent_plan_submit',
    'homeserver_world_mission_draft',
    'Bearer realm=\\"Microgifter HomeServer MCP\\"',
):
    path = MIGRATION if marker in ('token_hash TEXT NOT NULL UNIQUE', 'mcp_audit_receipts') else SERVICE
    require(path, marker, f"MCP runtime boundary is missing: {marker}")

for marker in (
    'const MCP_ENDPOINT: &str = "http://127.0.0.1:47831/mcp"',
    'const MCP_TOKEN_ENV: &str = "MG_HOMESERVER_MCP_TOKEN"',
    'MAX_REQUEST_BYTES',
    'MAX_RESPONSE_BYTES',
    'eprintln!',
    'stdout.write_all',
):
    require(BRIDGE, marker, f"MCP stdio bridge boundary is missing: {marker}")

for marker in (
    'homeserver_mcp_bridge_path',
    'resource_dir()',
    'microgifter-homeserver-mcp.exe',
):
    require(TAURI, marker, f"MCP Control Center bridge boundary is missing: {marker}")

for marker in (
    'Local MCP Runtime',
    'Create MCP Client',
    'homeserver_create_mcp_client',
    'homeserver_revoke_mcp_client',
    'MG_HOMESERVER_MCP_TOKEN',
):
    require(UI, marker, f"MCP Control Center UI is missing: {marker}")

for path in (SERVICE, BRIDGE):
    for marker in ('0.0.0.0', 'localhost:', 'https://', 'MG_HOMESERVER_MCP_URL'):
        forbid(path, marker, f"{path} contains a disallowed configurable/network MCP boundary: {marker}")

service_runtime = read(SERVICE).split("#[cfg(test)]", 1)[0]
for marker in (
    'models.write', 'knowledge.write', 'cloud.write', 'files.write', 'commerce.write',
    'campaign.create', 'reward.issue', 'claim.redeem', 'shell.execute',
    'homeserver_agent_plan_approve', 'homeserver_agent_plan_execute',
    'homeserver_world_mission_dispatch'
):
    if marker in service_runtime:
        ERRORS.append(f"HomeServer MCP contains a state-changing MCP capability: {marker}")

require('src-tauri/tauri.conf.json', 'resources/microgifter-homeserver-mcp.exe', 'MCP bridge is not packaged as a Tauri resource')
require('Cargo.toml', 'crates/homeserver-mcp', 'MCP bridge is not a workspace member')
require('crates/homeserver-service/src/app.rs', '.merge(mcp_runtime::router(state))', 'MCP router is not merged inside the secured local API')

if ERRORS:
    print('HomeServer MCP MCP validation failed:', file=sys.stderr)
    for error in ERRORS:
        print(f'- {error}', file=sys.stderr)
    raise SystemExit(1)
print('HomeServer MCP local read-only MCP boundaries validated.')
