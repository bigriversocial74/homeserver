# HomeServer v0.1.3 production release runbook

This runbook publishes the exact commit tagged `v0.1.3` through `.github/workflows/release-v013.yml`. The workflow is intentionally fail-closed and will not publish an unsigned, mismatched, untested, or untagged build.

## GitHub environment

Create a protected GitHub Actions environment named `production-release`. Require manual approval for deployment to that environment.

Configure these environment secrets:

| Secret | Purpose |
|---|---|
| `WINDOWS_CODESIGN_PFX_BASE64` | Base64-encoded production Authenticode PFX containing the private key and certificate chain. |
| `WINDOWS_CODESIGN_PFX_PASSWORD` | Password protecting the production PFX. |
| `WINDOWS_CODESIGN_EXPECTED_THUMBPRINT` | Expected SHA-1 or SHA-256 certificate thumbprint with or without spaces. |
| `HOMESERVER_UPDATE_PRIVATE_KEY_BASE64` | Standard Base64 encoding of the 32-byte Ed25519 release signing seed. |
| `HOMESERVER_UPDATE_PUBLIC_KEY_BASE64` | Standard Base64 encoding of the matching 32-byte Ed25519 public key compiled into the release. |

Do not store the PFX, passwords, or Ed25519 private key in the repository, release artifacts, logs, workflow inputs, or PR comments.

## Pre-release gates

Before tagging:

1. Merge the release PR into `main` only after HomeServer Production Quality and the release-contract job are green.
2. Confirm `Cargo.toml`, `package.json`, and `src-tauri/tauri.conf.json` all report `0.1.3`.
3. Confirm `docs/releases/v0.1.3.md` contains the approved release notes.
4. Confirm the protected `production-release` environment contains all five secrets.
5. Confirm the production certificate has at least 14 days remaining and its thumbprint matches the configured secret.
6. Confirm the Ed25519 private key derives the configured public key.

## Tag and publish

Create an annotated `v0.1.3` tag on the approved `main` commit and push the tag. The tag push starts the production release workflow.

The workflow then:

1. Checks out the exact tag and proves the tag, commit, Cargo, npm, and Tauri versions match.
2. Imports the production Authenticode certificate and verifies its thumbprint and expiration.
3. Runs dependency audits, formatting, tests, strict linting, security regression checks, and the release contract validator.
4. Builds and signs the LocalSystem service and updater before staging them into the desktop package.
5. Configures Tauri to use the same production certificate and builds the NSIS installer.
6. Verifies the installer signature and creates both versioned and stable-channel filenames.
7. Performs installed LocalSystem, ACL, backup, uninstall, version, registry, signed-update, health, and automatic-rollback tests.
8. Generates the Ed25519-signed stable manifest and SHA-256 checksum file.
9. Uploads a retained workflow artifact.
10. Publishes the GitHub Release only after every prior gate succeeds.

## Dry-run verification

Use the workflow-dispatch input `release_tag=v0.1.3` with `publish=false` to rebuild and verify an existing tag without creating a GitHub Release. The protected environment and all production secrets are still required because the dry run exercises the real production signing path.

## Published files

The GitHub Release must contain:

- `Microgifter-HomeServer-v0.1.3-Setup.exe`
- `Microgifter-HomeServer-Setup.exe`
- `homeserver-stable.json`
- `SHA256SUMS.txt`

The stable manifest points to the unversioned installer within the immutable `v0.1.3` release URL. The versioned installer is the customer-facing download.

## Failure handling

A failed workflow must not be bypassed by manually uploading files to a release. Diagnose and repair the release branch, merge through the normal PR process, create a new tag only when appropriate, and rerun the complete production workflow.

If a GitHub Release was created with incorrect assets, remove the release from distribution, rotate compromised signing material when applicable, and publish a corrected version under a new semantic version. Do not silently replace an immutable release artifact.
