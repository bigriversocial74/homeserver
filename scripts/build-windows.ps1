$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$ServiceTarget = Join-Path $Root "target\release\microgifter-homeserver-service.exe"
$ResourceTarget = Join-Path $Root "src-tauri\resources\microgifter-homeserver-service.exe"

Push-Location $Root
try {
    cargo build --release --package microgifter-homeserver-service
    Copy-Item $ServiceTarget $ResourceTarget -Force
    cargo test --workspace
    npm install
    npm run check:frontend
    npm run tauri:build

    $BundleDirectory = Join-Path $Root "target\release\bundle\nsis"
    $Installer = Get-ChildItem $BundleDirectory -Filter "*-setup.exe" | Select-Object -First 1
    if (-not $Installer) {
        throw "Tauri did not produce an NSIS installer."
    }

    $FinalInstaller = Join-Path $BundleDirectory "Microgifter-HomeServer-Setup.exe"
    Copy-Item $Installer.FullName $FinalInstaller -Force
    Write-Host "Installer ready: $FinalInstaller"
}
finally {
    Pop-Location
}
