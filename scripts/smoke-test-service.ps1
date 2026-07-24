param(
    [string]$ServiceBinary = "target/release/microgifter-homeserver-service.exe"
)

$ErrorActionPreference = "Stop"
$binaryPath = (Resolve-Path $ServiceBinary).Path
$dataDirectory = Join-Path $env:RUNNER_TEMP ("microgifter-homeserver-service-" + [guid]::NewGuid().ToString("N"))
$env:MG_HOMESERVER_DATA_DIR = $dataDirectory
$env:MG_HOMESERVER_NAME = "CI HomeServer"
$process = $null

try {
    $process = Start-Process -FilePath $binaryPath -ArgumentList "console" -PassThru -WindowStyle Hidden
    $ready = $false

    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        if ($process.HasExited) {
            throw "HomeServer console process exited before becoming ready with code $($process.ExitCode)"
        }

        try {
            $health = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:47831/healthz" -TimeoutSec 2
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

    $status = Invoke-RestMethod -Uri "http://127.0.0.1:47831/v1/status" -TimeoutSec 3
    if ($status.state -ne "running") {
        throw "Expected running state, received '$($status.state)'"
    }
    if ($status.database -ne "ready") {
        throw "Expected ready database, received '$($status.database)'"
    }
    if ($status.server_name -ne "CI HomeServer") {
        throw "Expected sanitized CI server name, received '$($status.server_name)'"
    }

    $databasePath = Join-Path $dataDirectory "homeserver.sqlite3"
    if (-not (Test-Path $databasePath)) {
        throw "HomeServer SQLite database was not created"
    }

    Write-Host "HomeServer console smoke test passed."
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
