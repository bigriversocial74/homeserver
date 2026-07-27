#!/usr/bin/env python3
"""One-time behavioral test integration for Phase 5B Agent Workspace."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    target = ROOT / path
    content = target.read_text(encoding="utf-8")
    count = content.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one test integration anchor, found {count}: {old[:100]!r}")
    target.write_text(content.replace(old, new, 1), encoding="utf-8", newline="\n")


runtime = ROOT / "crates/homeserver-service/src/agent_runtime.rs"
content = runtime.read_text(encoding="utf-8")
if "fn supervised_agent_schema_enforces_one_approval_and_idempotency_record" not in content:
    content += r'''

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database;
    use tempfile::{tempdir, TempDir};

    fn initialized_connection() -> (TempDir, Connection) {
        let directory = tempdir().unwrap();
        let connection = database::initialize(&directory.path().join("agent-runtime.sqlite3")).unwrap();
        initialize(&connection).unwrap();
        (directory, connection)
    }

    #[test]
    fn supervised_agent_migration_is_self_consistent() {
        let (_directory, connection) = initialized_connection();
        health_check(&connection).unwrap();
        let table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('agent_goals','agent_plans','agent_approvals','agent_execution_receipts','world_missions','world_conversations','world_follow_ups')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 7);
    }

    #[test]
    fn bounded_action_arguments_reject_open_world_operations() {
        assert!(validate_action_arguments("commerce.order.create", json!({})).is_err());
        assert!(validate_action_arguments("cloud.sync_all", json!({"command":"whoami"})).is_err());
        assert!(validate_action_arguments("backup.create", json!({"path":"C:/"})).is_err());
        let report = validate_action_arguments(
            "report.save",
            json!({"title":"  Weekly operations  ","content_markdown":"  Evidence only.  "}),
        )
        .unwrap();
        assert_eq!(report["title"], "Weekly operations");
        assert_eq!(report["content_markdown"], "Evidence only.");
    }

    #[test]
    fn world_mode_operations_are_closed_world() {
        let defaults = normalize_world_operations(&[], false).unwrap();
        assert_eq!(
            defaults,
            vec![
                "discover".to_owned(),
                "compare".to_owned(),
                "prepare_recommendation".to_owned()
            ]
        );
        assert!(normalize_world_operations(&["purchase".to_owned()], false).is_err());
        assert_eq!(
            normalize_world_operations(&["purchase".to_owned()], true).unwrap(),
            vec!["purchase".to_owned()]
        );
    }

    #[test]
    fn supervised_agent_schema_enforces_one_approval_and_idempotency_record() {
        let (_directory, connection) = initialized_connection();
        let plan_id = Uuid::new_v4().to_string();
        let approval_request_id = Uuid::new_v4().to_string();
        let approval_id = Uuid::new_v4().to_string();
        let now = now_string();
        let expires = (Utc::now() + ChronoDuration::minutes(30))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let plan_hash = "a".repeat(64);
        connection
            .execute(
                "INSERT INTO agent_plans (plan_id,requested_by_type,requested_by_id,title,rationale,action_type,arguments_json,dataset_keys_json,risk_level,state,plan_hash,fresh_state_token,expires_at_utc) VALUES (?1,'local_user','test','Test backup','Schema contract','backup.create','{}','[]','low','approved',?2,'local-state',?3)",
                params![plan_id, plan_hash, expires],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_approval_requests (approval_request_id,plan_id,plan_hash,state,risk_summary,requested_at_utc,expires_at_utc) VALUES (?1,?2,?3,'approved','One bounded backup',?4,?5)",
                params![approval_request_id, plan_id, plan_hash, now, expires],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_approvals (approval_id,approval_request_id,plan_id,plan_hash,approved_by,approved_at_utc,expires_at_utc) VALUES (?1,?2,?3,?4,'local_test',?5,?6)",
                params![approval_id, approval_request_id, plan_id, plan_hash, now, expires],
            )
            .unwrap();
        assert!(connection
            .execute(
                "INSERT INTO agent_approvals (approval_id,approval_request_id,plan_id,plan_hash,approved_by,approved_at_utc,expires_at_utc) VALUES (?1,?2,?3,?4,'duplicate',?5,?6)",
                params![Uuid::new_v4().to_string(), approval_request_id, plan_id, plan_hash, now, expires],
            )
            .is_err());
        connection
            .execute(
                "INSERT INTO agent_action_idempotency (idempotency_key,plan_id,state,created_at_utc,updated_at_utc) VALUES ('agent:first',?1,'executing',?2,?2)",
                params![plan_id, now],
            )
            .unwrap();
        assert!(connection
            .execute(
                "INSERT INTO agent_action_idempotency (idempotency_key,plan_id,state,created_at_utc,updated_at_utc) VALUES ('agent:second',?1,'executing',?2,?2)",
                params![plan_id, now],
            )
            .is_err());
    }
}
'''
runtime.write_text(content, encoding="utf-8", newline="\n")

smoke = "scripts/smoke-test-service.ps1"
replace_once(
    smoke,
    '''    $mcp = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/mcp" -TimeoutSec 10
    if (-not $mcp.local_only -or -not $mcp.read_only -or $mcp.endpoint -ne "$apiBase/mcp" -or $mcp.state -ne "waiting_for_client") {
        throw "Fresh local MCP runtime did not initialize at the fixed read-only loopback boundary"
    }
    if (@($mcp.clients).Count -ne 0 -or @($mcp.tools).Count -ne 5) {
        throw "Fresh local MCP runtime client or tool catalog is invalid"
    }
''',
    '''    $workspace = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/agent/workspace" -TimeoutSec 15
    if (-not $workspace.local_only -or @($workspace.goals).Count -ne 0 -or @($workspace.plans).Count -ne 0 -or @($workspace.approvals).Count -ne 0 -or @($workspace.missions).Count -ne 0) {
        throw "Fresh Agent Workspace did not initialize with an empty local-only control plane"
    }
    if ("approval_gated_execute" -notin @($workspace.capabilities)) {
        throw "Agent Workspace is missing its supervised execution capability marker"
    }
    $operationalSource = @($workspace.data_sources) | Where-Object { $_.key -eq "operational_data" } | Select-Object -First 1
    $worldSource = @($workspace.data_sources) | Where-Object { $_.key -eq "world_canvas" } | Select-Object -First 1
    if (-not $operationalSource -or $operationalSource.state -ne "planned_phase_5c" -or -not $worldSource -or $worldSource.state -ne "mission_drafting") {
        throw "Agent Workspace did not expose the Phase 5C and World Mission boundaries"
    }

    $goalBody = @{
        title = "Improve weekday operations"
        description = "Match current HomeServer evidence to a measurable operating goal."
        target_metric = "Weekday operational result"
        target_value = "+15%"
        target_date = $null
        connection_ids = @()
        dataset_keys = @("system", "goals")
        constraints = @{}
        allowed_actions = @("backup.create", "model.health_test", "cloud.sync_connection", "cloud.sync_all", "report.save")
        approval_policy = "always"
    } | ConvertTo-Json -Depth 8 -Compress
    $goal = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/goals" -ContentType "application/json" -Body $goalBody -TimeoutSec 15
    if ($goal.state -ne "active" -or $goal.title -ne "Improve weekday operations") {
        throw "Agent Workspace goal creation failed"
    }

    $promptBody = @{
        thread_id = $null
        mode = "analyze"
        prompt = "Summarize the current local operating context and identify unavailable data."
        connection_ids = @()
        dataset_keys = @("system", "goals", "knowledge")
        goal_ids = @($goal.goal_id)
        knowledge_query = "local operating context"
        model = $null
        proposed_action = $null
        world_mission = $null
    } | ConvertTo-Json -Depth 10 -Compress
    $promptResult = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/prompt" -ContentType "application/json" -Body $promptBody -TimeoutSec 30
    if (-not $promptResult.thread_id -or $promptResult.assistant_message.role -ne "assistant" -or $promptResult.approvals_required) {
        throw "Agent Workspace grounded prompt did not complete safely"
    }

    $missionBody = @{
        thread_id = $promptResult.thread_id
        goal_id = $goal.goal_id
        connection_id = $null
        world_agent_id = "ci-world-agent"
        title = "Investigate nearby operating options"
        objective = "Discover and compare qualifying Store Canvas options, then prepare a recommendation."
        allowed_operations = @("discover", "visit_store_canvas", "ask_questions", "compare", "prepare_recommendation", "schedule_follow_up", "close_conversation")
        prohibited_operations = @("purchase", "payment", "claim", "redemption", "share_private_profile", "accept_recurring_commitment", "publish_campaign", "bulk_message")
        limits = @{ maximum_visits = 5; maximum_messages = 10; distance_limit_miles = 8 }
        disclosure_policy = @{ minimum_necessary = $true; private_reasoning_local = $true }
        expires_minutes = 240
    } | ConvertTo-Json -Depth 10 -Compress
    $mission = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/world/missions" -ContentType "application/json" -Body $missionBody -TimeoutSec 15
    if ($mission.state -ne "draft" -or "purchase" -notin @($mission.prohibited_operations)) {
        throw "World Mission draft did not preserve its no-dispatch safety contract"
    }
    $missionCancelBody = @{ mission_id = $mission.mission_id; confirmation = "CANCEL" } | ConvertTo-Json -Compress
    $cancelledMission = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/world/missions/cancel" -ContentType "application/json" -Body $missionCancelBody -TimeoutSec 15
    if ($cancelledMission.state -ne "cancelled") { throw "Undispatched World Mission cancellation failed" }

    $invalidPlanBody = @{
        thread_id = $promptResult.thread_id
        title = "Invalid commerce request"
        rationale = "Prove commerce writes remain unavailable."
        action_type = "commerce.order.create"
        arguments = @{}
        connection_id = $null
        goal_id = $goal.goal_id
        dataset_keys = @("system")
        expires_minutes = 30
    } | ConvertTo-Json -Depth 8 -Compress
    $invalidPlan = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/plans" -ContentType "application/json" -Body $invalidPlanBody -TimeoutSec 15
    if ($invalidPlan.StatusCode -ne 400) { throw "Expected open-world commerce plan rejection, received HTTP $($invalidPlan.StatusCode)" }

    $backupPlanBody = @{
        thread_id = $promptResult.thread_id
        title = "Create supervised CI backup"
        rationale = "Validate one-use approval, execution, idempotency, and receipts."
        action_type = "backup.create"
        arguments = @{ note = "Agent Workspace CI backup" }
        connection_id = $null
        goal_id = $goal.goal_id
        dataset_keys = @("system", "goals")
        expires_minutes = 30
    } | ConvertTo-Json -Depth 8 -Compress
    $backupPlan = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/plans" -ContentType "application/json" -Body $backupPlanBody -TimeoutSec 15
    if ($backupPlan.state -ne "awaiting_approval" -or -not $backupPlan.plan_hash) {
        throw "Supervised backup plan was not created with an approval-bound hash"
    }
    $earlyExecuteBody = @{ plan_id = $backupPlan.plan_id; confirmation = "EXECUTE"; reason = $null } | ConvertTo-Json -Compress
    $earlyExecute = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/plans/execute" -ContentType "application/json" -Body $earlyExecuteBody -TimeoutSec 15
    if ($earlyExecute.StatusCode -ne 400) { throw "Unapproved plan execution was not rejected" }
    $approveBody = @{ plan_id = $backupPlan.plan_id; confirmation = "APPROVE"; reason = "CI local approval" } | ConvertTo-Json -Compress
    $approved = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/approvals/approve" -ContentType "application/json" -Body $approveBody -TimeoutSec 15
    if ($approved.plan.state -ne "approved" -or $approved.approval.state -ne "approved") {
        throw "Local one-use plan approval failed"
    }
    $receipt = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/plans/execute" -ContentType "application/json" -Body $earlyExecuteBody -TimeoutSec 120
    if ($receipt.state -ne "completed" -or $receipt.result_code -ne "backup_created") {
        throw "Approved bounded backup execution failed"
    }
    $repeatedReceipt = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/agent/plans/execute" -ContentType "application/json" -Body $earlyExecuteBody -TimeoutSec 30
    if ($repeatedReceipt.receipt_id -ne $receipt.receipt_id) {
        throw "Repeated execution did not return the existing idempotent receipt"
    }

    $workspace = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/agent/workspace" -TimeoutSec 15
    $completedPlan = @($workspace.plans) | Where-Object { $_.plan_id -eq $backupPlan.plan_id } | Select-Object -First 1
    if (-not $completedPlan -or $completedPlan.state -ne "completed" -or @($workspace.receipts).Count -ne 1) {
        throw "Agent Workspace did not persist completed plan and receipt state"
    }

    $mcp = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/mcp" -TimeoutSec 10
    if (-not $mcp.local_only -or $mcp.read_only -or -not $mcp.request_only -or $mcp.endpoint -ne "$apiBase/mcp" -or $mcp.state -ne "waiting_for_client") {
        throw "Fresh local MCP runtime did not initialize at the fixed supervised request-only boundary"
    }
    if (@($mcp.clients).Count -ne 0 -or @($mcp.tools).Count -ne 14) {
        throw "Fresh supervised MCP runtime client or tool catalog is invalid"
    }
''',
)
replace_once(
    smoke,
    '''    $clientBody = @{ display_name = "HomeServer CI MCP"; scopes = @("system.read", "cloud.read", "models.read", "knowledge.search", "knowledge.read"); expires_days = 30 } | ConvertTo-Json -Compress
''',
    '''    $clientBody = @{ display_name = "HomeServer CI MCP"; scopes = @("system.read", "cloud.read", "models.read", "knowledge.search", "knowledge.read", "agents.read", "agents.request", "world.request"); expires_days = 30 } | ConvertTo-Json -Compress
''',
)
replace_once(
    smoke,
    '''    if (@($tools.result.tools).Count -ne 5) { throw "MCP read-only tool catalog is incomplete" }
    foreach ($tool in @($tools.result.tools)) {
        if (-not $tool.annotations.readOnlyHint -or $tool.annotations.destructiveHint -or $tool.annotations.openWorldHint) {
            throw "MCP tool '$($tool.name)' is missing enforced read-only annotations"
        }
    }
''',
    '''    if (@($tools.result.tools).Count -ne 14) { throw "MCP supervised tool catalog is incomplete" }
    $requestToolNames = @("homeserver_agent_prompt", "homeserver_agent_plan_submit", "homeserver_agent_plan_cancel", "homeserver_world_mission_draft")
    $forbiddenMcpTools = @("homeserver_agent_plan_approve", "homeserver_agent_plan_execute", "homeserver_world_mission_dispatch")
    foreach ($forbiddenTool in $forbiddenMcpTools) {
        if ($forbiddenTool -in @($tools.result.tools.name)) { throw "MCP exposed prohibited authority tool '$forbiddenTool'" }
    }
    foreach ($tool in @($tools.result.tools)) {
        if ($tool.annotations.destructiveHint -or $tool.annotations.openWorldHint) {
            throw "MCP tool '$($tool.name)' is missing closed-world annotations"
        }
        if ($tool.name -in $requestToolNames) {
            if ($tool.annotations.readOnlyHint -or -not $tool.annotations.requestOnly) {
                throw "MCP request tool '$($tool.name)' is not marked request-only"
            }
        }
        elseif (-not $tool.annotations.readOnlyHint) {
            throw "MCP read tool '$($tool.name)' is missing read-only annotation"
        }
    }
''',
)
replace_once(
    smoke,
    '''    if ($statusTool.result.structuredContent.state -ne "running" -or $statusTool.result.isError) {
        throw "MCP HomeServer status tool failed"
    }
''',
    '''    if ($statusTool.result.structuredContent.state -ne "running" -or $statusTool.result.isError) {
        throw "MCP HomeServer status tool failed"
    }
    $mcpPromptBody = @{ jsonrpc = "2.0"; id = 4; method = "tools/call"; params = @{ name = "homeserver_agent_prompt"; arguments = @{ thread_id = $null; mode = "ask"; prompt = "Describe the supervised MCP boundary."; connection_ids = @(); dataset_keys = @("system"); goal_ids = @(); knowledge_query = $null; model = $null; proposed_action = $null; world_mission = $null } } } | ConvertTo-Json -Depth 12 -Compress
    $mcpPrompt = Invoke-RestMethod -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $mcpPromptBody -TimeoutSec 30
    if ($mcpPrompt.result.isError -or $mcpPrompt.result.structuredContent.approvals_required) {
        throw "MCP request-only Agent Workspace prompt failed"
    }
    $mcpPlanBody = @{ jsonrpc = "2.0"; id = 5; method = "tools/call"; params = @{ name = "homeserver_agent_plan_submit"; arguments = @{ thread_id = $null; title = "MCP requested report"; rationale = "Validate request-only plan ownership."; action_type = "report.save"; arguments = @{ title = "MCP request test"; content_markdown = "This report must not be saved without local approval." }; connection_id = $null; goal_id = $goal.goal_id; dataset_keys = @("system", "goals"); expires_minutes = 30 } } } | ConvertTo-Json -Depth 14 -Compress
    $mcpPlan = Invoke-RestMethod -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $mcpPlanBody -TimeoutSec 20
    if ($mcpPlan.result.isError -or $mcpPlan.result.structuredContent.state -ne "awaiting_approval" -or $mcpPlan.result.structuredContent.requested_by_type -ne "mcp_client") {
        throw "MCP could not submit a request-only supervised plan"
    }
    $mcpCancelBody = @{ jsonrpc = "2.0"; id = 6; method = "tools/call"; params = @{ name = "homeserver_agent_plan_cancel"; arguments = @{ plan_id = $mcpPlan.result.structuredContent.plan_id } } } | ConvertTo-Json -Depth 10 -Compress
    $mcpCancelled = Invoke-RestMethod -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $mcpCancelBody -TimeoutSec 20
    if ($mcpCancelled.result.structuredContent.state -ne "cancelled") {
        throw "MCP client could not cancel its own unexecuted plan"
    }
    $mcpMissionBody = @{ jsonrpc = "2.0"; id = 7; method = "tools/call"; params = @{ name = "homeserver_world_mission_draft"; arguments = @{ thread_id = $null; goal_id = $goal.goal_id; connection_id = $null; world_agent_id = "ci-mcp-world-agent"; title = "MCP World Mission draft"; objective = "Compare options and return a recommendation."; allowed_operations = @("discover", "compare", "prepare_recommendation"); prohibited_operations = @("purchase", "payment", "claim", "redemption", "share_private_profile", "accept_recurring_commitment", "publish_campaign", "bulk_message"); limits = @{ maximum_visits = 3 }; disclosure_policy = @{ minimum_necessary = $true }; expires_minutes = 120 } } } | ConvertTo-Json -Depth 14 -Compress
    $mcpMission = Invoke-RestMethod -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $mcpMissionBody -TimeoutSec 20
    if ($mcpMission.result.isError -or $mcpMission.result.structuredContent.state -ne "draft") {
        throw "MCP World Mission request did not remain a local draft"
    }
''',
)

print("Phase 5B behavioral tests and service smoke coverage applied.")
