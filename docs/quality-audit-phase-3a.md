# Phase 3A Backup and Recovery Quality Audit

## Baseline score

**2.0/10**

The merged Phase 1 foundation exposed only a `last_backup` placeholder. It had no backup schema, encryption, recovery packages, verification, retention, restore staging, rollback, API controls, Control Center workflow, or installed-service validation.

## Implemented remediation

- Additive and idempotent SQLite backup/recovery migration.
- Typed backup, catalog, verification, and restore contracts.
- Consistent online SQLite snapshots using the SQLite backup API.
- AES-256-GCM package encryption.
- Device backup key stored in the operating-system credential vault.
- Portable recovery packages using Argon2id passphrase derivation.
- Passphrase length and request size limits.
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
- Control Center backup creation, recovery-package creation, verification, and restore staging.
- Console service smoke test covering both encryption modes, wrong-passphrase rejection, restart restore, and restored audit state.
- Installed LocalSystem service smoke test covering credential-vault encryption, verification, uninstall preservation, and persistent data.

## Security boundaries

- Backup packages never contain raw cloud credentials outside the encrypted SQLite snapshot.
- Recovery passphrases are accepted only for the current operation and are never persisted.
- Automatic and manual backup keys are stored outside SQLite in the OS credential vault.
- Restore accepts only a cataloged backup identity; arbitrary filesystem paths are not accepted by the local API.
- Restore requires the exact confirmation value `RESTORE`.
- A restore is staged and verified before restart; the live database is not replaced by an API request.
- The local API remains loopback-only.
- Backup and restore do not grant cloud commerce, payment, claim, redemption, ownership, campaign, reward, wallet, or PPPM authority.

## Phase 3A acceptance gates

- Immutable dependency lockfiles.
- Frontend syntax and production build.
- Dependency vulnerability audit.
- Strict Rust formatting.
- Full workspace tests.
- Full workspace clippy with warnings denied.
- Encrypted manual backup creation and verification.
- Encrypted recovery-package creation and verification.
- Wrong recovery passphrase rejection.
- Staged restore and service restart application.
- Restored SQLite integrity and migration verification.
- NSIS installer build.
- Installed LocalSystem service backup-key access.
- Default uninstall preserves SQLite data and encrypted backup packages.

## Explicit exclusions

Phase 3A does not claim:

- Remote backup destinations.
- Cloud backup upload.
- Arbitrary package import from external paths.
- Signed update manifest download.
- Installer signature verification and update application.
- Binary rollback after a failed application update.

Those update capabilities are Phase 3B and must be validated in a separate PR.

## Final score

The final score remains **pending** until every Windows workflow gate passes on the immutable PR head. No 10/10 claim is valid before that proof.
