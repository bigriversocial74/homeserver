from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "scripts" / "smoke-test-updater.ps1"


def replace_once(content: str, old: str, new: str) -> str:
    if content.count(old) != 1:
        raise RuntimeError(f"expected exactly one updater smoke match: {old[:100]!r}")
    return content.replace(old, new, 1)


content = TARGET.read_text(encoding="utf-8")

content = replace_once(
    content,
    '$currentUserTrustStores = @("Root", "TrustedPublisher")\n',
    '''$currentUserTrustStores = @("Root", "TrustedPublisher")
$registryPaths = @(
    "HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*",
    "HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*"
)
''',
)

old_wait = '''function Wait-ForHomeServerHealth {
    param([Parameter(Mandatory = $true)][string]$ExpectedVersion)

    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        try {
            $service = Get-Service -Name $serviceName -ErrorAction Stop
            if ($service.Status -eq "Running") {
                $health = Invoke-WebRequest -UseBasicParsing -Uri "$apiBase/healthz" -TimeoutSec 2
                if ($health.StatusCode -eq 204) {
                    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 3
                    if ($status.version -eq $ExpectedVersion -and $status.state -eq "running") {
                        return $status
                    }
                }
            }
        }
        catch {
            Start-Sleep -Milliseconds 500
        }
    }

    throw "HomeServer did not become healthy at version $ExpectedVersion"
}
'''

new_wait = '''function Wait-ForHomeServerHealth {
    param([Parameter(Mandatory = $true)][string]$ExpectedVersion)

    $lastServiceStatus = "missing"
    $lastHealthStatus = $null
    $lastStatus = $null
    $lastError = $null

    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        try {
            $service = Get-Service -Name $serviceName -ErrorAction Stop
            $lastServiceStatus = [string]$service.Status
            if ($service.Status -eq "Running") {
                $health = Invoke-WebRequest -UseBasicParsing -Uri "$apiBase/healthz" -TimeoutSec 2
                $lastHealthStatus = [int]$health.StatusCode
                if ($health.StatusCode -eq 204) {
                    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 3
                    $lastStatus = $status
                    if ($status.version -eq $ExpectedVersion -and $status.state -eq "running") {
                        return $status
                    }
                }
            }
        }
        catch {
            $lastError = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 500
    }

    Write-Host "HomeServer health wait diagnostics: service=$lastServiceStatus health=$lastHealthStatus error=$lastError"
    if ($lastStatus) {
        Write-Host "HomeServer status snapshot: $($lastStatus | ConvertTo-Json -Depth 8 -Compress)"
    }
    $serviceLogs = Get-ChildItem (Join-Path $dataDirectory "logs") -Filter "microgifter-homeserver-service.log*" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTimeUtc -Descending
    foreach ($serviceLog in $serviceLogs) {
        Write-Host "HomeServer service log $($serviceLog.FullName):"
        Get-Content -LiteralPath $serviceLog.FullName -Tail 200 -ErrorAction SilentlyContinue
    }
    & "$env:SystemRoot\\System32\\sc.exe" queryex $serviceName 2>$null
    throw "HomeServer did not become healthy at version $ExpectedVersion"
}

function Get-HomeServerRegistryEntries {
    @(
        Get-ItemProperty $registryPaths -ErrorAction SilentlyContinue |
            Where-Object { $_.DisplayName -eq "Microgifter HomeServer" }
    )
}

function Reset-HomeServerInstallationBoundary {
    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
        & "$env:SystemRoot\\System32\\sc.exe" delete $serviceName 2>$null | Out-Null
        Get-Process -Name "microgifter-homeserver*" -ErrorAction SilentlyContinue |
            Stop-Process -Force -ErrorAction SilentlyContinue

        if (Test-Path $installDirectory) {
            Remove-Item $installDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path $dataDirectory) {
            Remove-Item $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
        foreach ($entry in Get-HomeServerRegistryEntries) {
            Remove-Item -LiteralPath $entry.PSPath -Recurse -Force -ErrorAction SilentlyContinue
        }

        $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
        $processes = @(Get-Process -Name "microgifter-homeserver*" -ErrorAction SilentlyContinue)
        $registryEntries = @(Get-HomeServerRegistryEntries)
        if (-not $service -and $processes.Count -eq 0 -and $registryEntries.Count -eq 0 -and -not (Test-Path $installDirectory) -and -not (Test-Path $dataDirectory)) {
            Start-Sleep -Milliseconds 1500
            if (-not (Get-Service -Name $serviceName -ErrorAction SilentlyContinue) -and
                @(Get-Process -Name "microgifter-homeserver*" -ErrorAction SilentlyContinue).Count -eq 0) {
                return
            }
        }
        Start-Sleep -Milliseconds 500
    }

    throw "Previous HomeServer installation did not fully release before updater validation"
}
'''
content = replace_once(content, old_wait, new_wait)

content = replace_once(
    content,
    '''    $install = Start-Process -FilePath $installer -ArgumentList "/S" -PassThru -Wait
''',
    '''    Reset-HomeServerInstallationBoundary
    $install = Start-Process -FilePath $installer -ArgumentList "/S" -PassThru -Wait
''',
)

TARGET.write_text(content, encoding="utf-8")
print("Updater smoke test boundary and diagnostics hardened.")
