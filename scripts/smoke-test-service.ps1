param(
    [string]$ServiceBinary = "target/release/microgifter-homeserver-service.exe",
    [string]$McpBinary = "target/release/microgifter-homeserver-mcp.exe"
)

$ErrorActionPreference = "Stop"
$binaryPath = (Resolve-Path $ServiceBinary).Path
$mcpBinaryPath = (Resolve-Path $McpBinary).Path
$primaryDataDirectory = Join-Path $env:RUNNER_TEMP ("microgifter-homeserver-primary-" + [guid]::NewGuid().ToString("N"))
$freshDataDirectory = Join-Path $env:RUNNER_TEMP ("microgifter-homeserver-recovery-" + [guid]::NewGuid().ToString("N"))
$exportedPackage = Join-Path $env:RUNNER_TEMP ("microgifter-homeserver-export-" + [guid]::NewGuid().ToString("N") + ".mghbackup")
$env:MG_HOMESERVER_DATA_DIR = $primaryDataDirectory
$env:MG_HOMESERVER_NAME = "CI HomeServer"
$process = $null
$apiBase = "http://127.0.0.1:47831"
$controlHeaders = @{ "X-MG-Local-Client" = "microgifter-control-center-v1" }

function ConvertTo-Base64Url {
    param([Parameter(Mandatory = $true)][string]$Value)
    return [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Value)).TrimEnd("=").Replace("+", "-").Replace("/", "_")
}

function Start-HomeServerProcess {
    $script:process = Start-Process -FilePath $binaryPath -ArgumentList "console" -PassThru -WindowStyle Hidden
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if ($script:process.HasExited) {
            throw "HomeServer console process exited before becoming ready with code $($script:process.ExitCode)"
        }
        try {
            $health = Invoke-WebRequest -UseBasicParsing -Uri "$apiBase/healthz" -TimeoutSec 2
            if ($health.StatusCode -eq 204) {
                return
            }
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }
    throw "HomeServer console service did not become healthy"
}

function Stop-HomeServerProcess {
    if ($script:process -and -not $script:process.HasExited) {
        Stop-Process -Id $script:process.Id -Force -ErrorAction SilentlyContinue
        $script:process.WaitForExit(5000) | Out-Null
    }
    $script:process = $null
}

try {
    Start-HomeServerProcess

    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 3
    if ($status.state -ne "running") {
        throw "Expected running state, received '$($status.state)'"
    }
    if ($status.database -ne "ready") {
        throw "Expected ready database, received '$($status.database)'"
    }
    if ($status.server_name -ne "CI HomeServer") {
        throw "Expected sanitized CI server name, received '$($status.server_name)'"
    }
    if ($status.backup -ne "ready") {
        throw "Expected ready backup service, received '$($status.backup)'"
    }

    $models = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/models" -TimeoutSec 15
    if ($models.runtime.api_url -ne "http://127.0.0.1:11434") {
        throw "Model Center runtime URL is not fixed to the approved loopback endpoint"
    }
    if ($models.runtime.state -notin @("running", "not_running")) {
        throw "Unexpected Model Center runtime state '$($models.runtime.state)'"
    }
    if (-not $models.local_only -or @($models.catalog).Count -ne 5) {
        throw "Model Center local-only catalog is incomplete"
    }
    if ([int]$models.settings.context_size -lt 512 -or [int]$models.settings.max_download_gb -lt 1) {
        throw "Model Center bounded settings were not initialized"
    }
    $unapprovedBody = @{ model = "unapproved/model:latest" } | ConvertTo-Json -Compress
    $unapproved = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/models/pull" -ContentType "application/json" -Body $unapprovedBody -TimeoutSec 10
    if ($unapproved.StatusCode -ne 422) {
        throw "Expected unapproved local model rejection, received HTTP $($unapproved.StatusCode)"
    }

    $vault = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/vault" -TimeoutSec 15
    if (-not $vault.local_only -or -not $vault.extraction.local_only) {
        throw "Knowledge Vault extraction did not initialize as local-only"
    }
    foreach ($requiredExtension in @("pdf", "docx", "png", "jpg", "tiff")) {
        if ($requiredExtension -notin @($vault.supported_extensions)) {
            throw "Knowledge Vault is missing the $requiredExtension extraction type"
        }
    }
    if ([int]$vault.extraction.total_pages -ne 0 -or @($vault.extraction.documents).Count -ne 0) {
        throw "Fresh document extraction catalog is not empty"
    }
    if ($vault.extraction.runtime.tesseract_install_command -ne "winget install --id tesseract-ocr.tesseract --exact --scope machine") {
        throw "Tesseract installation guidance is not fixed to the approved package"
    }
    if ($vault.extraction.runtime.poppler_install_command -ne "winget install --id oschwartz10612.Poppler --exact --scope machine") {
        throw "Poppler installation guidance is not fixed to the approved package"
    }

    $semantic = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/vault/semantic" -TimeoutSec 15
    if (-not $semantic.local_only -or $semantic.state -ne "not_configured") {
        throw "Semantic Knowledge Vault did not initialize in the safe unconfigured state"
    }
    if ([int]$semantic.chunk_count -ne 0 -or [int]$semantic.ready_documents -ne 0) {
        throw "Fresh semantic Knowledge Vault unexpectedly contains vectors"
    }
    $semanticRebuild = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/vault/semantic/rebuild" -ContentType "application/json" -Body '{"force":false}' -TimeoutSec 10
    if ($semanticRebuild.StatusCode -ne 422) {
        throw "Expected semantic rebuild to require a configured embedding model, received HTTP $($semanticRebuild.StatusCode)"
    }
    $keywordSearchBody = @{ query = "local policy"; mode = "keyword"; limit = 20 } | ConvertTo-Json -Compress
    $keywordSearch = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/vault/semantic/search" -ContentType "application/json" -Body $keywordSearchBody -TimeoutSec 10
    if ($keywordSearch.mode -ne "keyword" -or $keywordSearch.semantic_available -or @($keywordSearch.hits).Count -ne 0) {
        throw "Fresh semantic Knowledge Vault keyword fallback is invalid"
    }

    $operational = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/operational-data" -TimeoutSec 15
    if (-not $operational.local_only -or -not $operational.provider_authoritative -or -not $operational.imported_data_is_untrusted_evidence) {
        throw "Operational data authority or local evidence boundary was not initialized"
    }
    if ([int]$operational.provider_manifests -ne 1 -or [int]$operational.enabled_grants -ne 0 -or [int]$operational.imported_records -ne 0 -or [int]$operational.imported_events -ne 0) {
        throw "Fresh operational data manifest or evidence state is invalid"
    }
    if (@($operational.datasets).Count -ne 0 -or @($operational.recent_runs).Count -ne 0) {
        throw "Fresh HomeServer unexpectedly exposes connection datasets or import history"
    }
    $emptyOperationalQueryBody = @{ connection_id = $null; dataset_key = $null; source_object_type = $null; limit = 25 } | ConvertTo-Json -Compress
    $emptyOperationalQuery = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/operational-data/query" -ContentType "application/json" -Body $emptyOperationalQueryBody -TimeoutSec 15
    if ([int]$emptyOperationalQuery.available_records -ne 0 -or @($emptyOperationalQuery.records).Count -ne 0 -or -not $emptyOperationalQuery.provider_authoritative -or -not $emptyOperationalQuery.imported_data_is_untrusted_evidence) {
        throw "Fresh operational evidence query did not preserve its empty provider-authoritative boundary"
    }
    $invalidOperationalImportBody = @{
        connection_id = [guid]::NewGuid().ToString()
        provider_key = "microgifter"
        tenant_id = "wrong-tenant"
        site_id = "wrong-site"
        dataset_key = "merchant.products"
        import_mode = "snapshot"
        cursor_after = $null
        source_revision = "ci-invalid"
        records = @(@{ source_object_type = "product"; source_object_id = "ci-invalid"; source_revision = "1"; source_updated_at_utc = $null; payload = @{ name = "Rejected evidence" } })
        events = @()
    } | ConvertTo-Json -Depth 10 -Compress
    $invalidOperationalImport = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/operational-data/import" -ContentType "application/json" -Body $invalidOperationalImportBody -TimeoutSec 15
    if ($invalidOperationalImport.StatusCode -ne 422) {
        throw "Expected unknown-connection operational import rejection, received HTTP $($invalidOperationalImport.StatusCode)"
    }

    $workspace = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/agent/workspace" -TimeoutSec 15
    if (-not $workspace.local_only -or @($workspace.goals).Count -ne 0 -or @($workspace.plans).Count -ne 0 -or @($workspace.approvals).Count -ne 0 -or @($workspace.missions).Count -ne 0) {
        throw "Fresh Agent Workspace did not initialize with an empty local-only control plane"
    }
    if ("approval_gated_execute" -notin @($workspace.capabilities) -or "operational_data_evidence" -notin @($workspace.capabilities)) {
        throw "Agent Workspace is missing supervised execution or operational evidence capability markers"
    }
    $operationalSource = @($workspace.data_sources) | Where-Object { $_.key -eq "operational_data" } | Select-Object -First 1
    $worldSource = @($workspace.data_sources) | Where-Object { $_.key -eq "world_canvas" } | Select-Object -First 1
    if (-not $operationalSource -or $operationalSource.state -ne "empty" -or -not $worldSource -or $worldSource.state -ne "mission_drafting") {
        throw "Agent Workspace did not expose the operational evidence and World Mission boundaries"
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
    $initializeBody = @{ jsonrpc = "2.0"; id = 1; method = "initialize"; params = @{ protocolVersion = "2025-11-25"; capabilities = @{}; clientInfo = @{ name = "HomeServer CI"; version = "1.0" } } } | ConvertTo-Json -Depth 8 -Compress
    $unauthorized = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Uri "$apiBase/mcp" -ContentType "application/json" -Body $initializeBody -TimeoutSec 10
    if ($unauthorized.StatusCode -ne 401) {
        throw "Expected unauthenticated MCP rejection, received HTTP $($unauthorized.StatusCode)"
    }
    $clientBody = @{ display_name = "HomeServer CI MCP"; scopes = @("system.read", "cloud.read", "models.read", "knowledge.search", "knowledge.read", "agents.read", "agents.request", "world.request"); expires_days = 30 } | ConvertTo-Json -Compress
    $credential = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/mcp/clients" -ContentType "application/json" -Body $clientBody -TimeoutSec 10
    if ($credential.token -notmatch '^mghs_mcp_[A-Za-z0-9_-]+$' -or $credential.client.token_hint -eq $credential.token) {
        throw "MCP client credential was not created with a one-time bounded token"
    }
    $mcpHeaders = @{ Authorization = "Bearer $($credential.token)" }
    $initialized = Invoke-RestMethod -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $initializeBody -TimeoutSec 15
    if ($initialized.result.protocolVersion -ne "2025-11-25" -or $initialized.result.serverInfo.name -ne "Microgifter HomeServer") {
        throw "MCP initialize negotiation failed"
    }
    $toolsBody = @{ jsonrpc = "2.0"; id = 2; method = "tools/list"; params = @{} } | ConvertTo-Json -Depth 6 -Compress
    $tools = Invoke-RestMethod -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $toolsBody -TimeoutSec 15
    if (@($tools.result.tools).Count -ne 14) { throw "MCP supervised tool catalog is incomplete" }
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
    $statusToolBody = @{ jsonrpc = "2.0"; id = 3; method = "tools/call"; params = @{ name = "homeserver_status"; arguments = @{} } } | ConvertTo-Json -Depth 8 -Compress
    $statusTool = Invoke-RestMethod -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $statusToolBody -TimeoutSec 15
    if ($statusTool.result.structuredContent.state -ne "running" -or $statusTool.result.isError) {
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
    $browserRequest = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers (@{ Authorization = "Bearer $($credential.token)"; Origin = "https://example.invalid" }) -Uri "$apiBase/mcp" -ContentType "application/json" -Body $toolsBody -TimeoutSec 10
    if ($browserRequest.StatusCode -ne 403) {
        throw "Browser-originated MCP request was not rejected"
    }
    $previousMcpToken = $env:MG_HOMESERVER_MCP_TOKEN
    try {
        $env:MG_HOMESERVER_MCP_TOKEN = $credential.token
        $bridgeOutput = $initializeBody | & $mcpBinaryPath
        if ($LASTEXITCODE -ne 0) { throw "Packaged MCP stdio bridge exited with code $LASTEXITCODE" }
        $bridgeResult = ($bridgeOutput | Out-String).Trim() | ConvertFrom-Json
        if ($bridgeResult.result.protocolVersion -ne "2025-11-25") {
            throw "Packaged MCP stdio bridge did not complete initialize"
        }
    }
    finally {
        $env:MG_HOMESERVER_MCP_TOKEN = $previousMcpToken
    }
    $revokeBody = @{ client_id = $credential.client.client_id; confirmation = "REVOKE" } | ConvertTo-Json -Compress
    $revoked = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/mcp/clients/revoke" -ContentType "application/json" -Body $revokeBody -TimeoutSec 10
    if ($revoked.client.state -ne "revoked") { throw "MCP client revocation did not persist" }
    $revokedRequest = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $mcpHeaders -Uri "$apiBase/mcp" -ContentType "application/json" -Body $toolsBody -TimeoutSec 10
    if ($revokedRequest.StatusCode -ne 401) { throw "Revoked MCP token remained authorized" }

    $manualBody = @{
        kind = "manual"
        passphrase = $null
        note = "CI manual backup"
    } | ConvertTo-Json -Compress
    $manual = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/create" -ContentType "application/json" -Body $manualBody -TimeoutSec 90
    if ($manual.backup.kind -ne "manual" -or $manual.backup.state -ne "ready") {
        throw "Manual encrypted backup was not created correctly"
    }
    if (-not (Test-Path $manual.backup.storage_path)) {
        throw "Manual backup package was not written"
    }

    $verifyManualBody = @{
        backup_id = $manual.backup.backup_id
        passphrase = $null
        confirmation = $null
    } | ConvertTo-Json -Compress
    $verifiedManual = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/verify" -ContentType "application/json" -Body $verifyManualBody -TimeoutSec 90
    if ($verifiedManual.backup.state -ne "verified") {
        throw "Manual backup verification did not persist"
    }

    $recoveryPassphrase = "correct horse battery staple 2026"
    $recoveryBody = @{
        kind = "recovery"
        passphrase = $recoveryPassphrase
        note = "CI portable recovery package"
    } | ConvertTo-Json -Compress
    $recovery = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/create" -ContentType "application/json" -Body $recoveryBody -TimeoutSec 90
    if ($recovery.backup.kind -ne "recovery" -or $recovery.backup.state -ne "ready") {
        throw "Recovery package was not created correctly"
    }
    if (-not (Test-Path $recovery.backup.storage_path)) {
        throw "Recovery package was not written"
    }

    $wrongPassphraseBody = @{
        backup_id = $recovery.backup.backup_id
        passphrase = "wrong recovery passphrase value"
        confirmation = $null
    } | ConvertTo-Json -Compress
    $wrongPassphrase = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/verify" -ContentType "application/json" -Body $wrongPassphraseBody -TimeoutSec 90
    if ($wrongPassphrase.StatusCode -ne 422) {
        throw "Expected wrong recovery passphrase rejection, received HTTP $($wrongPassphrase.StatusCode)"
    }

    $verifyRecoveryBody = @{
        backup_id = $recovery.backup.backup_id
        passphrase = $recoveryPassphrase
        confirmation = $null
    } | ConvertTo-Json -Compress
    $verifiedRecovery = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/verify" -ContentType "application/json" -Body $verifyRecoveryBody -TimeoutSec 90
    if ($verifiedRecovery.backup.state -ne "verified") {
        throw "Recovery package verification did not persist"
    }

    Invoke-WebRequest -UseBasicParsing -Headers $controlHeaders -Uri "$apiBase/v1/backups/$($recovery.backup.backup_id)/package" -OutFile $exportedPackage -TimeoutSec 90
    if (-not (Test-Path $exportedPackage) -or (Get-Item $exportedPackage).Length -le 12) {
        throw "Portable recovery package export was not produced"
    }

    $catalog = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/backups" -TimeoutSec 5
    if (@($catalog.backups).Count -lt 2 -or [int]$catalog.retention_count -ne 14 -or [int]$catalog.interval_hours -ne 24) {
        throw "Backup catalog or policy is incomplete"
    }

    $restoreBody = @{
        backup_id = $manual.backup.backup_id
        passphrase = $null
        confirmation = "RESTORE"
    } | ConvertTo-Json -Compress
    $staged = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/stage-restore" -ContentType "application/json" -Body $restoreBody -TimeoutSec 90
    if (-not $staged.restart_required -or $staged.backup.state -ne "restore_staged") {
        throw "Verified backup was not staged for restore"
    }
    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 5
    if (-not $status.restore_pending) {
        throw "HomeServer status did not report the staged restore"
    }

    Stop-HomeServerProcess
    Start-HomeServerProcess
    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 5
    if ($status.restore_pending -or $status.database -ne "ready") {
        throw "Staged restore did not apply cleanly after restart"
    }
    $catalog = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/backups" -TimeoutSec 5
    $restored = $catalog.backups | Where-Object { $_.backup_id -eq $manual.backup.backup_id } | Select-Object -First 1
    if (-not $restored -or $restored.state -ne "restored") {
        throw "Applied restore was not recorded in the restored database"
    }

    Stop-HomeServerProcess
    $env:MG_HOMESERVER_DATA_DIR = $freshDataDirectory
    $env:MG_HOMESERVER_NAME = "CI Recovery HomeServer"
    Start-HomeServerProcess

    $freshStatus = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 5
    if ($freshStatus.server_name -ne "CI Recovery HomeServer" -or $freshStatus.database -ne "ready") {
        throw "Fresh HomeServer installation did not initialize correctly"
    }
    $freshCatalog = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/backups" -TimeoutSec 5
    if (@($freshCatalog.backups).Count -ne 0) {
        throw "Fresh HomeServer catalog was not empty before recovery import"
    }

    $wrongImportHeaders = @{
        "X-MG-Local-Client" = "microgifter-control-center-v1"
        "x-mg-recovery-passphrase" = ConvertTo-Base64Url "wrong recovery passphrase value"
    }
    $wrongImport = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Uri "$apiBase/v1/backups/import" -Headers $wrongImportHeaders -ContentType "application/vnd.microgifter.homeserver-backup" -InFile $exportedPackage -TimeoutSec 90
    if ($wrongImport.StatusCode -ne 422) {
        throw "Expected wrong import passphrase rejection, received HTTP $($wrongImport.StatusCode)"
    }
    $freshCatalog = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/backups" -TimeoutSec 5
    if (@($freshCatalog.backups).Count -ne 0) {
        throw "Failed recovery import left a catalog record"
    }
    $freshRecoveryDirectory = Join-Path $freshDataDirectory "recovery-packages"
    if (@(Get-ChildItem $freshRecoveryDirectory -Filter "*.mghbackup" -ErrorAction SilentlyContinue).Count -ne 0) {
        throw "Failed recovery import left a managed package"
    }

    $importHeaders = @{
        "X-MG-Local-Client" = "microgifter-control-center-v1"
        "x-mg-recovery-passphrase" = ConvertTo-Base64Url $recoveryPassphrase
    }
    $imported = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/backups/import" -Headers $importHeaders -ContentType "application/vnd.microgifter.homeserver-backup" -InFile $exportedPackage -TimeoutSec 90
    if ($imported.backup.kind -ne "recovery" -or $imported.backup.state -ne "verified") {
        throw "Portable recovery package was not imported and verified"
    }
    if ($imported.backup.backup_id -ne $recovery.backup.backup_id) {
        throw "Imported recovery package identity changed"
    }

    $freshRestoreBody = @{
        backup_id = $imported.backup.backup_id
        passphrase = $recoveryPassphrase
        confirmation = "RESTORE"
    } | ConvertTo-Json -Compress
    $freshStaged = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/stage-restore" -ContentType "application/json" -Body $freshRestoreBody -TimeoutSec 90
    if (-not $freshStaged.restart_required -or $freshStaged.backup.state -ne "restore_staged") {
        throw "Imported recovery package could not be staged on a fresh installation"
    }

    Stop-HomeServerProcess
    Start-HomeServerProcess
    $freshStatus = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 5
    if ($freshStatus.restore_pending -or $freshStatus.database -ne "ready") {
        throw "Fresh-install recovery did not apply cleanly"
    }
    $freshCatalog = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/backups" -TimeoutSec 5
    $freshRestored = $freshCatalog.backups | Where-Object { $_.backup_id -eq $recovery.backup.backup_id } | Select-Object -First 1
    if (-not $freshRestored -or $freshRestored.state -ne "restored") {
        throw "Fresh-install recovery was not recorded in the restored database"
    }
    $rollbackDatabase = Get-ChildItem (Join-Path $freshDataDirectory "restore") -Filter "rollback-*.sqlite3" -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $rollbackDatabase) {
        throw "Fresh-install recovery did not preserve its pre-restore database for rollback"
    }

    $databasePath = Join-Path $freshDataDirectory "homeserver.sqlite3"
    if (-not (Test-Path $databasePath)) {
        throw "Recovered HomeServer SQLite database was not created"
    }

    Write-Host "HomeServer operational evidence, supervised agents, MCP, encrypted backup, exported recovery, fresh-install import, verification, staged restore, and rollback-ready smoke test passed."
}
finally {
    Stop-HomeServerProcess
    Remove-Item Env:MG_HOMESERVER_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:MG_HOMESERVER_NAME -ErrorAction SilentlyContinue
    foreach ($path in @($primaryDataDirectory, $freshDataDirectory)) {
        if (Test-Path $path) {
            Remove-Item $path -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
    if (Test-Path $exportedPackage) {
        Remove-Item $exportedPackage -Force -ErrorAction SilentlyContinue
    }
}
