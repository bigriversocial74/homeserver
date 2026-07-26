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
$diagnosticPath = Join-Path (Get-Location) "installer-verification.log"

Remove-Item -LiteralPath $diagnosticPath -Force -ErrorAction SilentlyContinue

function Write-Diagnostic {
    param(
        [Parameter(Mandatory = $true)]
        [AllowEmptyString()]
        [string]$Message
    )

    $line = "[{0}] {1}" -f (Get-Date).ToUniversalTime().ToString("o"), $Message
    Add-Content -LiteralPath $diagnosticPath -Value $line -Encoding UTF8
    Write-Host $line
}

function Write-DiagnosticBlock {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Label,
        [AllowNull()]
        [object]$Value
    )

    Write-Diagnostic "$Label BEGIN"
    $rendered = if ($null -eq $Value) { "<null>" } else { $Value | Out-String }
    Add-Content -LiteralPath $diagnosticPath -Value $rendered.TrimEnd() -Encoding UTF8
    Write-Diagnostic "$Label END"
}

function Invoke-BoundedProcess {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [Parameter(Mandatory = $true)]
        [string[]]$ArgumentList,
        [Parameter(Mandatory = $true)]
        [ValidateRange(1, 600)]
        [int]$TimeoutSeconds,
        [Parameter(Mandatory = $true)]
        [string]$Label
    )

    $process = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -PassThru
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        Write-Diagnostic "$Label exceeded its $TimeoutSeconds-second process deadline"
        try {
            Stop-Process -Id $process.Id -Force -ErrorAction Stop
        }
        catch {
            Write-Diagnostic "Unable to terminate timed-out $Label process $($process.Id): $($_.Exception.Message)"
        }
        throw "$Label timed out after $TimeoutSeconds seconds"
    }

    $process.Refresh()
    Write-Diagnostic "$Label exited with code $($process.ExitCode)"
    return $process.ExitCode
}

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
    Write-Diagnostic "Starting installed LocalSystem smoke test with installer '$installer'"
    $installExitCode = Invoke-BoundedProcess -FilePath $installer -ArgumentList @("/S") -TimeoutSeconds 180 -Label "HomeServer installer"
    if ($installExitCode -ne 0) {
        throw "HomeServer installer failed with exit code $installExitCode"
    }

    Wait-ForHomeServerHealth
    Write-Diagnostic "Installed service became healthy"

    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 3
    Write-DiagnosticBlock "STATUS" $status
    if ($status.state -ne "running" -or $status.database -ne "ready" -or $status.backup -ne "ready") {
        throw "Installed HomeServer reported state '$($status.state)', database '$($status.database)', and backup '$($status.backup)'"
    }

    $databasePath = Join-Path $dataDirectory "homeserver.sqlite3"
    if (-not (Test-Path $databasePath)) {
        throw "Installed HomeServer did not create its SQLite database"
    }
    Write-Diagnostic "SQLite database exists at '$databasePath'"

    $dataAcl = Get-Acl -LiteralPath $dataDirectory
    Write-DiagnosticBlock "DATA DIRECTORY ACL" ($dataAcl | Format-List Path, Owner, AreAccessRulesProtected, Access)
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
        [int][System.Security.AccessControl.FileSystemRights]::WriteAttributes -bor
        [int][System.Security.AccessControl.FileSystemRights]::WriteExtendedAttributes -bor
        [int][System.Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles -bor
        [int][System.Security.AccessControl.FileSystemRights]::Delete -bor
        [int][System.Security.AccessControl.FileSystemRights]::ChangePermissions -bor
        [int][System.Security.AccessControl.FileSystemRights]::TakeOwnership
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
    Write-Diagnostic "Data-directory ACL boundary passed"

    $vault = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/cloud/vault-self-test" -ContentType "application/json" -Body "{}" -TimeoutSec 30
    Write-DiagnosticBlock "VAULT SELF TEST" $vault
    if (-not $vault.ok) {
        throw "Installed LocalSystem machine-scoped credential vault self-test failed"
    }

    $logDirectory = Join-Path $dataDirectory "logs"
    $logFiles = Get-ChildItem $logDirectory -Filter "microgifter-homeserver-service.log*" -ErrorAction SilentlyContinue
    if (-not $logFiles) {
        throw "Installed HomeServer did not create a persistent service log"
    }
    Write-Diagnostic "Persistent service logging exists"

    $backupBody = @{
        kind = "manual"
        passphrase = $null
        note = "Installed LocalSystem backup validation"
    } | ConvertTo-Json -Compress
    $backup = Invoke-RestMethod -Method Post -Headers $controlHeaders -Uri "$apiBase/v1/backups/create" -ContentType "application/json" -Body $backupBody -TimeoutSec 90
    Write-DiagnosticBlock "BACKUP CREATE" $backup
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
    Write-DiagnosticBlock "BACKUP VERIFY" $verified
    if ($verified.backup.state -ne "verified") {
        throw "Installed LocalSystem service could not decrypt and verify its backup"
    }

    Set-Content -Path $markerPath -Value "preserve" -Encoding UTF8
    Write-Diagnostic "Preservation marker written"
    $uninstallerPath = Resolve-HomeServerUninstaller
    Write-Diagnostic "Resolved uninstaller '$uninstallerPath'"
    $uninstallExitCode = Invoke-BoundedProcess -FilePath $uninstallerPath -ArgumentList @("/S") -TimeoutSeconds 180 -Label "HomeServer uninstaller"
    if ($uninstallExitCode -ne 0) {
        throw "HomeServer uninstaller failed with exit code $uninstallExitCode"
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

    Write-Diagnostic "HomeServer installer, LocalSystem backup encryption, verification, logging, data preservation, and uninstall smoke tests passed"
}
catch {
    Write-Diagnostic "FAILURE: $($_.Exception.Message)"
    if ($_.ScriptStackTrace) {
        Write-DiagnosticBlock "SCRIPT STACK" $_.ScriptStackTrace
    }

    try {
        Write-DiagnosticBlock "SERVICE SNAPSHOT" (Get-CimInstance Win32_Service -Filter "Name='$serviceName'" | Select-Object Name, State, StartMode, StartName, PathName, ExitCode)
    }
    catch {
        Write-Diagnostic "Unable to collect service snapshot: $($_.Exception.Message)"
    }

    try {
        if (Test-Path $dataDirectory) {
            Write-DiagnosticBlock "FAILURE ACL SNAPSHOT" (Get-Acl -LiteralPath $dataDirectory | Format-List Path, Owner, AreAccessRulesProtected, Access)
        }
    }
    catch {
        Write-Diagnostic "Unable to collect ACL snapshot: $($_.Exception.Message)"
    }

    try {
        $serviceLogs = Get-ChildItem (Join-Path $dataDirectory "logs") -Filter "microgifter-homeserver-service.log*" -ErrorAction SilentlyContinue |
            Sort-Object LastWriteTimeUtc -Descending
        foreach ($serviceLog in $serviceLogs) {
            Write-DiagnosticBlock "SERVICE LOG $($serviceLog.FullName)" (Get-Content -LiteralPath $serviceLog.FullName -Tail 160 -ErrorAction SilentlyContinue)
        }
    }
    catch {
        Write-Diagnostic "Unable to collect service logs: $($_.Exception.Message)"
    }

    throw
}
finally {
    $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    if ($service) {
        Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
        & "$env:SystemRoot\System32\sc.exe" delete $serviceName | Out-Null
    }
    if ($uninstallerPath -and (Test-Path $uninstallerPath)) {
        try {
            [void](Invoke-BoundedProcess -FilePath $uninstallerPath -ArgumentList @("/S") -TimeoutSeconds 45 -Label "HomeServer cleanup uninstaller")
        }
        catch {
            Write-Diagnostic "Cleanup uninstaller did not complete: $($_.Exception.Message)"
        }
    }
    if (Test-Path $dataDirectory) {
        Remove-Item $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}
