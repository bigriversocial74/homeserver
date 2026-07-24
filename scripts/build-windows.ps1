param(
    [switch]$SmokeTestInstaller
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$ServiceTarget = Join-Path $Root "target\release\microgifter-homeserver-service.exe"
$ResourceTarget = Join-Path $Root "src-tauri\resources\microgifter-homeserver-service.exe"

Push-Location $Root
try {
    if (-not (Test-Path "Cargo.lock") -or -not (Test-Path "package-lock.json")) {
        throw "Cargo.lock and package-lock.json are required for reproducible builds."
    }

    npm ci
    npm run check:frontend
    npm run build
    npm run prepare:icons
    npm audit --audit-level=high

    cargo fmt --all --check
    cargo test --workspace --locked
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo build --release --package microgifter-homeserver-service --locked
    & (Join-Path $PSScriptRoot "smoke-test-service.ps1") -ServiceBinary $ServiceTarget

    Copy-Item $ServiceTarget $ResourceTarget -Force
    npm run tauri:build

    $BundleDirectory = Join-Path $Root "target\release\bundle\nsis"
    $Installer = Get-ChildItem $BundleDirectory -Filter "*-setup.exe" | Select-Object -First 1
    if (-not $Installer) {
        throw "Tauri did not produce an NSIS installer."
    }

    $FinalInstaller = Join-Path $BundleDirectory "Microgifter-HomeServer-Setup.exe"
    Copy-Item $Installer.FullName $FinalInstaller -Force

    if ($SmokeTestInstaller) {
        & (Join-Path $PSScriptRoot "smoke-test-installer.ps1") -InstallerPath $FinalInstaller
    }

    Write-Host "Validated installer ready: $FinalInstaller"
}
finally {
    Pop-Location
}
