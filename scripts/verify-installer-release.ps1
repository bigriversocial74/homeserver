param(
    [Parameter(Mandatory = $true)]
    [string]$InstallerPath,

    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$ExpectedVersion
)

$ErrorActionPreference = "Stop"
$serviceName = "MicrogifterHomeServer"
$apiBase = "http://127.0.0.1:47831"
$controlHeaders = @{ "X-MG-Local-Client" = "microgifter-control-center-v1" }
$installer = (Resolve-Path $InstallerPath).Path
$installDirectory = Join-Path $env:ProgramFiles "Microgifter HomeServer"
$serviceBinary = Join-Path $installDirectory "resources\microgifter-homeserver-service.exe"
$dataDirectory = Join-Path $env:ProgramData "Microgifter\HomeServer"
$uninstallerPath = Join-Path $installDirectory "uninstall.exe"
$tempInstallerLog = Join-Path $env:TEMP "Microgifter-HomeServer-install.log"
$registryPaths = @(
    "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*",
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
)

function Get-HomeServerRegistryEntries {
    @(
        Get-ItemProperty $registryPaths -ErrorAction SilentlyContinue |
            Where-Object { $_.DisplayName -eq "Microgifter HomeServer" }
    )
}

function Get-HomeServerInstallProcesses {
    @(
        Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
            Where-Object {
                $path = [string]$_.ExecutablePath
                $commandLine = [string]$_.CommandLine
                ($path -and $path.StartsWith($installDirectory, [System.StringComparison]::OrdinalIgnoreCase)) -or
                ($commandLine -and $commandLine.IndexOf($installDirectory, [System.StringComparison]::OrdinalIgnoreCase) -ge 0)
            }
    )
}

function Wait-ForInstallationBoundary {
    param(
        [switch]$RemoveData
    )

    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        Stop-Service -Name $serviceName -Force -ErrorAction SilentlyContinue
        & "$env:SystemRoot\System32\sc.exe" delete $serviceName 2>$null | Out-Null
        Get-Process -Name "microgifter-homeserver*" -ErrorAction SilentlyContinue |
            Stop-Process -Force -ErrorAction SilentlyContinue

        if (Test-Path $installDirectory) {
            Remove-Item $installDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
        if ($RemoveData.IsPresent -and (Test-Path $dataDirectory)) {
            Remove-Item $dataDirectory -Recurse -Force -ErrorAction SilentlyContinue
        }
        foreach ($entry in Get-HomeServerRegistryEntries) {
            Remove-Item -LiteralPath $entry.PSPath -Recurse -Force -ErrorAction SilentlyContinue
        }

        $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
        $installProcesses = @(Get-HomeServerInstallProcesses)
        $registryEntries = @(Get-HomeServerRegistryEntries)
        $installRemoved = -not (Test-Path $installDirectory)
        $dataReady = -not $RemoveData.IsPresent -or -not (Test-Path $dataDirectory)

        if (-not $service -and $installProcesses.Count -eq 0 -and $registryEntries.Count -eq 0 -and $installRemoved -and $dataReady) {
            Start-Sleep -Milliseconds 1500

            $service = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
            $installProcesses = @(Get-HomeServerInstallProcesses)
            if (-not $service -and $installProcesses.Count -eq 0 -and -not (Test-Path $installDirectory)) {
                return
            }
        }

        Start-Sleep -Milliseconds 500
    }

    throw "Previous HomeServer installation did not fully release its Windows service, processes, files, or registration"
}

function Wait-ForHomeServer {
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

function Resolve-Uninstaller {
    $entry = Get-HomeServerRegistryEntries | Select-Object -First 1
    if ($entry) {
        $command = if ($entry.QuietUninstallString) { $entry.QuietUninstallString } else { $entry.UninstallString }
        if ($command -match '^"([^"]+)"') {
            return $matches[1]
        }
        if ($command -and (Test-Path $command)) {
            return $command
        }
    }
    if (Test-Path $uninstallerPath) {
        return $uninstallerPath
    }
    throw "Unable to locate the HomeServer uninstaller"
}

try {
    $existingUninstaller = $null
    try {
        $existingUninstaller = Resolve-Uninstaller
    }
    catch {
        $existingUninstaller = $null
    }
    if ($existingUninstaller -and (Test-Path $existingUninstaller)) {
        Start-Process -FilePath $existingUninstaller -ArgumentList "/S" -Wait -ErrorAction SilentlyContinue
    }
    Wait-ForInstallationBoundary -RemoveData
    Remove-Item -LiteralPath $tempInstallerLog -Force -ErrorAction SilentlyContinue

    $install = Start-Process -FilePath $installer -ArgumentList "/S" -PassThru -Wait
    if ($install.ExitCode -ne 0) {
        if (Test-Path $tempInstallerLog) {
            Write-Host "HomeServer installer diagnostic log:"
            Get-Content -LiteralPath $tempInstallerLog -Tail 200
        }
        throw "HomeServer installer failed with exit code $($install.ExitCode)"
    }

    Wait-ForHomeServer

    if (-not (Test-Path $serviceBinary)) {
        throw "Installed HomeServer service binary was not found at '$serviceBinary'"
    }

    $binaryVersion = (& $serviceBinary --version | Out-String).Trim()
    $expectedBinaryVersion = "MicrogifterHomeServer $ExpectedVersion"
    if ($binaryVersion -ne $expectedBinaryVersion) {
        throw "Installed service version mismatch. Expected '$expectedBinaryVersion' but received '$binaryVersion'."
    }

    $status = Invoke-RestMethod -Headers $controlHeaders -Uri "$apiBase/v1/status" -TimeoutSec 5
    if ($status.version -ne $ExpectedVersion) {
        throw "Installed API version mismatch. Expected '$ExpectedVersion' but received '$($status.version)'."
    }
    if ($status.state -ne "running" -or -not $status.api_available -or $status.database -ne "ready") {
        throw "Installed HomeServer did not report a healthy runtime state"
    }

    $registryEntry = Get-HomeServerRegistryEntries | Select-Object -First 1
    if (-not $registryEntry) {
        throw "Installed HomeServer registry entry was not found"
    }
    if ($registryEntry.DisplayVersion -and -not $registryEntry.DisplayVersion.StartsWith($ExpectedVersion)) {
        throw "Installer registry version mismatch. Expected '$ExpectedVersion' but received '$($registryEntry.DisplayVersion)'."
    }

    Write-Host "Verified installer, embedded service binary, local API, and Windows registration all report HomeServer $ExpectedVersion."
}
finally {
    $resolvedUninstaller = $null
    try {
        $resolvedUninstaller = Resolve-Uninstaller
    }
    catch {
        $resolvedUninstaller = $null
    }
    if ($resolvedUninstaller -and (Test-Path $resolvedUninstaller)) {
        Start-Process -FilePath $resolvedUninstaller -ArgumentList "/S" -Wait -ErrorAction SilentlyContinue
    }
    Wait-ForInstallationBoundary -RemoveData
}
