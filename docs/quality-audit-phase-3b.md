# Phase 3B Signed Updates and Rollback Quality Audit

## Baseline score

**1.5/10**

The merged Phase 3A foundation could create a protected pre-update database backup, but it had no trusted release metadata, download validation, installer signer enforcement, external updater process, post-install health proof, persistent update audit, or binary rollback.

## Implemented remediation

- Typed stable-channel update contracts shared by the service, updater, Tauri bridge, and Control Center.
- Additive and idempotent SQLite update migration.
- Durable singleton runtime state, release records, and update events.
- Pinned Ed25519 release-manifest verification.
- Production build-time injection for the release key identity and public verification key.
- HTTPS-only manifest and installer URLs.
- Redirect limits and HTTP downgrade rejection.
- Bounded manifest, release-note, installer, rollback-tree, result-file, and request sizes.
- Product, schema, channel, semantic-version, minimum-version, publication-time, filename, size, hash, and signer validation.
- Streamed installer download into managed HomeServer staging.
- Exact installer byte count and SHA-256 verification.
- Windows Authenticode status and exact signer-thumbprint enforcement.
- No Control Center input for manifest URL, release key, installer URL, installer path, hash, or signer.
- Exact `UPDATE` confirmation before application.
- DPAPI-protected encrypted pre-update SQLite backup.
- Separate updater helper copied outside the installation directory.
- Pre-install snapshot of the installed HomeServer binary tree.
- Symlink rejection and rollback file/byte limits.
- Silent NSIS application.
- Loopback health and exact target-version verification.
- Automatic restoration of the prior binary tree when installation or health verification fails.
- Prior-version health verification after rollback.
- Verified installer preservation after successful application.
- Updater-result polling and persistent ingestion without requiring another reboot.
- Control Center signed-release status, notes, integrity metadata, download, apply, success, failure, and rollback messaging.
- Scheduled stable-channel checks every six hours.

## Security boundaries

- Only a public release-verification key is compiled into HomeServer.
- Production signing private keys must remain outside source control, installers, workflow artifacts, and HomeServer devices.
- Production builds can inject `MG_HOMESERVER_RELEASE_KEY_ID` and `MG_HOMESERVER_RELEASE_PUBLIC_KEY_BASE64` at compile time.
- A valid Ed25519 manifest does not bypass installer size, SHA-256, Authenticode, version, managed-path, or health checks.
- A valid Authenticode signature from an unrelated signer is rejected.
- Update URLs cannot contain credentials and cannot downgrade to HTTP through redirects.
- The updater helper accepts only a bounded plan whose installer, rollback, archive, and result paths remain inside expected managed boundaries.
- The local API remains bound to `127.0.0.1`.
- Update delivery grants no Microgifter cloud commerce, payment, claim, redemption, ownership, campaign, reward, wallet, or PPPM authority.

## Phase 3B acceptance gates

- Immutable Cargo and npm lockfiles.
- Frontend and PowerShell syntax validation.
- Frontend production build.
- Dependency vulnerability audit.
- Strict Rust formatting.
- Native updater-helper compilation.
- Native Windows-service compilation.
- Full workspace tests.
- Full workspace clippy with warnings denied.
- Valid signed manifest acceptance.
- Tampered manifest rejection.
- Untrusted key-identity rejection.
- Insecure installer URL rejection.
- Update migration idempotency and typed state round trip.
- Fresh loopback update API begins idle on the stable channel.
- Unstaged update download rejection.
- Incorrect apply confirmation rejection.
- Existing backup and disaster-recovery smoke suite remains green.
- NSIS installer includes both service and updater resources.
- Installed LocalSystem backup validation remains green.
- Ephemeral CI Authenticode certificate is trusted only on the Windows runner.
- Signed test installer passes the exact updater Authenticode check.
- Forced target-version mismatch triggers post-install health failure.
- Failed-health path restores the prior binary tree.
- Restored prior version passes loopback health verification.
- Same-version signed test installation completes the successful path.
- Successful update preserves the verified installer.
- Temporary CI certificate trust is removed.
- Original validated development installer is uploaded, not the CI-signed test copy.

## Explicit exclusions

Phase 3B does not claim:

- Hosting or operating `updates.microgifter.com`.
- Production release private-key custody.
- Production Authenticode certificate procurement.
- Remote fleet rollout orchestration.
- Background installation without explicit local confirmation.
- Downgrade installation through the stable update API.

Those operational release controls must be provisioned before public update distribution.

## Final score

The final score remains **pending** until every acceptance gate passes on one immutable PR head. No 10/10 claim is valid before that Windows proof.
