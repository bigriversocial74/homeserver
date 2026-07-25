# HomeServer Signed Update Contract

## Purpose

Phase 3B delivers a Windows update path that does not trust a URL, filename, hash, signer, or target version supplied by the Control Center. Release metadata must be signed by the pinned Microgifter Ed25519 release key before HomeServer will download an installer.

## Trust chain

1. HomeServer requests the configured stable manifest over HTTPS.
2. Redirects are limited and may not downgrade to HTTP.
3. The manifest `key_id` must match the compiled release-key identity.
4. The Ed25519 signature is verified over the compact UTF-8 JSON serialization of the `payload` object in the Rust field order defined by `UpdateManifestPayload`.
5. Product, schema, channel, version, publication time, release-note size, installer URL, filename, size, SHA-256, and Authenticode thumbprint are validated.
6. The installer is streamed into managed staging storage with a strict byte limit.
7. The final byte count and SHA-256 must exactly match the signed payload.
8. Windows Authenticode status must be `Valid`, and the signer thumbprint must match the signed payload.
9. HomeServer creates a DPAPI-protected pre-update database backup.
10. A copied updater helper outside the installation directory stops the service, snapshots the installed binary tree, runs the installer silently, and verifies loopback health and the target version.
11. Failed installation or health verification restores the previous binary tree and restarts the prior service version.

## Manifest envelope

```json
{
  "key_id": "homeserver-release-2026-01",
  "payload": {
    "schema_version": 1,
    "product": "Microgifter HomeServer",
    "channel": "stable",
    "version": "0.2.0",
    "minimum_version": "0.1.0",
    "published_at_utc": "2026-07-24T23:00:00Z",
    "release_notes": "Release notes",
    "installer": {
      "url": "https://updates.microgifter.com/homeserver/stable/Microgifter-HomeServer-Setup.exe",
      "file_name": "Microgifter-HomeServer-Setup.exe",
      "size_bytes": 12345678,
      "sha256": "64 lowercase hexadecimal characters",
      "authenticode_thumbprint": "trusted Windows signer certificate thumbprint"
    }
  },
  "signature": "base64url Ed25519 signature without padding"
}
```

## Key custody

The repository contains only a public verification key. A production release private key must never be committed, placed in an installer, included in workflow artifacts, or stored on the HomeServer. It should be generated and held in an offline signing system or hardware-backed key service with access logging and dual-control release approval.

The current repository default is a non-secret verification anchor for development builds. Production release builds must provision the organization-controlled public key at build time and retain the corresponding private key outside GitHub source control.

## Authenticode

The release manifest signs the expected Authenticode certificate thumbprint. This allows certificate rotation without weakening the manifest trust anchor while still preventing a validly signed installer from an unrelated publisher from being accepted.

The Windows CI test creates a short-lived self-signed code-signing certificate, trusts it only on the ephemeral runner, signs a copied installer, exercises successful installation and forced rollback, and removes the certificate afterward. The uploaded development installer is not replaced by the CI-signed test copy.

## Rollback and audit

The updater helper records one of:

- `succeeded`
- `rolled_back`
- `failed`

The next service start consumes the result file, updates SQLite state, and adds an update event. A rollback is treated as a completed safety action, not a successful release installation.

## Explicit boundaries

- The local API remains loopback-only.
- The UI cannot provide a manifest URL, signing key, installer URL, installer path, hash, or signer thumbprint.
- Update delivery grants no Microgifter cloud commerce authority.
- Database restore and binary rollback remain separate mechanisms.
- A signed manifest does not bypass Authenticode, SHA-256, size, version, path, or health checks.
