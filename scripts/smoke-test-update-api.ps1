param(
    [string]$ServiceBinary = "target/release/microgifter-homeserver-service.exe"
)

$ErrorActionPreference = "Stop"
$binaryPath = (Resolve-Path $ServiceBinary).Path
$dataDirectory = Join-Path $env:RUNNER_TEMP ("microgifter-homeserver-update-api-" + [guid]::NewGuid().ToString("N"))
$env:MG_HOMESERVER_DATA_DIR = $dataDirectory
$env:MG_HOMESERVER_NAME = "Update API CI HomeServer"
$process = $null
$apiBase = "http://127.0.0.1:47831"
$controlHeaders = @{ "X-MG-Local-Client" = "microgifter-control-center-v1" }

try {
    $process = Start-Process -FilePath $binaryPath -ArgumentList "console" -PassThru -WindowStyle Hidden
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if ($process.HasExited) {
            throw "HomeServer console process exited before update API validation"
        }
        try {
            $health = Invoke-WebRequest -UseBasicParsing -Uri "$apiBase/healthz" -TimeoutSec 2
            if ($health.StatusCode -eq 204) { break }
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }

    $updates = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/updates" -TimeoutSec 3
    if ($updates.state -ne "idle" -or $updates.channel -ne "stable") {
        throw "Fresh HomeServer update state was not idle on the stable channel"
    }
    if ($updates.manifest_url -notlike "https://*") {
        throw "HomeServer update manifest URL was not HTTPS"
    }

    $download = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/updates/download" -ContentType "application/json" -Body "{}" -TimeoutSec 5
    if ($download.StatusCode -ne 422) {
        throw "Update download without a verified available release should return HTTP 422"
    }

    $applyBody = @{ confirmation = "not-update" } | ConvertTo-Json -Compress
    $apply = Invoke-WebRequest -SkipHttpErrorCheck -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/updates/apply" -ContentType "application/json" -Body $applyBody -TimeoutSec 5
    if ($apply.StatusCode -ne 422) {
        throw "Update apply without exact confirmation should return HTTP 422"
    }

    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 3
    if ($status.update -ne "idle" -or $status.update_version) {
        throw "Health snapshot did not preserve the idle signed-update state"
    }

    if (-not (Test-Path (Join-Path $dataDirectory "homeserver.sqlite3"))) {
        throw "Signed update API did not initialize the HomeServer database"
    }

    Write-Host "HomeServer signed update loopback API smoke test passed."
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
