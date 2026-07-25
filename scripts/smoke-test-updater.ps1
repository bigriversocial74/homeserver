param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,

    [Parameter(Mandatory = $true)]
    [string]$UpdateInstallerPath,

    [Parameter(Mandatory = $true)]
    [string]$SignerThumbprint
)

$ErrorActionPreference = "Stop"
$serviceName = "MicrogifterHomeServer"
$apiBase = "http://127.0.0.1:47831"
$installer = (Resolve-Path $InstallerPath).Path
$updateInstaller = (Resolve-Path $UpdateInstallerPath).Path
$dataDirectory = Join-Path $env:ProgramData "Microgifter\HomeServer"
$installDirectory = Join-Path $env:ProgramFiles "Microgifter HomeServer"
$stagingDirectory = Join-Path $dataDirectory "updates\staging"
$rollbackRoot = Join-Path $dataDirectory "updates\rollback"
$installedArchive = Join-Path $dataDirectory "updates\installed"
$resultPath = Join-Path $dataDirectory "updates\last-update-result.json"
$uninstallerPath = $null
$scriptExitCode = 0
$currentUserTrustStores = @("Root", "TrustedPublisher")

function Read-TrimmedText {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return ""
    }
    $content = Get-Content -LiteralPath $Path -Raw
    if ($null -eq $content) {
        return ""
    }
    return $content.Trim()
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
        }
        finally {
            $store.Close()
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

    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        try {
            $service = Get-Service -Name $serviceName -ErrorAction Stop
            if ($service.Status -eq "Running") {
                $health = Invoke-WebRequest -UseBasicParsing -Uri "$apiBase/healthz" -TimeoutSec 2
                if ($health.StatusCode -eq 204) {
                    $status = Invoke-RestMethod -Uri "$apiBase/v1/status" -TimeoutSec 3
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

    $diagnosticId = [guid]::NewGuid().ToString("N")
    $updaterCopy = Join-Path $stagingDirectory "updater-smoke-$diagnosticId.exe"
    $stdoutPath = Join-Path $stagingDirectory "updater-smoke-$diagnosticId.stdout.log"
    $stderrPath = Join-Path $stagingDirectory "updater-smoke-$diagnosticId.stderr.log"
    Copy-Item $installedUpdater $updaterCopy -Force
    Remove-Item $resultPath -Force -ErrorAction SilentlyContinue

    try {
        $process = Start-Process -FilePath $updaterCopy -ArgumentList @("apply", $Plan.PlanPath) -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -PassThru -Wait
        $stdout = Read-TrimmedText -Path $stdoutPath
        $stderr = Read-TrimmedText -Path $stderrPath
        $resultJson = Read-TrimmedText -Path $resultPath

        if ($stdout) {
            Write-Host "HomeServer updater stdout:`n$stdout"
        }
        if ($stderr) {
            Write-Host "HomeServer updater stderr:`n$stderr"
        }
        if ($resultJson) {
            Write-Host "HomeServer updater result:`n$resultJson"
        }

        if ($process.ExitCode -ne 0) {
            $detail = if ($resultJson) {
                $result = $resultJson | ConvertFrom-Json
                "state=$($result.state), failure_code=$($result.failure_code), message=$($result.message)"
            }
            elseif ($stderr) {
                $stderr
            }
            else {
                "no updater result or stderr was produced"
            }
            throw "HomeServer updater helper failed with exit code $($process.ExitCode): $detail"
        }
        if (-not $resultJson) {
            throw "HomeServer updater did not write an application result"
        }
        $resultJson | ConvertFrom-Json
    }
    finally {
        Remove-Item $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
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

    $install = Start-Process -FilePath $installer -ArgumentList "/S" -PassThru -Wait
    if ($install.ExitCode -ne 0) {
        throw "HomeServer installer failed with exit code $($install.ExitCode)"
    }
    $initialStatus = Wait-ForHomeServerHealth -ExpectedVersion "0.1.0"
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
    if (Test-Path $resultPath) {
        Write-Host "Last updater result:`n$(Get-Content $resultPath -Raw)"
    }
    $serviceQuery = & "$env:SystemRoot\System32\sc.exe" query $serviceName 2>&1 | Out-String
    Write-Host "Service query:`n$serviceQuery"
}
finally {
    Remove-CurrentUserPublisherTrust -Thumbprint $SignerThumbprint
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

if ($scriptExitCode -ne 0) {
    exit $scriptExitCode
}
