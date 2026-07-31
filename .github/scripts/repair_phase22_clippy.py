from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    value = target.read_text(encoding="utf-8")
    count = value.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one lint repair target, found {count}: {old[:120]!r}")
    target.write_text(value.replace(old, new, 1), encoding="utf-8")


integration_path = "crates/homeserver-service/src/agent_integrations.rs"

replace_once(
    integration_path,
    '''#[derive(Debug, Clone)]
struct IntegrationRecord {
    summary: SiteIntegrationSummary,
    credential_key: String,
    pending_expires_at_utc: Option<String>,
}
''',
    '''#[derive(Debug, Clone)]
struct IntegrationRecord {
    summary: SiteIntegrationSummary,
    credential_key: String,
    pending_expires_at_utc: Option<String>,
}

pub(crate) struct ContextReceiptInput<'a> {
    pub thread_id: Option<&'a str>,
    pub prompt: &'a str,
    pub source_keys: &'a [String],
    pub knowledge_hits: usize,
    pub operational_records: usize,
    pub mcp_tools: &'a [String],
    pub context_hash: &'a str,
    pub inference_state: &'a str,
    pub failure_code: Option<&'a str>,
}

struct GuidanceContext<'a> {
    health: &'a Value,
    knowledge: &'a Value,
    models: &'a Value,
    operational: &'a operational_data::OperationalDataSnapshot,
    clouds: &'a cloud_registry::CloudConnectionsSnapshot,
    backups: &'a microgifter_homeserver_core::BackupCatalog,
    integrations: &'a [SiteIntegrationSummary],
    last_user_prompt_at_utc: Option<&'a str>,
}

struct McpReceiptInput<'a> {
    connection_id: &'a str,
    tool_name: &'a str,
    operation_class: &'a str,
    request_hash: &'a str,
    result_hash: Option<&'a str>,
    outcome: &'a str,
    result_code: String,
    duration_ms: u64,
}
''',
)

replace_once(
    integration_path,
    '''    let guidance = build_guidance(
        &health,
        &knowledge,
        &models,
        &operational,
        &clouds,
        &backups,
        &integrations,
        last_user_prompt_at_utc.as_deref(),
    );
''',
    '''    let guidance = build_guidance(GuidanceContext {
        health: &health,
        knowledge: &knowledge,
        models: &models,
        operational: &operational,
        clouds: &clouds,
        backups: &backups,
        integrations: &integrations,
        last_user_prompt_at_utc: last_user_prompt_at_utc.as_deref(),
    });
''',
)

replace_once(
    integration_path,
    '''pub fn record_context_receipt(
    state: &AppState,
    thread_id: Option<&str>,
    prompt: &str,
    source_keys: &[String],
    knowledge_hits: usize,
    operational_records: usize,
    mcp_tools: &[String],
    context_hash: &str,
    inference_state: &str,
    failure_code: Option<&str>,
) -> Result<String> {
    ensure!(
        ["not_started", "completed", "unavailable", "failed"].contains(&inference_state),
        "Agent context inference state is invalid"
    );
    ensure!(context_hash.len() == 64, "Agent context hash is invalid");
    let receipt_id = Uuid::new_v4().to_string();
    state.connection()?.execute(
        "INSERT INTO agent_context_receipts (receipt_id,thread_id,prompt_hash,source_keys_json,knowledge_hit_count,operational_record_count,mcp_tool_names_json,context_hash,inference_state,failure_code) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            receipt_id,
            thread_id,
            sha256_hex(prompt.as_bytes()),
            serde_json::to_string(source_keys)?,
            knowledge_hits as i64,
            operational_records as i64,
            serde_json::to_string(mcp_tools)?,
            context_hash,
            inference_state,
            failure_code,
        ],
    )?;
    Ok(receipt_id)
}
''',
    '''pub fn record_context_receipt(
    state: &AppState,
    input: ContextReceiptInput<'_>,
) -> Result<String> {
    ensure!(
        ["not_started", "completed", "unavailable", "failed"]
            .contains(&input.inference_state),
        "Agent context inference state is invalid"
    );
    ensure!(
        input.context_hash.len() == 64,
        "Agent context hash is invalid"
    );
    let receipt_id = Uuid::new_v4().to_string();
    state.connection()?.execute(
        "INSERT INTO agent_context_receipts (receipt_id,thread_id,prompt_hash,source_keys_json,knowledge_hit_count,operational_record_count,mcp_tool_names_json,context_hash,inference_state,failure_code) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            receipt_id,
            input.thread_id,
            sha256_hex(input.prompt.as_bytes()),
            serde_json::to_string(input.source_keys)?,
            input.knowledge_hits as i64,
            input.operational_records as i64,
            serde_json::to_string(input.mcp_tools)?,
            input.context_hash,
            input.inference_state,
            input.failure_code,
        ],
    )?;
    Ok(receipt_id)
}
''',
)

replace_once(
    integration_path,
    '''            write_mcp_receipt(
                &state,
                &request.connection_id,
                &tool_name,
                &tool.operation_class,
                &request_hash,
                Some(&result_hash),
                "completed",
                "mcp_tool_completed",
                started.elapsed().as_millis() as u64,
            )?;
''',
    '''            write_mcp_receipt(
                &state,
                McpReceiptInput {
                    connection_id: &request.connection_id,
                    tool_name: &tool_name,
                    operation_class: &tool.operation_class,
                    request_hash: &request_hash,
                    result_hash: Some(&result_hash),
                    outcome: "completed",
                    result_code: "mcp_tool_completed".to_owned(),
                    duration_ms: started.elapsed().as_millis() as u64,
                },
            )?;
''',
)

replace_once(
    integration_path,
    '''            write_mcp_receipt(
                &state,
                &request.connection_id,
                &tool_name,
                &tool.operation_class,
                &request_hash,
                None,
                "failed",
                &public_error_code(&error),
                started.elapsed().as_millis() as u64,
            )?;
''',
    '''            write_mcp_receipt(
                &state,
                McpReceiptInput {
                    connection_id: &request.connection_id,
                    tool_name: &tool_name,
                    operation_class: &tool.operation_class,
                    request_hash: &request_hash,
                    result_hash: None,
                    outcome: "failed",
                    result_code: public_error_code(&error),
                    duration_ms: started.elapsed().as_millis() as u64,
                },
            )?;
''',
)

replace_once(
    integration_path,
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
    let mut items = Vec::new();
''',
    '''fn build_guidance(context: GuidanceContext<'_>) -> Vec<AgentGuidanceItem> {
    let GuidanceContext {
        health,
        knowledge,
        models,
        operational,
        clouds,
        backups,
        integrations,
        last_user_prompt_at_utc,
    } = context;
    let mut items = Vec::new();
''',
)

replace_once(
    integration_path,
    '''    items.sort_by(|left, right| right.priority.cmp(&left.priority));
''',
    '''    items.sort_by_key(|item| std::cmp::Reverse(item.priority));
''',
)

replace_once(
    integration_path,
    '''fn write_mcp_receipt(
    state: &AppState,
    connection_id: &str,
    tool_name: &str,
    operation_class: &str,
    request_hash: &str,
    result_hash: Option<&str>,
    outcome: &str,
    result_code: &str,
    duration_ms: u64,
) -> Result<()> {
    state.connection()?.execute(
        "INSERT INTO agent_mcp_invocation_receipts (receipt_id,connection_id,tool_name,operation_class,request_hash,result_hash,outcome,result_code,duration_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            Uuid::new_v4().to_string(),
            connection_id,
            tool_name,
            operation_class,
            request_hash,
            result_hash,
            outcome,
            truncate_chars(result_code, 160),
            duration_ms.min(i64::MAX as u64) as i64,
        ],
    )?;
    Ok(())
}
''',
    '''fn write_mcp_receipt(state: &AppState, input: McpReceiptInput<'_>) -> Result<()> {
    state.connection()?.execute(
        "INSERT INTO agent_mcp_invocation_receipts (receipt_id,connection_id,tool_name,operation_class,request_hash,result_hash,outcome,result_code,duration_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            Uuid::new_v4().to_string(),
            input.connection_id,
            input.tool_name,
            input.operation_class,
            input.request_hash,
            input.result_hash,
            input.outcome,
            truncate_chars(&input.result_code, 160),
            input.duration_ms.min(i64::MAX as u64) as i64,
        ],
    )?;
    Ok(())
}
''',
)

replace_once(
    integration_path,
    '''            .last()
            .context("MCP event stream did not contain JSON data")?;
''',
    '''            .next_back()
            .context("MCP event stream did not contain JSON data")?;
''',
)

replace_once(
    "crates/homeserver-service/src/agent_runtime.rs",
    '''    agent_integrations::record_context_receipt(
        &state,
        Some(&thread_id),
        &request.prompt,
        &source_keys,
        knowledge
            .as_ref()
            .map(|result| result.hits.len())
            .unwrap_or(0),
        operational
            .as_ref()
            .map(|result| result.records.len())
            .unwrap_or(0),
        &mcp_tool_names,
        &context_hash,
        if inference.is_some() {
            "completed"
        } else {
            "unavailable"
        },
        None,
    )?;
''',
    '''    agent_integrations::record_context_receipt(
        &state,
        agent_integrations::ContextReceiptInput {
            thread_id: Some(&thread_id),
            prompt: &request.prompt,
            source_keys: &source_keys,
            knowledge_hits: knowledge
                .as_ref()
                .map(|result| result.hits.len())
                .unwrap_or(0),
            operational_records: operational
                .as_ref()
                .map(|result| result.records.len())
                .unwrap_or(0),
            mcp_tools: &mcp_tool_names,
            context_hash: &context_hash,
            inference_state: if inference.is_some() {
                "completed"
            } else {
                "unavailable"
            },
            failure_code: None,
        },
    )?;
''',
)

replace_once(
    "scripts/validate-unified-agent-orchestration.py",
    '''    "ChronoDuration::hours(18)",
])
''',
    '''    "ChronoDuration::hours(18)",
    "pub(crate) struct ContextReceiptInput",
    "struct GuidanceContext",
    "struct McpReceiptInput",
    "sort_by_key(|item| std::cmp::Reverse(item.priority))",
    ".next_back()",
])
''',
)

print("Phase 22 strict lint refactor applied.")
