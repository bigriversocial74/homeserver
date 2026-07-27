from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET = ROOT / "scripts" / "verify-installer-release.ps1"


def replace_once(content: str, old: str, new: str) -> str:
    if content.count(old) != 1:
        raise RuntimeError(f"expected exactly one installed-release verifier match: {old[:100]!r}")
    return content.replace(old, new, 1)


content = TARGET.read_text(encoding="utf-8")
old = '''function Wait-ForHomeServer {
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        try {
            $service = Get-Service -Name $serviceName -ErrorAction Stop
            if ($service.Status -eq "Running") {
                $health = Invoke-WebRequest -UseBasicParsing -Uri "$apiBase/healthz" -TimeoutSec 2
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
'''
new = '''function Wait-ForHomeServer {
    $lastServiceStatus = "missing"
    $lastHealthStatus = $null
    $lastStatus = $null
    $lastError = $null

    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        try {
            $service = Get-Service -Name $serviceName -ErrorAction Stop
            $lastServiceStatus = [string]$service.Status
            if ($service.Status -eq "Running") {
                $health = Invoke-WebRequest -UseBasicParsing -SkipHttpErrorCheck -Uri "$apiBase/healthz" -TimeoutSec 2
                $lastHealthStatus = [int]$health.StatusCode
                try {
                    $lastStatus = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 3
                }
                catch {
                    $lastError = $_.Exception.Message
                }
                if ($health.StatusCode -eq 204) {
                    return
                }
            }
        }
        catch {
            $lastError = $_.Exception.Message
        }
        Start-Sleep -Milliseconds 500
    }

    Write-Host "Installed HomeServer health diagnostics: service=$lastServiceStatus health=$lastHealthStatus error=$lastError"
    if ($lastStatus) {
        Write-Host "Installed HomeServer status snapshot: $($lastStatus | ConvertTo-Json -Depth 8 -Compress)"
    }
    $serviceLogs = Get-ChildItem (Join-Path $dataDirectory "logs") -Filter "microgifter-homeserver-service.log*" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTimeUtc -Descending
    foreach ($serviceLog in $serviceLogs) {
        Write-Host "Installed HomeServer service log $($serviceLog.FullName):"
        Get-Content -LiteralPath $serviceLog.FullName -Tail 200 -ErrorAction SilentlyContinue
    }
    & "$env:SystemRoot\\System32\\sc.exe" queryex $serviceName 2>$null
    Get-NetTCPConnection -LocalPort 47831 -ErrorAction SilentlyContinue |
        Select-Object LocalAddress, LocalPort, State, OwningProcess |
        Format-Table -AutoSize
    throw "Installed HomeServer service did not become healthy"
}
'''
content = replace_once(content, old, new)
TARGET.write_text(content, encoding="utf-8")
print("Installed-release verifier startup diagnostics hardened.")
