param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,

    [Parameter(Mandatory = $true)]
    [string]$UpdateInstallerPath,

    [Parameter(Mandatory = $true)]
    [string]$SignerThumbprint,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$ExpectedVersion
)

$ErrorActionPreference = "Stop"
$serviceName = "MicrogifterHomeServer"
$apiBase = "http://127.0.0.1:47831"
$controlHeaders = @{ "X-MG-Local-Client" = "microgifter-control-center-v1" }
$installer = (Resolve-Path $InstallerPath).Path
$updateInstaller = (Resolve-Path $UpdateInstallerPath).Path
$dataDirectory = Join-Path $env:ProgramData "Microgifter\HomeServer"
$installDirectory = Join-Path $env:ProgramFiles "Microgifter HomeServer"
$stagingDirectory = Join-Path $dataDirectory "updates\staging"
$rollbackRoot = Join-Path $dataDirectory "updates\rollback"
$installedArchive = Join-Path $dataDirectory "updates\installed"
$resultPath = Join-Path $dataDirectory "updates\last-update-result.json"
$diagnosticDirectory = Join-Path $env:SystemRoot "Temp\Microgifter-HomeServer-Updater-Smoke"
$uninstallerPath = $null
$scriptExitCode = 0
$currentUserTrustStores = @("Root", "TrustedPublisher")
$registryPaths = @(
    "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*",
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
)

function Read-TrimmedText {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [ValidateRange(1, 600)]
        [int]$TimeoutSeconds = 30
    )

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $lastError = $null
    do {
        try {
            if (Test-Path -LiteralPath $Path) {
                $content = Get-Content -LiteralPath $Path -Raw -ErrorAction Stop
                if ($null -eq $content) {
                    return ""
                }
                return $content.Trim()
            }
        }
        catch {
            $lastError = $_
        }
        Start-Sleep -Milliseconds 250
    } while ([DateTime]::UtcNow -lt $deadline)

    if ($lastError) {
        throw "Unable to read '$Path' after $TimeoutSeconds seconds: $($lastError.Exception.Message)"
    }
    return ""
}

function Set-CurrentUserPublisherTrust {
    param(
        [Parameter(Mandatory = $true)]
        [System.Security.Cryptography.X509Certificates.X509Certificate2]$Certificate
    )

    foreach ($storeName in $currentUserTrustStores) {
        $store = [System.Security.Cryptography.X509Certificates.X509Store]::new(
            $storeName,
            [System.Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
        )
        try {
            $store.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
            $store.Add($Certificate)
        }
        finally {
            $store.Close()
        }
    }
}

function Remove-CurrentUserPublisherTrust {
    param([Parameter(Mandatory = $true)][string]$Thumbprint)

    foreach ($storeName in $currentUserTrustStores) {
        $removed = $false
        $lastError = $null
        for ($attempt = 0; $attempt -lt 20 -and -not $removed; $attempt++) {
            $store = [System.Security.Cryptography.X509Certificates.X509Store]::new(
                $storeName,
                [System.Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser
            )
            try {
                $store.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
                $matches = $store.Certificates.Find(
                    [System.Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint,
                    $Thumbprint,
                    $false
                )
                foreach ($certificate in $matches) {
                    $store.Remove($certificate)
                }
                $removed = $true
            }
            catch {
                $lastError = $_
                Start-Sleep -Milliseconds 500
            }
            finally {
                $store.Close()
            }
        }
        if (-not $removed -and $lastError) {
            throw $lastError
        }
    }
}

function Get-WindowsPowerShellSignature {
    param([Parameter(Mandatory = $true)][string]$Path)

    $previous = $env:MG_UPDATE_FILE
    try {
        $env:MG_UPDATE_FILE = $Path
        $json = & powershell.exe -NoLogo -NoProfile -NonInteractive -Command @'
$signature = Get-AuthenticodeSignature -LiteralPath $env:MG_UPDATE_FILE
[pscustomobject]@{
    status = [string]$signature.Status
    status_message = [string]$signature.StatusMessage
    thumbprint = if ($signature.SignerCertificate) { [string]$signature.SignerCertificate.Thumbprint } else { "" }
} | ConvertTo-Json -Compress
'@
        if ($LASTEXITCODE -ne 0 -or -not $json) {
            throw "Windows PowerShell could not inspect the CI update signature"
        }
        $json | ConvertFrom-Json
    }
    finally {
        $env:MG_UPDATE_FILE = $previous
    }
}

function Wait-ForHomeServerHealth {
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
    & "$env:SystemRoot\System32\sc.exe" queryex $serviceName 2>$null
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
        & "$env:SystemRoot\System32\sc.exe" delete $serviceName 2>$null | Out-Null
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
        if ($command -match '^"([^"]+)"') { return $matches[1] }
        if ($command -and (Test-Path $command)) { return $command }
    }
    $expected = Join-Path $installDirectory "uninstall.exe"
    if (Test-Path $expected) { return $expected }
    throw "Unable to locate the Microgifter HomeServer uninstaller"
}

function New-UpdatePlan {
    param(
        [Parameter(Mandatory = $true)][string]$UpdateId,
        [Parameter(Mandatory = $true)][string]$CurrentVersion,
        [Parameter(Mandatory = $true)][string]$TargetVersion
    )

    New-Item -ItemType Directory -Force $stagingDirectory, $rollbackRoot, $installedArchive | Out-Null
    $safeId = $UpdateId -replace '[^A-Za-z0-9_.-]', '-'
    $stagedInstaller = Join-Path $stagingDirectory "$safeId-Microgifter-HomeServer-Setup.exe"
    Copy-Item $updateInstaller $stagedInstaller -Force
    $installerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $stagedInstaller).Hash.ToLowerInvariant()
    $installerSize = (Get-Item -LiteralPath $stagedInstaller).Length
    $rollbackDirectory = Join-Path $rollbackRoot $safeId
    $archivedInstaller = Join-Path $installedArchive "Microgifter-HomeServer-$TargetVersion.exe"
    $planPath = Join-Path $stagingDirectory "$safeId-plan.json"

    $plan = [ordered]@{
        schema_version = 1
        update_id = $UpdateId
        current_version = $CurrentVersion
        target_version = $TargetVersion
        installer_path = $stagedInstaller
        installer_size_bytes = $installerSize
        installer_sha256 = $installerHash
        authenticode_thumbprint = $SignerThumbprint.ToUpperInvariant()
        install_dir = $installDirectory
        data_dir = $dataDirectory
        rollback_dir = $rollbackDirectory
        archived_installer_path = $archivedInstaller
        result_path = $resultPath
        service_name = $serviceName
        health_url = $apiBase
    }
    $plan | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $planPath -Encoding UTF8

    [pscustomobject]@{
        PlanPath = $planPath
        StagedInstaller = $stagedInstaller
        ArchivedInstaller = $archivedInstaller
        RollbackDirectory = $rollbackDirectory
    }
}

function Invoke-UpdatePlan {
    param([Parameter(Mandatory = $true)]$Plan)

    $installedUpdater = Join-Path $installDirectory "resources\microgifter-homeserver-updater.exe"
    if (-not (Test-Path $installedUpdater)) {
        throw "Installed HomeServer updater helper is missing"
    }

    New-Item -ItemType Directory -Force $diagnosticDirectory | Out-Null
    $diagnosticId = [guid]::NewGuid().ToString("N")
    $taskName = "MicrogifterHomeServerUpdaterSmoke-$diagnosticId"
    $taskDirectory = Join-Path $diagnosticDirectory $diagnosticId
    $updaterCopy = Join-Path $stagingDirectory "updater-smoke-$diagnosticId.exe"
    $taskConfigPath = Join-Path $taskDirectory "task-config.json"
    $taskScriptPath = Join-Path $taskDirectory "run-updater.ps1"
    $completionPath = Join-Path $taskDirectory "completion.json"
    $stdoutPath = Join-Path $taskDirectory "updater.stdout.log"
    $stderrPath = Join-Path $taskDirectory "updater.stderr.log"

    New-Item -ItemType Directory -Force $taskDirectory | Out-Null
    Copy-Item $installedUpdater $updaterCopy -Force
    Remove-Item $resultPath -Force -ErrorAction SilentlyContinue

    [ordered]@{
        updater_path = $updaterCopy
        plan_path = $Plan.PlanPath
        result_path = $resultPath
        stdout_path = $stdoutPath
        stderr_path = $stderrPath
        completion_path = $completionPath
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $taskConfigPath -Encoding UTF8

    @'
$ErrorActionPreference = "Stop"
$config = Get-Content -LiteralPath (Join-Path $PSScriptRoot "task-config.json") -Raw | ConvertFrom-Json

function Read-TaskText {
    param([string]$Path)
    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        try {
            if (Test-Path -LiteralPath $Path) {
                $value = Get-Content -LiteralPath $Path -Raw -ErrorAction Stop
                if ($null -eq $value) { return "" }
                return $value.Trim()
            }
        }
        catch {
            Start-Sleep -Milliseconds 250
            continue
        }
        Start-Sleep -Milliseconds 250
    }
    return ""
}

$exitCode = 1
$taskError = $null
try {
    $process = Start-Process -FilePath $config.updater_path -ArgumentList @("apply", $config.plan_path) -RedirectStandardOutput $config.stdout_path -RedirectStandardError $config.stderr_path -PassThru -Wait
    $exitCode = $process.ExitCode
}
catch {
    $taskError = $_.Exception.ToString()
}

$stdout = Read-TaskText -Path $config.stdout_path
$stderr = Read-TaskText -Path $config.stderr_path
$resultJson = Read-TaskText -Path $config.result_path
[ordered]@{
    exit_code = $exitCode
    task_error = $taskError
    stdout = $stdout
    stderr = $stderr
    result_json = $resultJson
} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $config.completion_path -Encoding UTF8
'@ | Set-Content -LiteralPath $taskScriptPath -Encoding UTF8

    try {
        $taskTime = (Get-Date).AddMinutes(1).ToString("HH:mm")
        $windowsPowerShell = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
        $taskCommand = "`"$windowsPowerShell`" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$taskScriptPath`""
        $createOutput = & "$env:SystemRoot\System32\schtasks.exe" /Create /TN $taskName /TR $taskCommand /SC ONCE /ST $taskTime /RU SYSTEM /RL HIGHEST /F 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) {
            throw "Unable to create LocalSystem updater task: $($createOutput.Trim())"
        }
        $runOutput = & "$env:SystemRoot\System32\schtasks.exe" /Run /TN $taskName 2>&1 | Out-String
        if ($LASTEXITCODE -ne 0) {
            throw "Unable to start LocalSystem updater task: $($runOutput.Trim())"
        }

        $completionJson = Read-TrimmedText -Path $completionPath -TimeoutSeconds 420
        if (-not $completionJson) {
            throw "LocalSystem updater task did not produce a completion record"
        }
        $completion = $completionJson | ConvertFrom-Json

        if ($completion.stdout) {
            Write-Host "HomeServer updater stdout:`n$($completion.stdout)"
        }
        if ($completion.stderr) {
            Write-Host "HomeServer updater stderr:`n$($completion.stderr)"
        }
        if ($completion.result_json) {
            Write-Host "HomeServer updater result:`n$($completion.result_json)"
        }
        if ($completion.task_error) {
            throw "LocalSystem updater task failed to launch the helper: $($completion.task_error)"
        }
        if ([int]$completion.exit_code -ne 0) {
            $detail = if ($completion.result_json) {
                $result = $completion.result_json | ConvertFrom-Json
                "state=$($result.state), failure_code=$($result.failure_code), message=$($result.message)"
            }
            elseif ($completion.stderr) {
                $completion.stderr
            }
            else {
                "no updater result or stderr was produced"
            }
            throw "HomeServer updater helper failed with exit code $($completion.exit_code): $detail"
        }
        if (-not $completion.result_json) {
            throw "HomeServer updater did not write an application result"
        }
        $completion.result_json | ConvertFrom-Json
    }
    finally {
        & "$env:SystemRoot\System32\schtasks.exe" /Delete /TN $taskName /F 2>$null | Out-Null
        Remove-Item $taskDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}

try {
    $signature = Get-AuthenticodeSignature -LiteralPath $updateInstaller
    if ($signature.Status -ne "Valid" -or -not $signature.SignerCertificate) {
        throw "CI update installer does not have a valid trusted Authenticode signature"
    }
    if ($signature.SignerCertificate.Thumbprint -ne $SignerThumbprint) {
        throw "CI update installer signer thumbprint does not match the test input"
    }

    Set-CurrentUserPublisherTrust -Certificate $signature.SignerCertificate
    $windowsPowerShellSignature = Get-WindowsPowerShellSignature -Path $updateInstaller
    Write-Host "Windows PowerShell Authenticode status: $($windowsPowerShellSignature.status); $($windowsPowerShellSignature.status_message)"
    if ($windowsPowerShellSignature.status -ne "Valid") {
        throw "Windows PowerShell does not trust the CI update installer: $($windowsPowerShellSignature.status) $($windowsPowerShellSignature.status_message)"
    }
    if ($windowsPowerShellSignature.thumbprint -ne $SignerThumbprint) {
        throw "Windows PowerShell resolved an unexpected CI update signer"
    }

    Reset-HomeServerInstallationBoundary
    $install = Start-Process -FilePath $installer -ArgumentList "/S" -PassThru -Wait
    if ($install.ExitCode -ne 0) {
        throw "HomeServer installer failed with exit code $($install.ExitCode)"
    }
    $initialStatus = Wait-ForHomeServerHealth -ExpectedVersion $ExpectedVersion
    $currentVersion = $initialStatus.version

    $rollbackPlan = New-UpdatePlan -UpdateId "ci-rollback-$([guid]::NewGuid().ToString('N'))" -CurrentVersion $currentVersion -TargetVersion "9.9.9"
    $stagedSignature = Get-WindowsPowerShellSignature -Path $rollbackPlan.StagedInstaller
    Write-Host "Staged Windows PowerShell Authenticode status: $($stagedSignature.status); $($stagedSignature.status_message); signer=$($stagedSignature.thumbprint)"
    if ($stagedSignature.status -ne "Valid" -or $stagedSignature.thumbprint -ne $SignerThumbprint) {
        throw "The staged CI update installer does not retain its trusted Authenticode identity"
    }
    $rollbackResult = Invoke-UpdatePlan -Plan $rollbackPlan
    if ($rollbackResult.state -ne "rolled_back" -or $rollbackResult.failure_code -ne "post_update_health_failed") {
        throw "Forced update did not complete the automatic binary rollback path"
    }
    if (-not (Test-Path $rollbackPlan.RollbackDirectory)) {
        throw "Updater did not preserve the previous binary tree for rollback"
    }
    Wait-ForHomeServerHealth -ExpectedVersion $currentVersion | Out-Null

    $successPlan = New-UpdatePlan -UpdateId "ci-success-$([guid]::NewGuid().ToString('N'))" -CurrentVersion $currentVersion -TargetVersion $currentVersion
    $successResult = Invoke-UpdatePlan -Plan $successPlan
    if ($successResult.state -ne "succeeded" -or $successResult.failure_code) {
        throw "Verified update did not complete the successful health-confirmed path"
    }
    if (-not (Test-Path $successPlan.ArchivedInstaller)) {
        throw "Successful update did not preserve the verified installer"
    }
    Wait-ForHomeServerHealth -ExpectedVersion $currentVersion | Out-Null

    $uninstallerPath = Resolve-HomeServerUninstaller
    $uninstall = Start-Process -FilePath $uninstallerPath -ArgumentList "/S" -PassThru -Wait
    if ($uninstall.ExitCode -ne 0) {
        throw "HomeServer uninstaller failed with exit code $($uninstall.ExitCode)"
    }

    Write-Host "HomeServer signed update, Authenticode verification, health confirmation, automatic rollback, installer preservation, and cleanup smoke tests passed."
}
catch {
    $scriptExitCode = 1
    Write-Host "HomeServer updater smoke failure: $($_.Exception.Message)"
    if ($_.ScriptStackTrace) {
        Write-Host "Script stack:`n$($_.ScriptStackTrace)"
    }
    try {
        $lastResult = Read-TrimmedText -Path $resultPath -TimeoutSeconds 10
        if ($lastResult) {
            Write-Host "Last updater result:`n$lastResult"
        }
    }
    catch {
        Write-Host "Unable to read the last updater result: $($_.Exception.Message)"
    }
    $serviceQuery = & "$env:SystemRoot\System32\sc.exe" query $serviceName 2>&1 | Out-String
    Write-Host "Service query:`n$serviceQuery"
}
finally {
    try {
        Remove-CurrentUserPublisherTrust -Thumbprint $SignerThumbprint
    }
    catch {
        Write-Warning "Unable to remove temporary CurrentUser publisher trust: $($_.Exception.Message)"
    }
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
    if (Test-Path $diagnosticDirectory) {
        Remove-Item $diagnosticDirectory -Recurse -Force -ErrorAction SilentlyContinue
    }
}

$global:LASTEXITCODE = $scriptExitCode
if ($scriptExitCode -ne 0) {
    exit $scriptExitCode
}
