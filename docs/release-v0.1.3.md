# HomeServer v0.1.3 release runbook

HomeServer v0.1.3 has two deliberately separate distribution paths:

1. **Internal Run Anyway build** for David Evans and a small, known test group.
2. **Production release** for launch after the permanent Windows code-signing certificate and protected Ed25519 release keys are available.

The internal path does not weaken or replace the production path.

## Internal Run Anyway build

Use `.github/workflows/internal-test-v013.yml` for pre-launch testing.

The workflow-dispatch screen exposes:

`RUN ANYWAY: build with temporary self-signed test credentials`

Leave that option enabled and run the workflow from the approved v0.1.3 source branch or merged `main` commit.

The internal workflow:

1. Verifies the Cargo, npm, and Tauri versions match.
2. Generates a new ephemeral Ed25519 key pair inside the isolated runner.
3. Compiles the temporary public key into that exact HomeServer build.
4. Generates and locally trusts a temporary self-signed Windows code-signing certificate.
5. Runs dependency audits, formatting, tests, strict linting, security checks, installer verification, installed service checks, signed-update testing, health confirmation, and automatic rollback.
6. Builds `Microgifter-HomeServer-v0.1.3-Internal-Test-Setup.exe`.
7. Generates `homeserver-internal-test.json`, release notes, a warning file, and SHA-256 checksums.
8. Uploads the complete bundle as a retained GitHub Actions artifact.
9. Removes the temporary certificate and private key material during cleanup.

The internal workflow has read-only repository permissions and contains no GitHub Release publication command.

Windows may show **Unknown publisher** or **Windows protected your PC** because the certificate is not publicly trusted. Approved testers may use **More info → Run anyway** after receiving the complete artifact directly from David or another authorized Microgifter test coordinator and verifying `SHA256SUMS.txt`.

See `docs/internal-test-v0.1.3.md` for the tester handoff.

Each internal run uses new signing material. Do not treat independently generated internal builds as one permanent automatic-update channel, and do not distribute an internal artifact publicly.

## Production GitHub environment

The launch workflow is `.github/workflows/release-v013.yml`. It publishes the exact commit tagged `v0.1.3` and remains intentionally fail-closed.

Create a protected GitHub Actions environment named `production-release`. Require manual approval for deployment to that environment.

Configure these environment secrets before launch:

| Secret | Purpose |
|---|---|
| `WINDOWS_CODESIGN_PFX_BASE64` | Base64-encoded production Authenticode PFX containing the private key and certificate chain. |
| `WINDOWS_CODESIGN_PFX_PASSWORD` | Password protecting the production PFX. |
| `WINDOWS_CODESIGN_EXPECTED_THUMBPRINT` | Expected SHA-1 or SHA-256 certificate thumbprint with or without spaces. |
| `HOMESERVER_UPDATE_PRIVATE_KEY_BASE64` | Standard Base64 encoding of the 32-byte Ed25519 release signing seed. |
| `HOMESERVER_UPDATE_PUBLIC_KEY_BASE64` | Standard Base64 encoding of the matching 32-byte Ed25519 public key compiled into the release. |

Do not store the PFX, passwords, or Ed25519 private key in the repository, release artifacts, logs, workflow inputs, or PR comments.

## Production pre-release gates

Before tagging:

1. Merge the release PR into `main` only after HomeServer Production Quality, the production release contract, and the internal-test contract are green.
2. Confirm `Cargo.toml`, `package.json`, and `src-tauri/tauri.conf.json` all report `0.1.3`.
3. Confirm `docs/releases/v0.1.3.md` contains the approved release notes.
4. Confirm the protected `production-release` environment contains all five secrets.
5. Confirm the production certificate has at least 14 days remaining and its thumbprint matches the configured secret.
6. Confirm the Ed25519 private key derives the configured public key.

## Production tag and publish

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

## Production dry-run verification

Use the production workflow-dispatch input `release_tag=v0.1.3` with `publish=false` to rebuild and verify an existing tag without creating a GitHub Release. The protected environment and all production secrets remain required because this dry run exercises the real production signing path.

## Published production files

The launch GitHub Release must contain:

- `Microgifter-HomeServer-v0.1.3-Setup.exe`
- `Microgifter-HomeServer-Setup.exe`
- `homeserver-stable.json`
- `SHA256SUMS.txt`

The stable manifest points to the unversioned installer within the immutable `v0.1.3` release URL. The versioned installer is the customer-facing download.

## Failure handling

A failed production workflow must not be bypassed by manually uploading files to a public release. Diagnose and repair the release branch, merge through the normal PR process, create a new tag only when appropriate, and rerun the complete production workflow.

If a GitHub Release was created with incorrect assets, remove the release from distribution, rotate compromised signing material when applicable, and publish a corrected version under a new semantic version. Do not silently replace an immutable release artifact.
