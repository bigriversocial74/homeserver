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
$sourcePath = Join-Path $PSScriptRoot "smoke-test-updater.ps1"
$source = Get-Content -LiteralPath $sourcePath -Raw
$needle = '    Write-Host "Service query:`n$serviceQuery"'
if (($source.Split($needle).Count - 1) -ne 1) {
    throw "Unable to locate updater failure diagnostic insertion point"
}

$diagnostic = @'

    $evidenceRoot = if ($env:RUNNER_TEMP) {
        Join-Path $env:RUNNER_TEMP "homeserver-updater-diagnostic"
    } else {
        Join-Path $env:TEMP "homeserver-updater-diagnostic"
    }
    New-Item -ItemType Directory -Force $evidenceRoot | Out-Null
    Write-Host "Updater diagnostic evidence root: $evidenceRoot"

    function Save-DiagnosticText {
        param([string]$Name, [scriptblock]$Command)
        $path = Join-Path $evidenceRoot $Name
        try {
            $value = & $Command 2>&1 | Out-String
        }
        catch {
            $value = $_ | Out-String
        }
        $value | Set-Content -LiteralPath $path -Encoding UTF8
        Write-Host "--- $Name ---`n$value"
    }

    Save-DiagnosticText "service-qc.txt" { & "$env:SystemRoot\System32\sc.exe" qc $serviceName }
    Save-DiagnosticText "service-queryex.txt" { & "$env:SystemRoot\System32\sc.exe" queryex $serviceName }
    Save-DiagnosticText "service-qfailure.txt" { & "$env:SystemRoot\System32\sc.exe" qfailure $serviceName }
    Save-DiagnosticText "service-qfailureflag.txt" { & "$env:SystemRoot\System32\sc.exe" qfailureflag $serviceName }
    Save-DiagnosticText "service-qsidtype.txt" { & "$env:SystemRoot\System32\sc.exe" qsidtype $serviceName }
    Save-DiagnosticText "service-cim.txt" { Get-CimInstance Win32_Service -Filter "Name='$serviceName'" | Format-List * }
    Save-DiagnosticText "service-registry.txt" { Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Services\$serviceName" | Format-List * }
    Save-DiagnosticText "microgifter-processes.txt" {
        Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like 'microgifter*' -or $_.ExecutablePath -like '*Microgifter HomeServer*' } |
            Select-Object ProcessId, ParentProcessId, Name, ExecutablePath, CommandLine, CreationDate | Format-List
    }
    Save-DiagnosticText "port-47831.txt" {
        Get-NetTCPConnection -LocalPort 47831 -ErrorAction SilentlyContinue |
            Select-Object LocalAddress, LocalPort, RemoteAddress, RemotePort, State, OwningProcess | Format-Table -AutoSize
        & "$env:SystemRoot\System32\netstat.exe" -ano | Select-String ':47831'
    }
    Save-DiagnosticText "install-tree.txt" { Get-ChildItem -LiteralPath $installDirectory -Recurse -Force | Select-Object FullName, Length, LastWriteTimeUtc | Format-Table -AutoSize }
    Save-DiagnosticText "data-tree.txt" { Get-ChildItem -LiteralPath $dataDirectory -Recurse -Force | Select-Object FullName, Length, LastWriteTimeUtc | Format-Table -AutoSize }
    Save-DiagnosticText "install-acl.txt" { & "$env:SystemRoot\System32\icacls.exe" $installDirectory /T /C }
    Save-DiagnosticText "data-acl.txt" { & "$env:SystemRoot\System32\icacls.exe" $dataDirectory /T /C }

    $serviceBinary = Join-Path $installDirectory "resources\microgifter-homeserver-service.exe"
    Save-DiagnosticText "service-binary.txt" {
        if (Test-Path -LiteralPath $serviceBinary) {
            Get-Item -LiteralPath $serviceBinary | Select-Object FullName, Length, LastWriteTimeUtc, VersionInfo | Format-List
            Get-FileHash -Algorithm SHA256 -LiteralPath $serviceBinary | Format-List
            & $serviceBinary --version
        } else {
            "Service binary is missing: $serviceBinary"
        }
    }

    $since = (Get-Date).AddMinutes(-20)
    Save-DiagnosticText "system-service-events.txt" {
        Get-WinEvent -FilterHashtable @{ LogName = 'System'; StartTime = $since; ProviderName = 'Service Control Manager' } -ErrorAction SilentlyContinue |
            Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message | Format-List
    }
    Save-DiagnosticText "application-events.txt" {
        Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $since } -ErrorAction SilentlyContinue |
            Where-Object { $_.Message -match 'Microgifter|HomeServer|microgifter-homeserver-service' -or $_.ProviderName -match 'Application Error|Windows Error Reporting' } |
            Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message | Format-List
    }

    $logsDirectory = Join-Path $dataDirectory "logs"
    if (Test-Path -LiteralPath $logsDirectory) {
        foreach ($logFile in Get-ChildItem -LiteralPath $logsDirectory -File -ErrorAction SilentlyContinue) {
            try {
                Copy-Item -LiteralPath $logFile.FullName -Destination (Join-Path $evidenceRoot $logFile.Name) -Force -ErrorAction Stop
            }
            catch {
                Save-DiagnosticText ("copy-error-" + $logFile.Name + ".txt") { $_ | Format-List * -Force }
            }
            Save-DiagnosticText ("tail-" + $logFile.Name + ".txt") { Get-Content -LiteralPath $logFile.FullName -Tail 300 -ErrorAction Stop }
        }
    }

    if (Test-Path -LiteralPath $serviceBinary) {
        $probeRoot = Join-Path $env:SystemRoot "Temp\Microgifter-HomeServer-Direct-Probe"
        Remove-Item -LiteralPath $probeRoot -Recurse -Force -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Force $probeRoot | Out-Null
        $probeScript = Join-Path $probeRoot "run-console-probe.ps1"
        $probeMarker = Join-Path $probeRoot "complete.json"
        $probeStdout = Join-Path $probeRoot "console.stdout.log"
        $probeStderr = Join-Path $probeRoot "console.stderr.log"
        $probeHealth = Join-Path $probeRoot "health.txt"
        $probePid = Join-Path $probeRoot "pid.txt"
        $binaryLiteral = $serviceBinary.Replace("'", "''")
        $stdoutLiteral = $probeStdout.Replace("'", "''")
        $stderrLiteral = $probeStderr.Replace("'", "''")
        $healthLiteral = $probeHealth.Replace("'", "''")
        $pidLiteral = $probePid.Replace("'", "''")
        $markerLiteral = $probeMarker.Replace("'", "''")

        @"
`$ErrorActionPreference = 'Continue'
`$result = [ordered]@{ started = `$false; exited = `$null; exit_code = `$null; error = `$null }
try {
    `$process = Start-Process -FilePath '$binaryLiteral' -ArgumentList 'console' -RedirectStandardOutput '$stdoutLiteral' -RedirectStandardError '$stderrLiteral' -PassThru
    `$result.started = `$true
    `$process.Id | Set-Content -LiteralPath '$pidLiteral'
    Start-Sleep -Seconds 15
    try {
        Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:47831/healthz' -TimeoutSec 3 | Out-String | Set-Content -LiteralPath '$healthLiteral'
    } catch {
        (`$_ | Out-String) | Set-Content -LiteralPath '$healthLiteral'
    }
    `$process.Refresh()
    `$result.exited = `$process.HasExited
    if (`$process.HasExited) {
        `$result.exit_code = `$process.ExitCode
    } else {
        Stop-Process -Id `$process.Id -Force -ErrorAction SilentlyContinue
    }
} catch {
    `$result.error = `$_ | Out-String
}
`$result | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath '$markerLiteral' -Encoding UTF8
"@ | Set-Content -LiteralPath $probeScript -Encoding UTF8

        $taskName = "MicrogifterHomeServerDirectProbe-$([guid]::NewGuid().ToString('N'))"
        $taskTime = (Get-Date).AddMinutes(1).ToString("HH:mm")
        $powershell = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
        $taskCommand = "`"$powershell`" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$probeScript`""
        try {
            & "$env:SystemRoot\System32\schtasks.exe" /Create /TN $taskName /TR $taskCommand /SC ONCE /ST $taskTime /RU SYSTEM /RL HIGHEST /F | Out-Null
            & "$env:SystemRoot\System32\schtasks.exe" /Run /TN $taskName | Out-Null
            for ($attempt = 0; $attempt -lt 120 -and -not (Test-Path -LiteralPath $probeMarker); $attempt++) {
                Start-Sleep -Milliseconds 500
            }
            foreach ($probeFile in @($probeMarker, $probeStdout, $probeStderr, $probeHealth, $probePid)) {
                if (Test-Path -LiteralPath $probeFile) {
                    Copy-Item -LiteralPath $probeFile -Destination $evidenceRoot -Force -ErrorAction SilentlyContinue
                }
            }
            Save-DiagnosticText "direct-console-probe.txt" {
                if (Test-Path -LiteralPath $probeMarker) { Get-Content -LiteralPath $probeMarker -Raw }
                if (Test-Path -LiteralPath $probeHealth) { "HEALTH:`n"; Get-Content -LiteralPath $probeHealth -Raw }
                if (Test-Path -LiteralPath $probeStdout) { "STDOUT:`n"; Get-Content -LiteralPath $probeStdout -Raw }
                if (Test-Path -LiteralPath $probeStderr) { "STDERR:`n"; Get-Content -LiteralPath $probeStderr -Raw }
            }
        }
        catch {
            Save-DiagnosticText "direct-console-probe-error.txt" { $_ | Format-List * -Force }
        }
        finally {
            & "$env:SystemRoot\System32\schtasks.exe" /Delete /TN $taskName /F 2>$null | Out-Null
        }
    }
'@

$harnessPath = if ($env:RUNNER_TEMP) {
    Join-Path $env:RUNNER_TEMP "diagnostic-smoke-test-updater.ps1"
} else {
    Join-Path $env:TEMP "diagnostic-smoke-test-updater.ps1"
}
$source = $source.Replace($needle, $needle + $diagnostic)
Set-Content -LiteralPath $harnessPath -Value $source -Encoding UTF8

$tokens = $null
$errors = $null
[System.Management.Automation.Language.Parser]::ParseFile($harnessPath, [ref]$tokens, [ref]$errors) | Out-Null
if ($errors.Count -gt 0) {
    $errors | Format-List | Out-String | Write-Host
    throw "Diagnostic updater harness has syntax errors"
}

& pwsh -NoLogo -NoProfile -NonInteractive `
    -File $harnessPath `
    -InstallerPath $InstallerPath `
    -UpdateInstallerPath $UpdateInstallerPath `
    -SignerThumbprint $SignerThumbprint `
    -ExpectedVersion $ExpectedVersion
exit $LASTEXITCODE
