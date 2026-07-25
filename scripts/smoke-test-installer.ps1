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
$apiBase = "http://127.0.0.1:47831"
$controlHeaders = @{ "X-MG-Local-Client" = "microgifter-control-center-v1" }
$backupPath = $null

function Wait-ForHomeServerHealth {
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
    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 3
    if ($status.state -ne "running" -or $status.database -ne "ready" -or $status.backup -ne "ready") {
        throw "Installed HomeServer reported state '$($status.state)', database '$($status.database)', and backup '$($status.backup)'"
    }

    if (-not (Test-Path (Join-Path $dataDirectory "homeserver.sqlite3"))) {
        throw "Installed HomeServer did not create its SQLite database"
    }

    $dataAcl = Get-Acl -LiteralPath $dataDirectory
    if (-not $dataAcl.AreAccessRulesProtected) {
        throw "HomeServer data directory still inherits broad parent permissions"
    }
    $broadLocalSids = @("S-1-1-0", "S-1-5-11", "S-1-5-32-545")
    $requiredFullControlSids = @("S-1-5-18", "S-1-5-32-544")
    $fullControlFound = @{}
    foreach ($sid in $requiredFullControlSids) { $fullControlFound[$sid] = $false }
    $writeRights = [int][System.Security.AccessControl.FileSystemRights]::WriteData -bor
        [int][System.Security.AccessControl.FileSystemRights]::CreateFiles -bor
        [int][System.Security.AccessControl.FileSystemRights]::CreateDirectories -bor
        [int][System.Security.AccessControl.FileSystemRights]::Modify -bor
        [int][System.Security.AccessControl.FileSystemRights]::FullControl
    foreach ($rule in $dataAcl.Access) {
        $sid = $rule.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value
        if ($rule.AccessControlType -eq [System.Security.AccessControl.AccessControlType]::Allow) {
            if ($broadLocalSids -contains $sid -and (([int]$rule.FileSystemRights -band $writeRights) -ne 0)) {
                throw "HomeServer data directory grants broad write access to $sid"
            }
            if ($requiredFullControlSids -contains $sid -and $rule.FileSystemRights.HasFlag([System.Security.AccessControl.FileSystemRights]::FullControl)) {
                $fullControlFound[$sid] = $true
            }
        }
    }
    foreach ($sid in $requiredFullControlSids) {
        if (-not $fullControlFound[$sid]) {
            throw "HomeServer data directory is missing required full control for $sid"
        }
    }

    $vault = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/vault-self-test" -ContentType "application/json" -Body "{}" -TimeoutSec 30
    if (-not $vault.ok) {
        throw "Installed LocalSystem machine-scoped credential vault self-test failed"
    }

    $logFiles = Get-ChildItem (Join-Path $dataDirectory "logs") -Filter "microgifter-homeserver-service.log*" -ErrorAction SilentlyContinue
    if (-not $logFiles) {
        throw "Installed HomeServer did not create a persistent service log"
    }

    $backupBody = @{
        kind = "manual"
        passphrase = $null
        note = "Installed LocalSystem backup validation"
    } | ConvertTo-Json -Compress
    $backup = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/create" -ContentType "application/json" -Body $backupBody -TimeoutSec 90
    if ($backup.backup.state -ne "ready" -or $backup.backup.encryption -ne "device_key_aes256gcm") {
        throw "Installed LocalSystem service did not create a device-key encrypted backup"
    }
    $backupPath = $backup.backup.storage_path
    if (-not (Test-Path $backupPath)) {
        throw "Installed HomeServer backup package was not written"
    }

    $verifyBody = @{
        backup_id = $backup.backup.backup_id
        passphrase = $null
        confirmation = $null
    } | ConvertTo-Json -Compress
    $verified = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/verify" -ContentType "application/json" -Body $verifyBody -TimeoutSec 90
    if ($verified.backup.state -ne "verified") {
        throw "Installed LocalSystem service could not decrypt and verify its backup"
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
    if (-not $backupPath -or -not (Test-Path $backupPath)) {
        throw "Encrypted HomeServer backups were removed during default uninstall"
    }

    Write-Host "HomeServer installer, LocalSystem backup encryption, verification, logging, data preservation, and uninstall smoke tests passed."
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
