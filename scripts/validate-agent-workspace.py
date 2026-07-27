#!/usr/bin/env python3
"""Validate the Phase 5B Agent Workspace and World Mission security contract."""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ERRORS: list[str] = []


def read(path: str) -> str:
    target = ROOT / path
    if not target.is_file():
        ERRORS.append(f"required Agent Workspace file is missing: {path}")
        return ""
    return target.read_text(encoding="utf-8")


def require(path: str, marker: str, message: str) -> None:
    if marker not in read(path):
        ERRORS.append(message)


def forbid(path: str, marker: str, message: str) -> None:
    if marker in read(path):
        ERRORS.append(message)


SERVICE = "crates/homeserver-service/src/agent_runtime.rs"
MIGRATION = "database/migrations/0011_supervised_agent_workspace.sql"
TAURI = "src-tauri/src/agent.rs"
UI = "src/agent-workspace.js"
STYLE = "src/agent-workspace.css"
DOC = "docs/phase-5b-agent-workspace-world-missions.md"

for table in (
    "agent_goals",
    "agent_threads",
    "agent_messages",
    "agent_plans",
    "agent_plan_steps",
    "agent_approval_requests",
    "agent_approvals",
    "agent_action_idempotency",
    "agent_execution_receipts",
    "agent_reports",
    "world_missions",
    "world_tasks",
    "world_conversations",
    "world_conversation_commitments",
    "world_follow_ups",
    "world_mission_events",
    "world_receipts",
):
    require(MIGRATION, table, f"Agent Workspace migration is missing {table}")

for marker in (
    '"/v1/agent/workspace"',
    '"/v1/agent/prompt"',
    '"/v1/agent/approvals/approve"',
    '"/v1/agent/plans/execute"',
    '"/v1/world/missions"',
    "ALLOWED_ACTION_TYPES",
    '"backup.create"',
    '"model.health_test"',
    '"cloud.sync_connection"',
    '"cloud.sync_all"',
    '"report.save"',
    "fresh_state_token",
    "plan_hash",
    "consumed_at_utc",
    "agent_action_idempotency",
    "agent_execution_receipts",
    "LOCAL_ACTOR_ID",
    "requesting MCP client",
    "mission_drafting_only",
    "provider_import_not_enabled_until_phase_5c",
):
    require(SERVICE, marker, f"Agent Workspace service boundary is missing {marker}")

for marker in (
    '"purchase"',
    '"payment"',
    '"claim"',
    '"redemption"',
    '"share_private_profile"',
    '"accept_recurring_commitment"',
    '"publish_campaign"',
    '"bulk_message"',
):
    require(SERVICE, marker, f"World Mode prohibited operation is missing {marker}")

for marker in (
    "homeserver_agent_workspace",
    "homeserver_agent_prompt",
    "homeserver_create_agent_goal",
    "homeserver_approve_agent_plan",
    "homeserver_execute_agent_plan",
    "homeserver_create_world_mission",
):
    require(TAURI, marker, f"Tauri Agent Workspace command is missing {marker}")

for marker in (
    "Agent Workspace",
    "Talk to your HomeServer",
    "Operational data · Phase 5C",
    "World Mission drafting",
    "Execute Once",
    "homeserver_agent_prompt",
    "homeserver_approve_agent_plan",
    "homeserver_execute_agent_plan",
    "homeserver_create_world_mission",
):
    require(UI, marker, f"Agent Workspace UI is missing {marker}")

for marker in (
    ".agent-workspace-shell",
    ".agent-chat-stream",
    ".agent-modal-backdrop",
    ".agent-status-badge",
):
    require(STYLE, marker, f"Agent Workspace style contract is missing {marker}")

for marker in (
    "MCP clients can request",
    "cannot approve",
    "World Mission",
    "Phase 5C",
    "Connected platforms remain authoritative",
):
    require(DOC, marker, f"Agent Workspace documentation is missing {marker}")

# MCP may request plans and missions, but must never receive approval or execution tools.
MCP = "crates/homeserver-service/src/mcp_runtime.rs"
MCP_SOURCE = read(MCP)
MCP_PRODUCTION = MCP_SOURCE.split("#[cfg(test)]", 1)[0]
for marker in (
    "homeserver_agent_prompt",
    "homeserver_agent_plan_submit",
    "homeserver_agent_plan_get",
    "homeserver_agent_plan_list",
    "homeserver_agent_plan_cancel",
    "homeserver_world_mission_draft",
    "homeserver_world_mission_get",
    "homeserver_agent_receipts_list",
):
    if marker not in MCP_PRODUCTION:
        ERRORS.append(f"request-only MCP surface is missing {marker}")

for forbidden in (
    "homeserver_agent_plan_approve",
    "homeserver_agent_plan_execute",
    "homeserver_world_mission_dispatch",
    "shell.execute",
    "commerce.write",
    "campaign.publish",
    "reward.issue",
    "claim.redeem",
):
    if forbidden in MCP_PRODUCTION:
        ERRORS.append(
            f"MCP contains prohibited approval/execution capability: {forbidden}"
        )

# The agent runtime may call only the fixed loopback HomeServer API and local model endpoint.
for forbidden in ("0.0.0.0", "localhost:", "MG_HOMESERVER_AGENT_URL"):
    forbid(
        SERVICE,
        forbidden,
        f"Agent runtime contains a configurable or non-fixed local boundary: {forbidden}",
    )

require(
    "crates/homeserver-service/src/app.rs",
    ".merge(agent_runtime::router(state.clone()))",
    "Agent Workspace router is not inside the secured local API",
)
require(
    "crates/homeserver-service/src/app.rs",
    "agent_runtime::initialize(&connection)?",
    "Agent Workspace migration is not initialized",
)
require(
    "src-tauri/src/lib.rs",
    "mod agent;",
    "Agent Workspace Tauri module is not registered",
)
require(
    "index.html",
    "/src/agent-workspace.js",
    "Agent Workspace frontend module is not loaded",
)
require(
    "package.json",
    "validate-agent-workspace.py",
    "Agent Workspace validator is not part of frontend validation",
)

if ERRORS:
    print("Phase 5B Agent Workspace validation failed:", file=sys.stderr)
    for error in ERRORS:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print(
    "Phase 5B Agent Workspace, supervised approval, and World Mission boundaries validated."
)
