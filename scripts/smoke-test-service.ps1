param(
    [string]$ServiceBinary = "target/release/microgifter-homeserver-service.exe"
)

$ErrorActionPreference = "Stop"
$binaryPath = (Resolve-Path $ServiceBinary).Path
$dataDirectory = Join-Path $env:RUNNER_TEMP ("microgifter-homeserver-service-" + [guid]::NewGuid().ToString("N"))
$env:MG_HOMESERVER_DATA_DIR = $dataDirectory
$env:MG_HOMESERVER_NAME = "CI HomeServer"
$process = $null
$apiBase = "http://127.0.0.1:47831"

try {
    $process = Start-Process -FilePath $binaryPath -ArgumentList "console" -PassThru -WindowStyle Hidden
    $ready = $false

    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        if ($process.HasExited) {
            throw "HomeServer console process exited before becoming ready with code $($process.ExitCode)"
        }

        try {
            $health = Invoke-WebRequest -UseBasicParsing -Uri "$apiBase/healthz" -TimeoutSec 2
            if ($health.StatusCode -eq 204) {
                $ready = $true
                break
            }
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }

    if (-not $ready) {
        throw "HomeServer console service did not become healthy"
    }

    $status = Invoke-RestMethod -Uri "$apiBase/v1/status" -TimeoutSec 3
    if ($status.state -ne "running") {
        throw "Expected running state, received '$($status.state)'"
    }
    if ($status.database -ne "ready") {
        throw "Expected ready database, received '$($status.database)'"
    }
    if ($status.server_name -ne "CI HomeServer") {
        throw "Expected sanitized CI server name, received '$($status.server_name)'"
    }
    if ($status.cloud -ne "not_paired") {
        throw "Expected not_paired cloud state, received '$($status.cloud)'"
    }
    if ([int]$status.pending_sync -ne 0) {
        throw "Expected an empty synchronization queue"
    }

    $connection = Invoke-RestMethod -Uri "$apiBase/v1/connection" -TimeoutSec 3
    if ($connection.state -ne "not_paired") {
        throw "Expected an unpaired connection snapshot"
    }

    $vault = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/diagnostics/credential-vault" -ContentType "application/json" -Body "{}" -TimeoutSec 5
    if (-not $vault.ok -or $vault.credential_vault -ne "ready") {
        throw "HomeServer operating-system credential vault self-test failed"
    }

    $idempotencyKey = "ci.settings.snapshot.1"
    $enqueueBody = @{
        operation_type = "local.settings.snapshot"
        idempotency_key = $idempotencyKey
        payload = @{ source = "service-smoke"; enabled = $true }
    } | ConvertTo-Json -Depth 5 -Compress
    $enqueue = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/sync/enqueue" -ContentType "application/json" -Body $enqueueBody -TimeoutSec 3
    if (-not $enqueue.ok -or $enqueue.idempotency_key -ne $idempotencyKey) {
        throw "Approved local synchronization work was not queued correctly"
    }

    $duplicate = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/sync/enqueue" -ContentType "application/json" -Body $enqueueBody -TimeoutSec 3
    if ($duplicate.idempotency_key -ne $idempotencyKey) {
        throw "Idempotent synchronization replay did not return the existing key"
    }

    $status = Invoke-RestMethod -Uri "$apiBase/v1/status" -TimeoutSec 3
    if ([int]$status.pending_sync -ne 1) {
        throw "Expected one pending synchronization operation, received '$($status.pending_sync)'"
    }

    $conflictBody = @{
        operation_type = "local.settings.snapshot"
        idempotency_key = $idempotencyKey
        payload = @{ source = "different-work" }
    } | ConvertTo-Json -Depth 5 -Compress
    $conflict = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Uri "$apiBase/v1/sync/enqueue" -ContentType "application/json" -Body $conflictBody -TimeoutSec 3
    if ($conflict.StatusCode -ne 422) {
        throw "Expected idempotency conflict rejection, received HTTP $($conflict.StatusCode)"
    }

    $commerceBody = @{
        operation_type = "commerce.order.create"
        idempotency_key = "ci.commerce.rejected.1"
        payload = @{}
    } | ConvertTo-Json -Depth 5 -Compress
    $commerce = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Uri "$apiBase/v1/sync/enqueue" -ContentType "application/json" -Body $commerceBody -TimeoutSec 3
    if ($commerce.StatusCode -ne 422) {
        throw "Expected local commerce-authority rejection, received HTTP $($commerce.StatusCode)"
    }

    $sync = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/sync/run" -ContentType "application/json" -Body "{}" -TimeoutSec 3
    if ([int]$sync.processed -ne 0 -or [int]$sync.pending -ne 1) {
        throw "Unpaired synchronization must preserve pending work without claiming completion"
    }

    $databasePath = Join-Path $dataDirectory "homeserver.sqlite3"
    if (-not (Test-Path $databasePath)) {
        throw "HomeServer SQLite database was not created"
    }

    Write-Host "HomeServer Phase 2 console and credential-vault smoke test passed."
}
finally {
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
        $process.WaitForExit(5000) | Out-Null
    }
    Remove-Item Env:MG_HOMESERVER_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:MG_HOMESERVER_NAME -ErrorAction SilentlyContinue
    if (Test-Path $dataDirectory) {
        Remove-Item $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
