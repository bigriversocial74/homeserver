# Phase 1 — Installable Foundation Acceptance

Phase 1 establishes the buildable Windows foundation. It does not claim production readiness.

## Delivered in this phase

- Dedicated HomeServer repository.
- Tauri 2 Control Center shell.
- Rust workspace and shared health contract.
- Native Windows service entry point.
- Loopback-only local API at `127.0.0.1:47831`.
- Embedded SQLite database with WAL mode and foreign keys.
- Initial settings, event, and synchronization queue schema.
- NSIS per-machine installer configuration.
- Installer hooks that register, start, stop, and remove the Windows service.
- PowerShell development and installer build scripts.
- Windows CI for tests, frontend build, service build, and Tauri NSIS bundling.

## Acceptance checklist

- [ ] Clean Windows 11 runner installs Node and Rust dependencies.
- [ ] `cargo test --workspace` passes.
- [ ] Frontend JavaScript syntax checks pass.
- [ ] Vite production build passes.
- [ ] Service release binary builds.
- [ ] Tauri NSIS bundle builds.
- [ ] Workflow uploads `Microgifter-HomeServer-Setup.exe`.
- [ ] Installer requires elevation and installs per machine.
- [ ] Installer registers `MicrogifterHomeServer` as an automatic Windows service.
- [ ] Service binds only to `127.0.0.1:47831`.
- [ ] `/healthz` returns HTTP 204.
- [ ] `/v1/status` returns the shared health contract.
- [ ] Control Center reports running or offline state accurately.
- [ ] Uninstall removes the Windows service while preserving application data by default.

## Explicitly deferred

- Code signing certificate and signed release pipeline.
- Cloud pairing and device registration.
- Production synchronization.
- Backup and restore workflows.
- Model Center and Ollama installation.
- Knowledge Vault indexing.
- MCP gateway.
- LAN and remote access.
- Automatic updater.
- Repair UI and migration package UI.
