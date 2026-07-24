# Microgifter HomeServer

Microgifter HomeServer is the private local edge platform for Microgifter. It provides a Windows-first Control Center, native background services, local data and knowledge, optional local AI, MCP access, synchronization, backup, recovery, diagnostics, and secure updates.

## Product direction

The approved HomeServer v1 product and technical blueprint was adopted in `bigriversocial74/contactform` through PR #1341 and merge commit `80055acb325a6e5714f12ce9fd7d1283d20965a3`.

The dedicated repository is now the implementation authority. The blueprint is maintained under `docs/product-technical-blueprint.md`.

## Primary customer release

`Microgifter-HomeServer-Setup.exe`

- Windows 11 x64 first.
- Tauri 2 Control Center.
- Native Windows service.
- Loopback-only local API and embedded SQLite database.
- NSIS per-machine installer.
- Docker retained for later Linux, NAS, development, and appliance deployments.

## Backup and recovery

Phase 3A adds:

- Automatic encrypted SQLite backups.
- Manual protected backups.
- Portable passphrase-encrypted recovery packages.
- Package, hash, archive, and database integrity verification.
- Retention controls.
- Staged restore on service restart.
- Preservation of the current database for rollback.
- Automatic rollback when a restored database fails integrity checks.

Recovery passphrases are never stored. Device backup keys are stored in the operating-system credential vault.

## Development

On Windows with Node.js, Rust, and the Tauri prerequisites installed:

```powershell
./scripts/dev-windows.ps1
```

Build the service, tests, Control Center, and NSIS installer:

```powershell
./scripts/build-windows.ps1
```

## Status

- Phase 1 installable foundation: merged and validated.
- Phase 2 cloud pairing and synchronization: active draft PR, coordinated with a separate Microgifter cloud PR.
- Phase 3A backup and recovery: active scoped branch.
- Phase 3B signed updates: planned after Phase 3A.

No public production installer has been released or code signed.
