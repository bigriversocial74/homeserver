param(
    [string]$ServiceBinary = "target/release/microgifter-homeserver-service.exe"
)

$ErrorActionPreference = "Stop"
$binaryPath = (Resolve-Path $ServiceBinary).Path
$dataDirectory = Join-Path $env:RUNNER_TEMP ("microgifter-homeserver-cloud-" + [guid]::NewGuid().ToString("N"))
$env:MG_HOMESERVER_DATA_DIR = $dataDirectory
$env:MG_HOMESERVER_NAME = "CI Connector HomeServer"
$apiBase = "http://127.0.0.1:47831"
$controlHeaders = @{ "X-MG-Local-Client" = "microgifter-control-center-v1" }
$process = $null

function Start-ConnectorHomeServer {
    $script:process = Start-Process -FilePath $binaryPath -ArgumentList "console" -PassThru -WindowStyle Hidden
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if ($script:process.HasExited) {
            throw "HomeServer connector process exited before becoming ready with code $($script:process.ExitCode)"
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
    throw "HomeServer connector service did not become healthy"
}

function Stop-ConnectorHomeServer {
    if ($script:process -and -not $script:process.HasExited) {
        Stop-Process -Id $script:process.Id -Force -ErrorAction SilentlyContinue
        $script:process.WaitForExit(5000) | Out-Null
    }
    $script:process = $null
}

try {
    Start-ConnectorHomeServer

    $cloud = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/cloud" -TimeoutSec 5
    if ($cloud.state -ne "not_paired") {
        throw "Expected an unpaired cloud state, received '$($cloud.state)'"
    }
    if ([int]$cloud.pending_sync -ne 0) {
        throw "Fresh connector queue was not empty"
    }

    $unmarked = Invoke-WebRequest -SkipHttpErrorCheck -Method Get -Uri "$apiBase/v1/cloud" -TimeoutSec 10
    if ($unmarked.StatusCode -ne 403) {
        throw "Unmarked local API request should return HTTP 403"
    }
    $browserHeaders = @{
        "X-MG-Local-Client" = "microgifter-control-center-v1"
        "Origin" = "https://example.com"
    }
    $browserRequest = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $browserHeaders -Uri "$apiBase/v1/cloud/vault-self-test" -ContentType "application/json" -Body "{}" -TimeoutSec 10
    if ($browserRequest.StatusCode -ne 403) {
        throw "Browser-originated local mutation should return HTTP 403"
    }

    $vault = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/vault-self-test" -ContentType "application/json" -Body "{}" -TimeoutSec 30
    if (-not $vault.ok) {
        throw "Operating-system credential vault self-test did not pass"
    }

    $commerceBody = @{
        operation_type = "commerce.order.create"
        payload = @{ source = "ci" }
        idempotency_key = "ci-commerce-rejected"
    } | ConvertTo-Json -Depth 5 -Compress
    $commerce = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/enqueue" -ContentType "application/json" -Body $commerceBody -TimeoutSec 10
    if ($commerce.StatusCode -ne 422) {
        throw "Expected local commerce rejection, received HTTP $($commerce.StatusCode)"
    }

    $safeBody = @{
        operation_type = "cache.refresh.request"
        payload = @{ source = "ci"; reason = "connector-boundary-smoke" }
        idempotency_key = "ci-cache-refresh-1"
    } | ConvertTo-Json -Depth 5 -Compress
    $queued = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/enqueue" -ContentType "application/json" -Body $safeBody -TimeoutSec 10
    if ($queued.idempotency_key -ne "ci-cache-refresh-1") {
        throw "Safe synchronization operation did not retain its idempotency key"
    }

    $duplicate = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/enqueue" -ContentType "application/json" -Body $safeBody -TimeoutSec 10
    if ($duplicate.idempotency_key -ne "ci-cache-refresh-1") {
        throw "Retry-safe synchronization enqueue changed its idempotency key"
    }

    $conflictBody = @{
        operation_type = "local.settings.snapshot"
        payload = @{ source = "ci" }
        idempotency_key = "ci-cache-refresh-1"
    } | ConvertTo-Json -Depth 5 -Compress
    $conflict = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/enqueue" -ContentType "application/json" -Body $conflictBody -TimeoutSec 10
    if ($conflict.StatusCode -ne 422) {
        throw "Expected idempotency conflict rejection, received HTTP $($conflict.StatusCode)"
    }

    $cloud = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/cloud" -TimeoutSec 5
    if ([int]$cloud.pending_sync -ne 1) {
        throw "Expected one pending safe synchronization operation, received '$($cloud.pending_sync)'"
    }

    $sync = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/sync" -ContentType "application/json" -Body "{}" -TimeoutSec 10
    if ([int]$sync.processed -ne 0 -or [int]$sync.pending -ne 1) {
        throw "Unpaired synchronization did not remain safely queued"
    }

    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 5
    if ($status.state -ne "running" -or [int]$status.pending_sync -ne 1) {
        throw "HomeServer health snapshot did not expose the connector queue"
    }

    Write-Host "HomeServer credential vault, local authority boundary, idempotent queue, and unpaired synchronization smoke test passed."
}
finally {
    Stop-ConnectorHomeServer
    Remove-Item Env:MG_HOMESERVER_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:MG_HOMESERVER_NAME -ErrorAction SilentlyContinue
    if (Test-Path $dataDirectory) {
        Remove-Item $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
