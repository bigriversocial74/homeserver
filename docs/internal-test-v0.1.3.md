# HomeServer v0.1.3 internal test build

This build is intended only for David Evans and a small, known test group before the public HomeServer launch.

## Important warning

The installer is signed with a temporary self-signed test certificate generated inside its GitHub Actions run. It is not signed by a publicly trusted Microgifter publisher certificate.

Windows may therefore display **Unknown publisher**, **Windows protected your PC**, or another SmartScreen warning. Testers must use **More info → Run anyway** only when the installer was received directly from David or another approved Microgifter test coordinator and its SHA-256 checksum matches `SHA256SUMS.txt`.

Do not repost this installer, advertise it as a public release, or distribute it through public download pages.

## Included files

- `Microgifter-HomeServer-v0.1.3-Internal-Test-Setup.exe` — clearly labeled installer for testers.
- `Microgifter-HomeServer-Setup.exe` — update-compatible copy used by the test manifest.
- `homeserver-internal-test.json` — manifest signed by the ephemeral internal-test Ed25519 key compiled into this build.
- `SHA256SUMS.txt` — SHA-256 checksums for the included files.
- `INTERNAL-TEST-WARNING.txt` — plain-text distribution warning.
- `release-notes.md` — v0.1.3 release notes.

## Tester procedure

1. Download the complete workflow artifact and extract it locally.
2. Verify the installer SHA-256 value against `SHA256SUMS.txt`.
3. Uninstall an older HomeServer test build when directed, while preserving retained HomeServer data.
4. Run `Microgifter-HomeServer-v0.1.3-Internal-Test-Setup.exe` as an administrator.
5. When Windows shows the untrusted-publisher warning, select **More info → Run anyway**.
6. Confirm the Control Center opens and the HomeServer service becomes healthy.
7. Test cloud pairing, synchronization, encrypted backup, recovery verification, restart behavior, and uninstall/data preservation.
8. Report the Windows version, exact steps, screenshots, logs, and whether the issue reproduced.

## Security boundary

The temporary Authenticode certificate and Ed25519 private key exist only during the GitHub Actions run and are removed during cleanup. Each internal build receives new signing material. An internal build cannot be used as a permanent update channel across independently generated test runs.

The public launch build will replace this process with the permanent production Authenticode certificate and protected HomeServer Ed25519 release keys.
