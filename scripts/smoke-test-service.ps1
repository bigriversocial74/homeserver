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
    if ($status.backup -ne "ready") {
        throw "Expected ready backup service, received '$($status.backup)'"
    }

    $manualBody = @{
        kind = "manual"
        passphrase = $null
        note = "CI manual backup"
    } | ConvertTo-Json -Compress
    $manual = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/backups/create" -ContentType "application/json" -Body $manualBody -TimeoutSec 90
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
    $verifiedManual = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/backups/verify" -ContentType "application/json" -Body $verifyManualBody -TimeoutSec 90
    if ($verifiedManual.backup.state -ne "verified") {
        throw "Manual backup verification did not persist"
    }

    $recoveryPassphrase = "correct horse battery staple 2026"
    $recoveryBody = @{
        kind = "recovery"
        passphrase = $recoveryPassphrase
        note = "CI portable recovery package"
    } | ConvertTo-Json -Compress
    $recovery = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/backups/create" -ContentType "application/json" -Body $recoveryBody -TimeoutSec 90
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
    $wrongPassphrase = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Uri "$apiBase/v1/backups/verify" -ContentType "application/json" -Body $wrongPassphraseBody -TimeoutSec 90
    if ($wrongPassphrase.StatusCode -ne 422) {
        throw "Expected wrong recovery passphrase rejection, received HTTP $($wrongPassphrase.StatusCode)"
    }

    $verifyRecoveryBody = @{
        backup_id = $recovery.backup.backup_id
        passphrase = $recoveryPassphrase
        confirmation = $null
    } | ConvertTo-Json -Compress
    $verifiedRecovery = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/backups/verify" -ContentType "application/json" -Body $verifyRecoveryBody -TimeoutSec 90
    if ($verifiedRecovery.backup.state -ne "verified") {
        throw "Recovery package verification did not persist"
    }

    $catalog = Invoke-RestMethod -Uri "$apiBase/v1/backups" -TimeoutSec 5
    if ($catalog.backups.Count -lt 2 -or [int]$catalog.retention_count -ne 14 -or [int]$catalog.interval_hours -ne 24) {
        throw "Backup catalog or policy is incomplete"
    }

    $restoreBody = @{
        backup_id = $manual.backup.backup_id
        passphrase = $null
        confirmation = "RESTORE"
    } | ConvertTo-Json -Compress
    $staged = Invoke-RestMethod -Method Post -Uri "$apiBase/v1/backups/stage-restore" -ContentType "application/json" -Body $restoreBody -TimeoutSec 90
    if (-not $staged.restart_required -or $staged.backup.state -ne "restore_staged") {
        throw "Verified backup was not staged for restore"
    }
    $status = Invoke-RestMethod -Uri "$apiBase/v1/status" -TimeoutSec 5
    if (-not $status.restore_pending) {
        throw "HomeServer status did not report the staged restore"
    }

    Stop-HomeServerProcess
    Start-HomeServerProcess
    $status = Invoke-RestMethod -Uri "$apiBase/v1/status" -TimeoutSec 5
    if ($status.restore_pending -or $status.database -ne "ready") {
        throw "Staged restore did not apply cleanly after restart"
    }
    $catalog = Invoke-RestMethod -Uri "$apiBase/v1/backups" -TimeoutSec 5
    $restored = $catalog.backups | Where-Object { $_.backup_id -eq $manual.backup.backup_id } | Select-Object -First 1
    if (-not $restored -or $restored.state -ne "restored") {
        throw "Applied restore was not recorded in the restored database"
    }

    $databasePath = Join-Path $dataDirectory "homeserver.sqlite3"
    if (-not (Test-Path $databasePath)) {
        throw "HomeServer SQLite database was not created"
    }

    Write-Host "HomeServer encrypted backup, recovery, verification, staged restore, and rollback-ready smoke test passed."
}
finally {
    Stop-HomeServerProcess
    Remove-Item Env:MG_HOMESERVER_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:MG_HOMESERVER_NAME -ErrorAction SilentlyContinue
    if (Test-Path $dataDirectory) {
        Remove-Item $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
