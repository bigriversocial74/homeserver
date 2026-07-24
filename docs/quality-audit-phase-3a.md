# Phase 3A Backup and Recovery Quality Audit

## Baseline score

**2.0/10**

The merged Phase 1 foundation exposed only a `last_backup` placeholder. It had no backup schema, encryption, portable recovery packages, verification, retention, restore staging, rollback, transfer workflow, Control Center operations, or installed-service validation.

## Implemented remediation

- Additive and idempotent SQLite backup/recovery migration.
- Typed backup, catalog, verification, restore, import, and export contracts.
- Consistent online SQLite snapshots using the SQLite backup API.
- AES-256-GCM package encryption.
- Automatic and manual backup key protected by Windows DPAPI in the HomeServer data directory.
- Portable recovery packages using Argon2id passphrase derivation.
- Recovery passphrases are zeroized after use and never stored.
- Character-count, encoded-header, request, package, archive, and database size limits.
- Package magic, format version, bounded header, and bounded package/database sizes.
- SHA-256 verification for the compressed archive and extracted SQLite database.
- Safe archive extraction that accepts only the manifest and database paths.
- SQLite quick-check and migration verification before a package is accepted.
- Atomic package writes.
- Manual, automatic, recovery, and pre-update backup kinds.
- Automatic 24-hour schedule with 14-backup retention.
- Restore staging without replacing a live database.
- Restore application before SQLite opens on service startup.
- Preservation of the previous database for rollback.
- Automatic rollback when a staged database fails integrity validation.
- Invalid staged-restore quarantine to prevent a restart loop.
- Streamed recovery-package export from cataloged managed storage.
- Streamed recovery-package import into managed staging using bounded `tokio-util` readers without accepting filesystem paths through the API.
- Import registration becomes externally visible only after package decryption, archive verification, database verification, and catalog validation succeed.
- Failed first-time imports leave no catalog record and no managed package.
- Native Tauri Open and Save dialogs with bounded streaming I/O.
- Control Center backup creation, recovery-package creation, export, import, verification, and restore staging.
- Console service smoke coverage for both encryption modes, wrong-passphrase rejection, export, fresh-install import, clean failed-import handling, restart restore, and restored audit state.
- Installed LocalSystem service smoke coverage for DPAPI encryption, verification, uninstall preservation, and persistent data.

## Security boundaries

- Backup packages never contain raw cloud credentials outside the encrypted SQLite snapshot.
- Recovery passphrases are accepted only for the current operation and are never persisted.
- Automatic and manual backup keys are stored outside SQLite as a Windows DPAPI-protected key file.
- The backup key fails closed when it cannot be decrypted; HomeServer does not silently rotate it and orphan existing backups.
- Import and export stream package bytes and never accept arbitrary filesystem paths through the service API.
- Export accepts only a cataloged portable recovery identity whose package resolves inside managed recovery storage.
- Restore accepts only a cataloged backup identity.
- Restore requires the exact confirmation value `RESTORE`.
- A restore is staged and verified before restart; the live database is not replaced by an API request.
- The local API remains loopback-only.
- Backup and restore do not grant cloud commerce, payment, claim, redemption, ownership, campaign, reward, wallet, or PPPM authority.

## Phase 3A acceptance gates

- Immutable dependency lockfiles.
- Frontend and PowerShell syntax validation.
- Frontend production build.
- Dependency vulnerability audit.
- Strict Rust formatting.
- Native Windows service compilation.
- Full workspace tests.
- Full workspace clippy with warnings denied.
- Maximum 256-character multibyte recovery passphrase acceptance.
- Oversized encoded passphrase-header rejection.
- Encrypted manual backup creation and verification.
- Encrypted recovery-package creation and verification.
- Wrong recovery passphrase rejection.
- Portable recovery-package export.
- Fresh-install recovery catalog starts empty.
- Wrong import passphrase returns HTTP 422 and leaves no catalog or package residue.
- Correct import preserves the package identity and reaches verified state.
- Imported recovery can be staged and restored on a fresh HomeServer installation.
- Staged restore and service restart application.
- Restored SQLite integrity and migration verification.
- Pre-restore database preservation for rollback.
- NSIS installer build.
- Installed LocalSystem service backup-key access.
- Default uninstall preserves SQLite data and encrypted backup packages.

## Validation evidence

Windows production-quality run `30131612185` passed every gate on implementation head `834f4cf41246503b86e6aa69f968628e32de966a`:

- Dependency lock stability and diagnostic artifact upload.
- Frontend, PowerShell, vulnerability, resource, and rustfmt validation.
- Native Windows service compilation.
- Full workspace tests and strict clippy.
- Encrypted backup, exported recovery, failed-import cleanup, fresh-install import, staged restore, restored integrity, and rollback-ready console smoke testing.
- NSIS installer build and normalized installer artifact.
- Installed LocalSystem service backup, verification, uninstall preservation, and validated installer upload.

## Explicit exclusions

Phase 3A does not claim:

- Remote backup destinations.
- Cloud backup upload.
- Signed update manifest download.
- Installer signature verification and update application.
- Binary rollback after a failed application update.

Those update capabilities are Phase 3B and must be validated in a separate PR.

## Final score

**10.0/10 — Phase 3A scope complete and fully validated.**

The score applies only to the documented Phase 3A backup and disaster-recovery scope. Phase 3B signed-update capabilities remain excluded and require their own implementation and validation PR.
