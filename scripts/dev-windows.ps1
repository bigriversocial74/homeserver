$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot

Push-Location $Root
try {
    npm install
    Start-Process powershell -ArgumentList "-NoExit", "-Command", "Set-Location '$Root'; cargo run --package microgifter-homeserver-service -- console"
    npm run tauri:dev
}
finally {
    Pop-Location
}
