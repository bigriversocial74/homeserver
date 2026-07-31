from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    value = target.read_text(encoding="utf-8")
    if value.count(old) != 1:
        raise SystemExit(f"{path}: engagement patch target not found exactly once: {old[:100]!r}")
    target.write_text(value.replace(old, new, 1), encoding="utf-8")


replace_once(
    "crates/homeserver-service/src/agent_integrations.rs",
    '''    let dismissed = {
        let connection = state.connection()?;
        dismissed_prompt_keys(&connection)?
    };
    let guidance = build_guidance(
''',
    '''    let (dismissed, last_user_prompt_at_utc) = {
        let connection = state.connection()?;
        (
            dismissed_prompt_keys(&connection)?,
            last_user_prompt_at_utc(&connection)?,
        )
    };
    let guidance = build_guidance(
''',
)
replace_once(
    "crates/homeserver-service/src/agent_integrations.rs",
    '''        &backups,
        &integrations,
    );
''',
    '''        &backups,
        &integrations,
        last_user_prompt_at_utc.as_deref(),
    );
''',
)
replace_once(
    "crates/homeserver-service/src/agent_integrations.rs",
    '''fn build_guidance(
    health: &Value,
    knowledge: &Value,
    models: &Value,
    operational: &operational_data::OperationalDataSnapshot,
    clouds: &cloud_registry::CloudConnectionsSnapshot,
    backups: &microgifter_homeserver_core::BackupCatalog,
    integrations: &[SiteIntegrationSummary],
) -> Vec<AgentGuidanceItem> {
''',
    '''fn build_guidance(
    health: &Value,
    knowledge: &Value,
    models: &Value,
    operational: &operational_data::OperationalDataSnapshot,
    clouds: &cloud_registry::CloudConnectionsSnapshot,
    backups: &microgifter_homeserver_core::BackupCatalog,
    integrations: &[SiteIntegrationSummary],
    last_user_prompt_at_utc: Option<&str>,
) -> Vec<AgentGuidanceItem> {
''',
)
replace_once(
    "crates/homeserver-service/src/agent_integrations.rs",
    '''    if items.is_empty() {
        items.push(guidance(
            "daily_brief",
            "Your HomeServer is ready",
            "Ask for a daily brief across system health, Knowledge Vault, connected sites, goals, schedules, approvals, and recent activity.",
            "Start daily brief",
            "agent:prompt:Give me my HomeServer daily brief.",
            10,
        ));
    }
''',
    '''    if items.is_empty() && daily_brief_due(last_user_prompt_at_utc) {
        items.push(guidance(
            "daily_brief",
            "Your HomeServer daily brief is ready",
            "Review system health, Knowledge Vault, connected sites, goals, schedules, approvals, and recent activity in one conversation.",
            "Start daily brief",
            "agent:prompt:Give me my HomeServer daily brief.",
            10,
        ));
    }
''',
)
replace_once(
    "crates/homeserver-service/src/agent_integrations.rs",
    '''fn dismissed_prompt_keys(connection: &Connection) -> Result<Vec<String>> {
''',
    '''fn last_user_prompt_at_utc(connection: &Connection) -> Result<Option<String>> {
    connection
        .query_row(
            "SELECT last_user_prompt_at_utc FROM agent_engagement_state WHERE singleton_id=1",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub fn record_user_engagement(state: &AppState) -> Result<()> {
    let now = now_string();
    state.connection()?.execute(
        "UPDATE agent_engagement_state SET last_user_prompt_at_utc=?1,onboarding_completed_at_utc=COALESCE(onboarding_completed_at_utc,?1),engagement_revision=engagement_revision+1,updated_at_utc=?1 WHERE singleton_id=1",
        params![now],
    )?;
    Ok(())
}

fn daily_brief_due(last_user_prompt_at_utc: Option<&str>) -> bool {
    last_user_prompt_at_utc
        .and_then(|value| parse_time(value).ok())
        .map_or(true, |value| value <= Utc::now() - ChronoDuration::hours(18))
}

fn dismissed_prompt_keys(connection: &Connection) -> Result<Vec<String>> {
''',
)
replace_once(
    "crates/homeserver-service/src/agent_runtime.rs",
    '''    let user_message_id = save_message(
        &state,
        &thread_id,
        "user",
        &request.mode,
        &request.prompt,
        &json!({
            "connection_ids": &request.connection_ids,
            "dataset_keys": &request.dataset_keys,
            "goal_ids": &request.goal_ids,
            "actor_type": actor_type,
            "actor_id": actor_id,
        }),
    )?;

    let cloud_snapshot = {
''',
    '''    let user_message_id = save_message(
        &state,
        &thread_id,
        "user",
        &request.mode,
        &request.prompt,
        &json!({
            "connection_ids": &request.connection_ids,
            "dataset_keys": &request.dataset_keys,
            "goal_ids": &request.goal_ids,
            "actor_type": actor_type,
            "actor_id": actor_id,
        }),
    )?;
    agent_integrations::record_user_engagement(&state)?;

    let cloud_snapshot = {
''',
)

replace_once(
    "src/main.js",
    '''let mcpBridgePath = null;
let notice = null;
''',
    '''let mcpBridgePath = null;
let agentIntegrationSnapshot = null;
let notice = null;
''',
)
replace_once(
    "src/main.js",
    '''  if (updateDisplayState() === "not_configured") items.push({ tone: "info", icon: "update", title: "Release channel setup needed", detail: "Configure the signed HomeServer update source.", page: "system" });
  if (!items.length) items.push({ tone: "success", icon: "shield", title: "HomeServer is healthy", detail: "No active system alerts require attention.", page: "dashboard" });
''',
    '''  if (updateDisplayState() === "not_configured") items.push({ tone: "info", icon: "update", title: "Release channel setup needed", detail: "Configure the signed HomeServer update source.", page: "system" });
  const agentPrompt = agentIntegrationSnapshot?.active_prompt;
  if (agentPrompt) items.push({ tone: "info", icon: "integrations", title: agentPrompt.title, detail: agentPrompt.message, page: "agent" });
  if (!items.length) items.push({ tone: "success", icon: "shield", title: "HomeServer is healthy", detail: "No active system alerts require attention.", page: "dashboard" });
''',
)
replace_once(
    "src/main.js",
    '''    invoke("homeserver_mcp_bridge_path"),
    invoke("control_center_autostart_enabled"),
  ]);
  if (results[9].status === "fulfilled") desktopAutostartEnabled = Boolean(results[9].value);
''',
    '''    invoke("homeserver_mcp_bridge_path"),
    invoke("homeserver_agent_integrations"),
    invoke("control_center_autostart_enabled"),
  ]);
  if (results[10].status === "fulfilled") desktopAutostartEnabled = Boolean(results[10].value);
''',
)
replace_once(
    "src/main.js",
    '''  mcpSnapshot = results[7].status === "fulfilled" ? results[7].value : mcpSnapshot;
  mcpBridgePath = results[8].status === "fulfilled" ? results[8].value : mcpBridgePath;

  const health = {
''',
    '''  mcpSnapshot = results[7].status === "fulfilled" ? results[7].value : mcpSnapshot;
  mcpBridgePath = results[8].status === "fulfilled" ? results[8].value : mcpBridgePath;
  agentIntegrationSnapshot = results[9].status === "fulfilled" ? results[9].value : agentIntegrationSnapshot;

  const health = {
''',
)
replace_once(
    "scripts/validate-unified-agent-orchestration.py",
    '''    "record_context_receipt",
])
''',
    '''    "record_context_receipt",
    "record_user_engagement",
])
''',
)
replace_once(
    "scripts/validate-unified-agent-orchestration.py",
    '''require("src/homeserver-agent-chat.js", [
''',
    '''require("src/main.js", [
    "let agentIntegrationSnapshot = null;",
    "agentIntegrationSnapshot?.active_prompt",
    'invoke("homeserver_agent_integrations")',
])
require("src/homeserver-agent-chat.js", [
''',
)

print("Phase 22 engagement integration applied.")
