# Microgifter HomeServer

Microgifter HomeServer is the private Windows edge platform for Microgifter. It provides a native Control Center, a background Windows service, local SQLite storage, cloud pairing and synchronization, encrypted backup and recovery, a local Knowledge Vault, bounded Ollama model management, a client-scoped read-only MCP runtime, diagnostics, and a signed-update/rollback foundation.

The dedicated `bigriversocial74/homeserver` repository is the implementation authority. The approved product and technical blueprint is maintained in `docs/product-technical-blueprint.md`.

## Current release line

- Current release source version: `0.1.3`.
- Windows 11 x64 first.
- Tauri 2 Control Center.
- Native delayed-auto Windows service with recovery actions.
- Loopback-only API at `127.0.0.1:47831`.
- Embedded SQLite database using WAL, foreign keys, and integrity checks.
- NSIS per-machine installer.
- Internal **Run Anyway** workflow for a small, known test group using temporary self-signed Authenticode and ephemeral Ed25519 keys.
- Tag-gated production release pipeline with permanent Authenticode signing, protected Ed25519 update-manifest signing, checksums, installed verification, signed-update testing, and rollback testing.

The customer-facing production installer artifact is versioned as:

`Microgifter-HomeServer-v<version>-Setup.exe`

The stable production update channel also publishes:

- `Microgifter-HomeServer-Setup.exe`
- `homeserver-stable.json`
- `SHA256SUMS.txt`

Internal test builds are uploaded only as retained GitHub Actions artifacts and use the clearly labeled filename:

`Microgifter-HomeServer-v0.1.3-Internal-Test-Setup.exe`

They are not public releases, are not signed by a publicly trusted publisher certificate, and may show the Windows **More info → Run anyway** prompt. See `docs/internal-test-v0.1.3.md` for controlled tester instructions.

See `docs/release-v0.1.3.md` for both the temporary internal-test procedure and the protected production-release procedure. GitHub Releases remains the source of truth for whether a production-signed installer has been publicly published.

## Security boundaries

- The service does not expose a LAN, public database, model, or MCP listener. Ollama is fixed to `127.0.0.1:11434`; MCP is fixed to `127.0.0.1:47831/mcp` and requires an active scoped client token.
- Browser-originated local API requests are rejected.
- State-changing local API calls require the trusted Control Center marker.
- Cloud requests use HTTPS, bearer credentials, Ed25519 request signatures, timestamps, and nonces.
- Cloud and backup secrets use Windows machine-scoped DPAPI and fail closed on unsupported platforms.
- `%ProgramData%\Microgifter\HomeServer` is restricted to LocalSystem and local administrators by the installer.
- Recovery imports, cloud responses, synchronization queues, logs, and archive extraction are bounded.
- Update manifests are signed, installers are hash- and Authenticode-verified, and update paths are constrained to canonical managed directories.

See `docs/quality-audit-full-production.md` for the complete repository audit and acceptance rubric.

## Backup and recovery

HomeServer supports:

- Automatic encrypted SQLite backups.
- Manual protected backups.
- Portable passphrase-encrypted recovery packages.
- Native Control Center export and import dialogs.
- Streamed package transfer through the loopback-only service API.
- Fresh-install recovery from an exported `.mghbackup` package.
- Package, hash, archive, and database integrity verification.
- Failed-import cleanup with no residual managed package.
- Retention controls.
- Staged restore on service restart.
- Preservation of the current database for rollback.
- Automatic rollback when a restored database fails integrity checks.

Recovery passphrases are never stored. Automatic/manual backup keys and cloud credentials are protected with Windows machine-scoped DPAPI. Portable recovery packages use an independent Argon2id-derived passphrase key so they remain importable after reinstalling HomeServer or moving to another Windows installation.

## Development

On Windows with Node.js, Rust, and the Tauri prerequisites installed:

```powershell
./scripts/dev-windows.ps1
```

Build the service, tests, Control Center, and NSIS installer:

```powershell
./scripts/build-windows.ps1
```

The production-quality workflow validates synchronized release metadata, immutable dependency locks, pinned GitHub Actions, frontend and PowerShell syntax, static security boundaries, npm and RustSec dependency audits, clean native compilation, workspace tests, strict Clippy, encrypted backup/recovery, cloud contract behavior, loopback API defenses, NSIS packaging, installed LocalSystem behavior, ProgramData ACLs, installed binary/API/registry versions, signed updates, health confirmation, and automatic rollback.

The v0.1.3 internal-test workflow performs the same source, installer, installed-service, signed-update, health, and rollback validation while generating temporary signing material inside the isolated runner. It uploads a private test artifact and has read-only repository permissions; it cannot create a GitHub Release.

The v0.1.3 production workflow additionally requires an exact semantic-version tag, a protected production-release environment, a matching permanent Authenticode certificate, a matching protected Ed25519 release key pair, production-signed native binaries and installer, a signed stable update manifest, SHA-256 checksums, and successful installed update/rollback verification before GitHub Release publication.

## Implemented phases

- Phase 1: installable Windows foundation.
- Phase 2: cloud pairing and bounded signed synchronization.
- Phase 3A: encrypted backup, portable recovery, restore, and rollback.
- Phase 3B foundation: signed update verification, application, health validation, and rollback.
- Phase 4A foundation: managed local text-document import, indexing, search, change detection, and deletion.
- Phase 4B foundation: fixed-loopback Ollama detection, approved model catalog, local inventory, bounded pull tracking, local testing, unload/delete controls, default model assignments, and a user-controlled Windows installation assistant.
- Phase 4C foundation: local PDF and DOCX extraction, page metadata, scanned-document detection, bounded local Tesseract/Poppler OCR, retry-safe extraction state, and Control Center runtime guidance.
- Phase 4D foundation: bounded local embeddings, restart-safe semantic indexing, stale detection, page-aware cited keyword/semantic/hybrid retrieval, and Control Center progress.
- Phase 5A foundation: fixed-loopback Streamable HTTP and packaged stdio MCP transports, hashed client credentials, revocation, rate limits, audit receipts, and client-scoped read-only local tools/resources.

Supervised state-changing agents and broader Linux/NAS deployment remain future phases and are not represented as complete.
