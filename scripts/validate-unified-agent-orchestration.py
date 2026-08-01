#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def require(path: str, needles: list[str]) -> None:
    value = (ROOT / path).read_text(encoding="utf-8")
    for needle in needles:
        if needle not in value:
            raise SystemExit(f"{path}: missing unified Agent contract: {needle}")


def require_absent(paths: list[str]) -> None:
    for path in paths:
        if (ROOT / path).exists():
            raise SystemExit(f"temporary Phase 22 file must not be retained: {path}")


require_absent([
    ".github/scripts/repair_phase22_clippy.py",
    ".github/workflows/_repair_phase22_clippy.yml",
    ".github/scripts/apply_phase22_unified_agent.py",
    ".github/workflows/_stage_phase22_unified_agent.yml",
])

require("crates/homeserver-service/src/agent_integrations.rs", [
    "oauth_callback_router",
    "http://127.0.0.1:47831/oauth/microgifter/callback",
    "collect_mcp_grounding",
    "read_tools_may_run_automatically",
    "state_changing_tools_require_authority",
    "CALL {tool_name}",
    "https://mcp.microgifter.com/mcp",
    "PKCE verifier is unavailable",
    "agent_mcp_invocation_receipts",
    "let connection = task_state.connection()?;\n            read_integrations(&connection)",
    "crate::database::installation_id(&connection)?",
    "Ok(integration_by_id(&connection, connection_id)?.summary)",
    "daily_brief_due(last_user_prompt_at_utc)",
    "ChronoDuration::hours(18)",
    "pub(crate) struct ContextReceiptInput",
    "struct GuidanceContext",
    "struct McpReceiptInput",
    "sort_by_key(|item| std::cmp::Reverse(item.priority))",
    ".rfind(|line| !line.is_empty())",
])
require("crates/homeserver-service/src/agent_runtime.rs", [
    "Knowledge Vault search failed",
    "live_site_mcp_tools",
    "compact_knowledge_evidence",
    "compact_mcp_evidence",
    "primary interface to this HomeServer",
    "record_context_receipt",
    "record_user_engagement",
])
require("crates/homeserver-service/src/semantic_vault.rs", [
    "pub(crate) fn snapshot(state: &AppState)",
])
require("src/main.js", [
    "let agentIntegrationSnapshot = null;",
    "agentIntegrationSnapshot?.active_prompt",
    'invoke("homeserver_agent_integrations")',
])
require("src/homeserver-agent-chat.js", [
    "renderAgentGuidance",
    "renderMcpIntegrationPanel",
    'value="operational_data" checked',
    "homeserver_open_agent_authorization",
    "goals().map((goal) => goal.goal_id)",
])
require("database/migrations/0029_unified_agent_orchestration.sql", [
    "agent_site_integrations",
    "agent_engagement_state",
    "agent_context_receipts",
    "agent_mcp_invocation_receipts",
])
print("Unified HomeServer Agent orchestration contract passed.")
