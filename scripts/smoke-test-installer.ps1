param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath
)

$ErrorActionPreference = "Stop"
$serviceName = "MicrogifterHomeServer"
$installer = (Resolve-Path $InstallerPath).Path
$dataDirectory = Join-Path $env:ProgramData "Microgifter\HomeServer"
$markerPath = Join-Path $dataDirectory "ci-preservation-marker.txt"
$uninstallerPath = $null

function Wait-ForHomeServerHealth {
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        try {
            $service = Get-Service -Name $serviceName -ErrorAction Stop
            if ($service.Status -eq "Running") {
                $health = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:47831/healthz" -TimeoutSec 2
                if ($health.StatusCode -eq 204) {
                    return
                }
            }
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }

    throw "Installed HomeServer service did not become healthy"
}

function Resolve-HomeServerUninstaller {
    $registryPaths = @(
        "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*",
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
    )

    $entry = Get-ItemProperty $registryPaths -ErrorAction SilentlyContinue |
        Where-Object { $_.DisplayName -eq "Microgifter HomeServer" } |
        Select-Object -First 1

    if ($entry) {
        $command = if ($entry.QuietUninstallString) { $entry.QuietUninstallString } else { $entry.UninstallString }
        if ($command -match '^"([^"]+)"') {
            return $matches[1]
        }
        if ($command -and (Test-Path $command)) {
            return $command
        }
    }

    $expected = Join-Path $env:ProgramFiles "Microgifter HomeServer\uninstall.exe"
    if (Test-Path $expected) {
        return $expected
    }

    throw "Unable to locate the Microgifter HomeServer uninstaller"
}

try {
    $install = Start-Process -FilePath $installer -ArgumentList "/S" -PassThru -Wait
    if ($install.ExitCode -ne 0) {
        throw "HomeServer installer failed with exit code $($install.ExitCode)"
    }

    Wait-ForHomeServerHealth
    $status = Invoke-RestMethod -Uri "http://127.0.0.1:47831/v1/status" -TimeoutSec 3
    if ($status.state -ne "running" -or $status.database -ne "ready") {
        throw "Installed HomeServer reported state '$($status.state)' and database '$($status.database)'"
    }

    if (-not (Test-Path (Join-Path $dataDirectory "homeserver.sqlite3"))) {
        throw "Installed HomeServer did not create its SQLite database"
    }

    $logFiles = Get-ChildItem (Join-Path $dataDirectory "logs") -Filter "microgifter-homeserver-service.log*" -ErrorAction SilentlyContinue
    if (-not $logFiles) {
        throw "Installed HomeServer did not create a persistent service log"
    }

    Set-Content -Path $markerPath -Value "preserve" -Encoding UTF8
    $uninstallerPath = Resolve-HomeServerUninstaller
    $uninstall = Start-Process -FilePath $uninstallerPath -ArgumentList "/S" -PassThru -Wait
    if ($uninstall.ExitCode -ne 0) {
        throw "HomeServer uninstaller failed with exit code $($uninstall.ExitCode)"
    }

    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        if (-not (Get-Service -Name $serviceName -ErrorAction SilentlyContinue)) {
            break
        }
        Start-Sleep -Milliseconds 500
    }

    if (Get-Service -Name $serviceName -ErrorAction SilentlyContinue) {
        throw "HomeServer Windows service remained after uninstall"
    }
    if (-not (Test-Path $markerPath)) {
        throw "HomeServer data was removed during default uninstall"
    }

    Write-Host "HomeServer installer, service, health, logging, data-preservation, and uninstall smoke tests passed."
}
finally {
    $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    if ($service) {
        Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
        & "$env:SystemRoot\System32\sc.exe" delete $serviceName | Out-Null
    }
    if ($uninstallerPath -and (Test-Path $uninstallerPath)) {
        Start-Process -FilePath $uninstallerPath -ArgumentList "/S" -Wait -ErrorAction SilentlyContinue
    }
    if (Test-Path $dataDirectory) {
        Remove-Item $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
