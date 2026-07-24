# Microgifter HomeServer

Microgifter HomeServer is the private local edge platform for Microgifter. It provides a Windows-first Control Center, native background services, local data and knowledge, optional local AI, MCP access, synchronization, backup, recovery, diagnostics, and secure updates.

## Product direction

The approved HomeServer v1 product and technical blueprint was adopted in `bigriversocial74/contactform` through PR #1341 and merge commit `80055acb325a6e5714f12ce9fd7d1283d20965a3`.

The dedicated repository is now the implementation authority. The blueprint will be maintained under `docs/product-technical-blueprint.md`.

## Primary customer release

`Microgifter-HomeServer-Setup.exe`

- Windows 11 x64 first.
- Tauri 2 Control Center.
- Native Windows service.
- Local API and embedded database.
- NSIS per-machine installer.
- Docker retained for later Linux, NAS, development, and appliance deployments.

## Status

Phase 1 foundation is starting. No production installer has been released.
