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
        $stdout = if (Test-Path $stdoutPath) { (Get-Content $stdoutPath -Raw).Trim() } else { "" }
        $stderr = if (Test-Path $stderrPath) { (Get-Content $stderrPath -Raw).Trim() } else { "" }
        $resultJson = if (Test-Path $resultPath) { (Get-Content $resultPath -Raw).Trim() } else { "" }

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

    $install = Start-Process -FilePath $installer -ArgumentList "/S" -PassThru -Wait
    if ($install.ExitCode -ne 0) {
        throw "HomeServer installer failed with exit code $($install.ExitCode)"
    }
    $initialStatus = Wait-ForHomeServerHealth -ExpectedVersion "0.1.0"
    $currentVersion = $initialStatus.version

    $rollbackPlan = New-UpdatePlan -UpdateId "ci-rollback-$([guid]::NewGuid().ToString('N'))" -CurrentVersion $currentVersion -TargetVersion "9.9.9"
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
