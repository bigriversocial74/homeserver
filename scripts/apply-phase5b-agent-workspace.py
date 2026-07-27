#!/usr/bin/env python3
"""One-time deterministic integration patch for the Phase 5B Agent Workspace branch."""
from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def save(path: str, content: str) -> None:
    (ROOT / path).write_text(content, encoding="utf-8", newline="\n")


def replace_once(content: str, old: str, new: str, path: str) -> str:
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one occurrence, found {count}: {old[:100]!r}")
    return content.replace(old, new, 1)


# Wire the service module, migration, retention, health, and secured router.
path = "crates/homeserver-service/src/app.rs"
content = load(path)
content = replace_once(
    content,
    "    backup, config::AppConfig, database, document_extraction, http, knowledge_vault, mcp_runtime,\n    model_center, semantic_vault, update, update_store, AppState,\n",
    "    agent_runtime, backup, config::AppConfig, database, document_extraction, http,\n    knowledge_vault, mcp_runtime, model_center, semantic_vault, update, update_store, AppState,\n",
    path,
)
content = replace_once(
    content,
    "    semantic_vault::initialize(&connection)?;\n    mcp_runtime::initialize(&connection)?;\n",
    "    semantic_vault::initialize(&connection)?;\n    agent_runtime::initialize(&connection)?;\n    mcp_runtime::initialize(&connection)?;\n",
    path,
)
content = replace_once(
    content,
    "            .merge(semantic_vault::router(state.clone()))\n            .merge(mcp_runtime::router(state)),\n",
    "            .merge(semantic_vault::router(state.clone()))\n            .merge(agent_runtime::router(state.clone()))\n            .merge(mcp_runtime::router(state)),\n",
    path,
)
save(path, content)

path = "crates/homeserver-service/src/main.rs"
content = load(path)
content = replace_once(content, "mod app;\n", "mod agent_runtime;\nmod app;\n", path)
content = replace_once(
    content,
    "        if let Err(error) = mcp_runtime::health_check(&connection) {\n            error!(\n                ?error,\n                \"HomeServer MCP runtime database health check failed\"\n            );\n            return HealthSnapshot::needs_attention(\n                &self.config.server_name,\n                \"mcp_runtime_integrity_check_failed\",\n            );\n        }\n\n        let mut snapshot",
    "        if let Err(error) = mcp_runtime::health_check(&connection) {\n            error!(\n                ?error,\n                \"HomeServer MCP runtime database health check failed\"\n            );\n            return HealthSnapshot::needs_attention(\n                &self.config.server_name,\n                \"mcp_runtime_integrity_check_failed\",\n            );\n        }\n        if let Err(error) = agent_runtime::health_check(&connection) {\n            error!(\n                ?error,\n                \"HomeServer Agent Workspace database health check failed\"\n            );\n            return HealthSnapshot::needs_attention(\n                &self.config.server_name,\n                \"agent_workspace_integrity_check_failed\",\n            );\n        }\n\n        let mut snapshot",
    path,
)
content = replace_once(
    content,
    "            model_center::maintain_history(&connection)?;\n            mcp_runtime::maintain_history(&connection)?;\n",
    "            model_center::maintain_history(&connection)?;\n            agent_runtime::maintain_history(&connection)?;\n            mcp_runtime::maintain_history(&connection)?;\n",
    path,
)
save(path, content)

# Register the trusted Control Center commands.
path = "src-tauri/src/lib.rs"
content = load(path)
content = replace_once(content, "mod cloud;\n", "mod agent;\nmod cloud;\n", path)
content = replace_once(
    content,
    "        .invoke_handler(tauri::generate_handler![\n            homeserver_status,\n",
    "        .invoke_handler(tauri::generate_handler![\n            homeserver_status,\n            agent::homeserver_agent_workspace,\n            agent::homeserver_agent_prompt,\n            agent::homeserver_create_agent_goal,\n            agent::homeserver_archive_agent_goal,\n            agent::homeserver_create_agent_plan,\n            agent::homeserver_cancel_agent_plan,\n            agent::homeserver_approve_agent_plan,\n            agent::homeserver_reject_agent_plan,\n            agent::homeserver_execute_agent_plan,\n            agent::homeserver_create_world_mission,\n            agent::homeserver_cancel_world_mission,\n",
    path,
)
save(path, content)

# Load and validate the frontend module.
path = "index.html"
content = load(path)
content = replace_once(
    content,
    '    <script type="module" src="/src/main.js"></script>\n',
    '    <script type="module" src="/src/main.js"></script>\n    <script type="module" src="/src/agent-workspace.js"></script>\n',
    path,
)
save(path, content)

path = "package.json"
content = load(path)
content = replace_once(
    content,
    'node --check src/main.js && ',
    'node --check src/main.js && node --check src/agent-workspace.js && ',
    path,
)
content = replace_once(
    content,
    'python scripts/validate-mcp-runtime.py && ',
    'python scripts/validate-mcp-runtime.py && python scripts/validate-agent-workspace.py && ',
    path,
)
save(path, content)

# Update permanent security markers and MCP UI copy.
path = "scripts/validate-security-boundaries.py"
content = load(path)
content = replace_once(
    content,
    '    ".merge(semantic_vault::router(state.clone()))",\n    ".merge(mcp_runtime::router(state))",\n',
    '    ".merge(semantic_vault::router(state.clone()))",\n    ".merge(agent_runtime::router(state.clone()))",\n    ".merge(mcp_runtime::router(state))",\n',
    path,
)
content = replace_once(
    content,
    "# Phase 5A MCP must remain fixed-loopback, client-scoped, read-only, and audited.\n",
    "# MCP must remain fixed-loopback, client-scoped, request-only for state changes, and audited.\n",
    path,
)
for marker in (
    '"agents.request"',
    '"world.request"',
    'requestOnly',
    'homeserver_agent_plan_submit',
    'homeserver_world_mission_draft',
):
    insertion_anchor = '    \'mcp_audit_receipts\',\n'
    if marker not in content:
        content = replace_once(
            content,
            insertion_anchor,
            insertion_anchor + f'    \'{marker}\',\n',
            path,
        )
save(path, content)

path = "src/main.js"
content = load(path)
content = replace_once(
    content,
    "Client-scoped, read-only access to local health, sync status, model inventory, and cited Knowledge Vault retrieval.",
    "Client-scoped local reads plus request-only supervised plan and World Mission drafting.",
    path,
)
content = replace_once(content, "Read-only local boundary", "Supervised local boundary", path)
content = replace_once(
    content,
    "No write tools, cloud actions, commerce actions, arbitrary endpoints, LAN listener, or public listener are exposed.",
    "Read tools and request-only plan or mission tools are exposed. Approval, execution, and World dispatch remain local Control Center actions.",
    path,
)
save(path, content)

# Correct initial Agent Runtime source issues before compiling.
path = "crates/homeserver-service/src/agent_runtime.rs"
content = load(path)
content = content.replace("use anyhow::{anyhow, bail, ensure, Context, Result};", "use anyhow::{bail, ensure, Context, Result};")
content = content.replace("model.name == ***candidate", "model.name == candidate.as_str()")
content = content.replace('"connection_ids": request.connection_ids,', '"connection_ids": &request.connection_ids,')
content = content.replace('"dataset_keys": request.dataset_keys,', '"dataset_keys": &request.dataset_keys,')
content = content.replace('"goal_ids": request.goal_ids,', '"goal_ids": &request.goal_ids,')
content = content.replace('"plan_id": plan_id,', '"plan_id": &plan_id,', 1)
content = content.replace('"requested_by_id": actor_id,', '"requested_by_id": &actor_id,', 1)
content = content.replace('"title": title,', '"title": &title,', 1)
content = content.replace('"rationale": rationale,', '"rationale": &rationale,', 1)
content = content.replace('"action_type": action_type,', '"action_type": &action_type,', 1)
content = content.replace('"arguments": arguments,', '"arguments": &arguments,', 1)
content = content.replace('"connection_id": connection_id,', '"connection_id": &connection_id,', 1)
content = content.replace('"goal_id": goal_id,', '"goal_id": &goal_id,', 1)
content = content.replace('"dataset_keys": dataset_keys,', '"dataset_keys": &dataset_keys,', 1)
content = content.replace('"fresh_state_token": fresh_state_token,', '"fresh_state_token": &fresh_state_token,', 1)
content = content.replace('"expires_at_utc": expires_at_utc,', '"expires_at_utc": &expires_at_utc,', 1)
content = content.replace('"created_at_utc": created_at_utc,', '"created_at_utc": &created_at_utc,', 1)
for field in ("plan_id", "requested_by_type", "requested_by_id", "title", "rationale", "action_type", "arguments", "connection_id", "goal_id", "dataset_keys"):
    content = content.replace(f'"{field}": plan.{field},', f'"{field}": &plan.{field},')
content = content.replace(
    "    approval_request_id: String,\n    plan_id: String,\n    plan_hash: String,\n",
    "    plan_hash: String,\n",
)
content = content.replace(
    '"SELECT approval_id,approval_request_id,plan_id,plan_hash,expires_at_utc,consumed_at_utc FROM agent_approvals WHERE plan_id=?1",',
    '"SELECT approval_id,plan_hash,expires_at_utc,consumed_at_utc FROM agent_approvals WHERE plan_id=?1",',
)
content = content.replace(
    "                    approval_id: row.get(0)?,\n                    approval_request_id: row.get(1)?,\n                    plan_id: row.get(2)?,\n                    plan_hash: row.get(3)?,\n                    expires_at_utc: row.get(4)?,\n                    consumed_at_utc: row.get(5)?,\n",
    "                    approval_id: row.get(0)?,\n                    plan_hash: row.get(1)?,\n                    expires_at_utc: row.get(2)?,\n                    consumed_at_utc: row.get(3)?,\n",
)
save(path, content)

# Expand MCP with read tools plus request-only supervised tools.
path = "crates/homeserver-service/src/mcp_runtime.rs"
content = load(path)
content = replace_once(
    content,
    "use crate::{model_center, semantic_vault, AppState};",
    "use crate::{agent_runtime, model_center, semantic_vault, AppState};",
    path,
)
content = replace_once(
    content,
    '    "knowledge.read",\n];',
    '    "knowledge.read",\n    "agents.read",\n    "agents.request",\n    "world.request",\n];',
    path,
)
content = replace_once(
    content,
    "    pub read_only: bool,\n    pub local_only: bool,",
    "    pub read_only: bool,\n    pub request_only: bool,\n    pub local_only: bool,",
    path,
)
content = content.replace('"read-only MCP operation failed"', '"MCP operation failed"')
content = content.replace('"HomeServer could not complete the local read-only operation."', '"HomeServer could not complete the local MCP operation."')
content = content.replace("This read-only HomeServer MCP runtime is stateless.", "This supervised HomeServer MCP runtime is stateless.")
content = replace_once(
    content,
    '            "title": "Microgifter HomeServer — Read-only Local MCP",',
    '            "title": "Microgifter HomeServer — Supervised Local MCP",',
    path,
)
content = replace_once(
    content,
    '        "instructions": "This server exposes only client-scoped, read-only local HomeServer status, model inventory, cloud status, and cited Knowledge Vault retrieval. It cannot modify files, models, cloud records, commerce, campaigns, rewards, claims, or settings."',
    '        "instructions": "This server exposes client-scoped local reads plus request-only HomeServer agent plans and World Mission drafts. MCP clients cannot approve, execute, dispatch, modify commerce, publish campaigns, issue rewards, redeem claims, run shell commands, or bypass the local Control Center."',
    path,
)

call_arms = r'''        "homeserver_agent_workspace" => {
            require_scope(client, "agents.read")?;
            let snapshot = agent_runtime::workspace_snapshot(state.clone())
                .await
                .map_err(|error| RpcFailure::internal(error, "agents.read"))?;
            (
                serde_json::to_value(snapshot)
                    .map_err(|error| RpcFailure::internal(error.into(), "agents.read"))?,
                "agents.read",
            )
        }
        "homeserver_agent_prompt" => {
            require_scope(client, "agents.request")?;
            let payload = agent_runtime::mcp_prompt(
                state.clone(),
                &client.client_id,
                arguments,
            )
            .await
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "agents.request"))?;
            (payload, "agents.request")
        }
        "homeserver_agent_plan_submit" => {
            require_scope(client, "agents.request")?;
            let state_for_plan = state.clone();
            let client_id = client.client_id.clone();
            let payload = tokio::task::spawn_blocking(move || {
                agent_runtime::mcp_submit_plan(&state_for_plan, &client_id, arguments)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "agents.request"))?
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "agents.request"))?;
            (payload, "agents.request")
        }
        "homeserver_agent_plan_get" => {
            require_scope(client, "agents.read")?;
            let plan_id = required_string(&arguments, "plan_id", "agents.read")?;
            let state_for_plan = state.clone();
            let client_id = client.client_id.clone();
            let payload = tokio::task::spawn_blocking(move || {
                agent_runtime::mcp_get_plan(&state_for_plan, &plan_id, &client_id)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "agents.read"))?
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "agents.read"))?;
            (payload, "agents.read")
        }
        "homeserver_agent_plan_list" => {
            require_scope(client, "agents.read")?;
            let state_for_plan = state.clone();
            let client_id = client.client_id.clone();
            let payload = tokio::task::spawn_blocking(move || {
                agent_runtime::mcp_list_plans(&state_for_plan, &client_id)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "agents.read"))?
            .map_err(|error| RpcFailure::internal(error, "agents.read"))?;
            (payload, "agents.read")
        }
        "homeserver_agent_plan_cancel" => {
            require_scope(client, "agents.request")?;
            let plan_id = required_string(&arguments, "plan_id", "agents.request")?;
            let state_for_plan = state.clone();
            let client_id = client.client_id.clone();
            let payload = tokio::task::spawn_blocking(move || {
                agent_runtime::mcp_cancel_plan(&state_for_plan, &client_id, &plan_id)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "agents.request"))?
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "agents.request"))?;
            (payload, "agents.request")
        }
        "homeserver_world_mission_draft" => {
            require_scope(client, "world.request")?;
            let state_for_mission = state.clone();
            let client_id = client.client_id.clone();
            let payload = tokio::task::spawn_blocking(move || {
                agent_runtime::mcp_draft_world_mission(
                    &state_for_mission,
                    &client_id,
                    arguments,
                )
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "world.request"))?
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "world.request"))?;
            (payload, "world.request")
        }
        "homeserver_world_mission_get" => {
            require_scope(client, "agents.read")?;
            let mission_id = required_string(&arguments, "mission_id", "agents.read")?;
            let state_for_mission = state.clone();
            let payload = tokio::task::spawn_blocking(move || {
                agent_runtime::mcp_get_world_mission(&state_for_mission, &mission_id)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "agents.read"))?
            .map_err(|error| RpcFailure::invalid_params(error.to_string(), "agents.read"))?;
            (payload, "agents.read")
        }
        "homeserver_agent_receipts_list" => {
            require_scope(client, "agents.read")?;
            let state_for_receipts = state.clone();
            let client_id = client.client_id.clone();
            let payload = tokio::task::spawn_blocking(move || {
                agent_runtime::mcp_list_receipts(&state_for_receipts, &client_id)
            })
            .await
            .map_err(|error| RpcFailure::internal(error.into(), "agents.read"))?
            .map_err(|error| RpcFailure::internal(error, "agents.read"))?;
            (payload, "agents.read")
        }
'''
anchor = '            (payload, "knowledge.read")\n        }\n        _ => {'
content = replace_once(content, anchor, '            (payload, "knowledge.read")\n        }\n' + call_arms + '        _ => {', path)
content = content.replace("Read-only MCP tool '{name}' is not available.", "MCP tool '{name}' is not available.")

# Tool definitions remain closed-world and request-only.
tool_insert = r'''    if scopes.contains("agents.read") {
        tools.push(read_only_tool(
            "homeserver_agent_workspace",
            "Read the local Agent Workspace snapshot, goals, plans, approvals, World Mission drafts, and receipts.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ));
        tools.push(read_only_tool(
            "homeserver_agent_plan_get",
            "Read one supervised plan requested by this MCP client.",
            id_schema("plan_id"),
        ));
        tools.push(read_only_tool(
            "homeserver_agent_plan_list",
            "List supervised plans requested by this MCP client.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ));
        tools.push(read_only_tool(
            "homeserver_world_mission_get",
            "Read one locally stored World Mission draft.",
            id_schema("mission_id"),
        ));
        tools.push(read_only_tool(
            "homeserver_agent_receipts_list",
            "List execution receipts for this MCP client's approved plans.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ));
    }
    if scopes.contains("agents.request") {
        tools.push(request_tool(
            "homeserver_agent_prompt",
            "Ask the private HomeServer agent to use selected local context. This tool cannot approve or execute actions.",
            json!({
                "type": "object",
                "properties": {
                    "thread_id": { "type": ["string", "null"] },
                    "mode": { "type": "string", "enum": ["ask", "analyze", "plan", "dispatch", "execute"] },
                    "prompt": { "type": "string", "minLength": 1, "maxLength": 4000 },
                    "connection_ids": { "type": "array", "maxItems": 20, "items": { "type": "string" } },
                    "dataset_keys": { "type": "array", "maxItems": 20, "items": { "type": "string" } },
                    "goal_ids": { "type": "array", "maxItems": 20, "items": { "type": "string" } },
                    "knowledge_query": { "type": ["string", "null"], "maxLength": 200 },
                    "model": { "type": ["string", "null"], "maxLength": 160 },
                    "proposed_action": { "type": "null" },
                    "world_mission": { "type": "null" }
                },
                "required": ["mode", "prompt"],
                "additionalProperties": false
            }),
        ));
        tools.push(request_tool(
            "homeserver_agent_plan_submit",
            "Submit a bounded supervised action plan for local approval. This tool cannot approve or execute it.",
            plan_schema(),
        ));
        tools.push(request_tool(
            "homeserver_agent_plan_cancel",
            "Cancel an unexecuted plan requested by this MCP client.",
            id_schema("plan_id"),
        ));
    }
    if scopes.contains("world.request") {
        tools.push(request_tool(
            "homeserver_world_mission_draft",
            "Draft a bounded World Mission locally. This tool cannot dispatch a World Agent.",
            world_mission_schema(),
        ));
    }
'''
content = replace_once(content, "    tools\n}\n\nfn read_only_tool", tool_insert + "    tools\n}\n\nfn read_only_tool", path)

helper_insert = r'''
fn request_tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "title": name.replace('_', " "),
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": false,
            "requestOnly": true
        }
    })
}

fn id_schema(field: &str) -> Value {
    json!({
        "type": "object",
        "properties": { field: { "type": "string", "minLength": 1, "maxLength": 80 } },
        "required": [field],
        "additionalProperties": false
    })
}

fn plan_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "thread_id": { "type": ["string", "null"] },
            "title": { "type": "string", "minLength": 1, "maxLength": 180 },
            "rationale": { "type": "string", "minLength": 1, "maxLength": 4000 },
            "action_type": { "type": "string", "enum": ["backup.create", "model.health_test", "cloud.sync_connection", "cloud.sync_all", "report.save"] },
            "arguments": { "type": "object" },
            "connection_id": { "type": ["string", "null"] },
            "goal_id": { "type": ["string", "null"] },
            "dataset_keys": { "type": "array", "maxItems": 20, "items": { "type": "string" } },
            "expires_minutes": { "type": ["integer", "null"], "minimum": 5, "maximum": 1440 }
        },
        "required": ["title", "rationale", "action_type"],
        "additionalProperties": false
    })
}

fn world_mission_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "thread_id": { "type": ["string", "null"] },
            "goal_id": { "type": ["string", "null"] },
            "connection_id": { "type": ["string", "null"] },
            "world_agent_id": { "type": "string", "minLength": 1, "maxLength": 160 },
            "title": { "type": "string", "minLength": 1, "maxLength": 180 },
            "objective": { "type": "string", "minLength": 1, "maxLength": 4000 },
            "allowed_operations": { "type": "array", "maxItems": 20, "items": { "type": "string" } },
            "prohibited_operations": { "type": "array", "maxItems": 20, "items": { "type": "string" } },
            "limits": { "type": "object" },
            "disclosure_policy": { "type": "object" },
            "expires_minutes": { "type": ["integer", "null"], "minimum": 15, "maximum": 10080 }
        },
        "required": ["world_agent_id", "title", "objective"],
        "additionalProperties": false
    })
}

fn required_string(arguments: &Value, field: &str, capability: &'static str) -> Result<String, RpcFailure> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 80)
        .map(ToOwned::to_owned)
        .ok_or_else(|| RpcFailure::invalid_params(format!("{field} is required."), capability))
}
'''
content = replace_once(content, "fn resource_definitions", helper_insert + "\nfn resource_definitions", path)
content = replace_once(
    content,
    '            "homeserver_knowledge_document".to_owned(),\n        ],',
    '            "homeserver_knowledge_document".to_owned(),\n            "homeserver_agent_workspace".to_owned(),\n            "homeserver_agent_prompt".to_owned(),\n            "homeserver_agent_plan_submit".to_owned(),\n            "homeserver_agent_plan_get".to_owned(),\n            "homeserver_agent_plan_list".to_owned(),\n            "homeserver_agent_plan_cancel".to_owned(),\n            "homeserver_world_mission_draft".to_owned(),\n            "homeserver_world_mission_get".to_owned(),\n            "homeserver_agent_receipts_list".to_owned(),\n        ],',
    path,
)
content = replace_once(
    content,
    "        read_only: true,\n        local_only: true,",
    "        read_only: false,\n        request_only: true,\n        local_only: true,",
    path,
)
save(path, content)

# Update MCP validation wording and request-only guarantees.
path = "scripts/validate-mcp-runtime.py"
content = load(path)
content = content.replace("Validate Phase 5A local read-only MCP security and packaging boundaries.", "Validate local MCP read and request-only security and packaging boundaries.")
content = content.replace("Phase 5A", "HomeServer MCP")
content = content.replace("Phase 5A contains a state-changing MCP capability", "MCP contains a prohibited direct state-changing capability")
content = content.replace("Phase 5A local read-only MCP boundaries validated.", "HomeServer local MCP read and request-only boundaries validated.")
content = replace_once(
    content,
    "    'mcp_audit_receipts',\n",
    "    'mcp_audit_receipts',\n    'requestOnly',\n    'homeserver_agent_plan_submit',\n    'homeserver_world_mission_draft',\n",
    path,
)
content = replace_once(
    content,
    "    'campaign.create', 'reward.issue', 'claim.redeem', 'shell.execute'\n",
    "    'campaign.create', 'reward.issue', 'claim.redeem', 'shell.execute',\n    'homeserver_agent_plan_approve', 'homeserver_agent_plan_execute',\n    'homeserver_world_mission_dispatch'\n",
    path,
)
save(path, content)

print("Phase 5B Agent Workspace integration patch applied.")
