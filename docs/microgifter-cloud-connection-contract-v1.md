# Microgifter Cloud Connection Contract v1

Status: Phase 6A HomeServer client contract

This document defines the versioned HomeServer-to-Microgifter provider contract implemented by `crates/homeserver-service/src/microgifter_connection.rs`.

## Ownership and trust boundaries

- Microgifter is one provider connection. It does not own the HomeServer runtime, local data, local models, agents, Knowledge Vault, backups, or unrelated provider connections.
- Pairing, synchronization, entitlement, and software-update verification are separate systems.
- A pairing credential or entitlement lease is never the software-update cryptographic trust root.
- Signed update-manifest verification, installer SHA-256 verification, Authenticode verification, pre-update backup, exact-version health verification, and rollback remain mandatory.
- Bootstrap, security, and recovery updates remain available without an active paid entitlement.

## Provider base URL

Production provider URLs must use HTTPS and must not contain credentials, query parameters, fragments, or a path. Loopback HTTP is permitted only for non-production fixtures.

## Provider endpoints

All provider endpoints are relative to the paired provider base URL.

| Operation | Method | Path |
|---|---:|---|
| Sync Code exchange | POST | `/api/homeserver/v1/pairing/exchange` |
| Entitlement refresh | POST | `/api/homeserver/v1/entitlements/refresh` |
| Privacy-safe heartbeat | POST | `/api/homeserver/v1/devices/heartbeat` |
| Credential rotation | POST | `/api/homeserver/v1/devices/credentials/rotate` |
| Update authorization | POST | `/api/homeserver/v1/updates/authorize` |
| Update receipt submission | POST | `/api/homeserver/v1/updates/receipts` |
| Device replacement start | POST | `/api/homeserver/v1/devices/replacements/start` |
| Device replacement completion | POST | `/api/homeserver/v1/devices/replacements/complete` |

## Local Control Center endpoints

These endpoints are loopback-only and remain protected by the HomeServer local-client boundary.

| Operation | Method | Path |
|---|---:|---|
| Connection status | GET | `/v1/providers/microgifter/status` |
| Connect using Sync Code | POST | `/v1/providers/microgifter/connect` |
| Refresh entitlement | POST | `/v1/providers/microgifter/entitlement/refresh` |
| Send heartbeat | POST | `/v1/providers/microgifter/heartbeat` |
| Rotate credentials | POST | `/v1/providers/microgifter/credentials/rotate` |
| Read update preferences | GET | `/v1/providers/microgifter/update-preferences` |
| Save update preferences | POST | `/v1/providers/microgifter/update-preferences` |
| Authorize update | POST | `/v1/providers/microgifter/updates/authorize` |
| Start device replacement | POST | `/v1/providers/microgifter/device-replacement/start` |
| Complete device replacement | POST | `/v1/providers/microgifter/device-replacement/complete` |

Local request bodies are limited to 128 KiB. Provider responses are limited to 1 MiB.

## Pairing exchange

The Sync Code is exchanged once and is never retained locally.

Request fields:

```json
{
  "provider_key": "microgifter",
  "sync_code": "one-time-code",
  "request_id": "idempotent-request-id",
  "installation_id": "local-installation-id",
  "device_display_name": "Office HomeServer",
  "homeserver_version": "0.1.3",
  "device_public_key": "base64url-ed25519-public-key",
  "requested_capabilities": ["pairing.v1", "entitlement-lease.v1"],
  "merchant_id": null,
  "site_id": null,
  "replacement_id": null
}
```

The provider response must contain:

- Permanent provider connection identity
- Owner account identity
- UUID device identity
- Device bearer token of at least 32 characters
- Initial granted scopes/capabilities
- A provider entitlement-signing public key and key ID
- A signed entitlement lease

The HomeServer creates its Ed25519 device signing key locally and stores the private key and bearer token in the machine-scoped credential vault. The provider receives only the public key.

`request_id` is idempotent. Repeating a completed exchange with the same request ID returns the already-created local connection rather than creating a duplicate device.

## Signed provider requests

After pairing, HomeServer requests use:

- `Authorization: Bearer <device-token>`
- `X-MG-Homeserver-ID: <device-id>`
- `X-MG-Connection-ID: <provider-connection-id>`
- `X-MG-Timestamp: <unix-seconds>`
- `X-MG-Nonce: <unique-value>`
- `X-MG-Signature: <base64url-ed25519-signature>`
- `X-MG-Homeserver-Version: <semantic-version>`

The canonical signature input is:

```text
METHOD\nPATH\nTIMESTAMP\nNONCE\nSHA256_HEX(BODY)
```

The provider must reject expired timestamps, reused nonces, invalid bearer tokens, mismatched device or connection identities, and invalid signatures.

## Provider response envelope

Successful and unsuccessful responses use the same bounded JSON envelope:

```json
{
  "ok": true,
  "message": "Human-readable summary",
  "data": {}
}
```

For an unsuccessful response, `ok` is `false`; the HTTP status and message are mapped to a stable HomeServer application error category. Secrets must never be returned in the message.

## Entitlement lease

The entitlement lease is an Ed25519-signed JSON payload. Its signing key is separate from the software-update manifest key.

Required claims:

```json
{
  "schema_version": 1,
  "lease_id": "lease-id",
  "provider_id": "microgifter",
  "account_id": "account-id",
  "connection_id": "provider-connection-id",
  "device_id": "uuid-device-id",
  "issued_at_utc": "2026-07-28T16:00:00Z",
  "not_before_utc": "2026-07-28T16:00:00Z",
  "expires_at_utc": "2026-07-29T16:00:00Z",
  "subscription_state": "active",
  "granted_capabilities": [],
  "denied_capabilities": [],
  "merchant_scope": [],
  "site_scope": [],
  "device_allowance": {},
  "update_eligibility": true,
  "allowed_update_channels": ["stable"],
  "minimum_homeserver_version": null,
  "signing_key_id": "provider-key-id"
}
```

The client verifies:

- Known, active provider signing key
- Valid Ed25519 signature over the serialized payload
- Supported schema and provider identity
- Matching provider connection and device identities
- Valid issue, not-before, and expiration times
- Supported capability identifiers
- Supported update channels
- Minimum HomeServer version, when present

A rejected lease is stored only as a bounded audit result. It does not overwrite the last accepted lease.

## Connection lifecycle

The Phase 6A lifecycle states are:

```text
unpaired
pairing_pending
active
offline
grace
suspended
revoked
replacing
error
```

Cloud unavailability moves an otherwise valid connection to `offline`; it does not disable local operation. An accepted grace lease maps the connection to `grace`. Suspended or canceled subscription state maps to `suspended`. Credential rejection or explicit provider revocation maps to `revoked`.

## Capability registry

Phase 6A recognizes these versioned capabilities:

```text
pairing.v1
device-registration.v1
device-heartbeat.v1
entitlement-lease.v1
credential-rotation.v1
merchant-assignments.v1
site-assignments.v1
dataset-grants.v1
sync.incremental.v1
operational-data.v1
campaign-actions.v1
signed-updates.v1
update-authorization.v1
update-receipts.v1
device-replacement.v1
```

Unknown capabilities are not silently activated. Effective access is calculated from client support, account grants, device grants, the current signed lease, and server availability.

## Heartbeat privacy boundary

Heartbeat data may contain only operational connection metadata such as:

- Installation and device identifiers
- Provider connection identifier
- HomeServer version
- Connection lifecycle state
- Capability registry version
- Counts and health categories
- Last synchronization or entitlement timestamps

It must not contain Knowledge Vault content, document text, filenames, prompts, conversations, model inputs or outputs, local filesystem paths, tokens, private keys, signing keys, recovery secrets, or data belonging to another provider connection.

## Credential rotation

Credential rotation requires a valid signed request using the current credential. The provider returns a replacement bearer token. The HomeServer writes the replacement to the machine credential vault before acknowledging success. A failed vault write leaves the previous credential intact.

## Device replacement and duplicate protection

A machine fingerprint is represented only by a SHA-256-derived observation value; raw hardware identifiers are not transmitted or stored.

The provider replacement flow is explicit:

1. Start replacement from the existing connection.
2. Obtain a replacement ID.
3. Pair the new installation using a Sync Code and the replacement ID.
4. Review assignments and grants.
5. Complete replacement.
6. Revoke or retire the old device credential.

A restored database presenting a device identity on a different installation is not silently trusted. It is recorded as a duplicate, stale restore, replacement-pending, or rejected observation.

## Update policy

Update classes are:

```text
bootstrap
security
maintenance
feature
preview
recovery
```

Bootstrap, security, and recovery updates do not require paid-provider authorization. Maintenance, feature, and preview updates may require an active eligible entitlement and an unexpired provider authorization.

Provider authorization never bypasses:

- Signed update-manifest verification
- Pinned update-signing key verification
- Installer size and SHA-256 validation
- Authenticode signer verification
- Pre-update encrypted backup
- Exact-version post-install health check
- Automatic rollback

Supported local install modes are:

```text
install_now
when_idle
tonight
maintenance_window
defer_until
```

Maintenance windows are stored as UTC start-minute and duration values. Deferred installations cannot begin before the configured UTC timestamp.

## Update receipts

HomeServer records local receipts before attempting provider delivery. Receipt delivery is retryable and does not change the cryptographic validity or local result of an update.

A receipt contains bounded operational metadata only: update ID, target version, terminal state, optional stable failure code, device and provider connection identifiers, and completion time.

## Stable application errors

Phase 6A defines these client error identifiers:

```text
microgifter_sync_code_invalid
microgifter_sync_code_expired
microgifter_sync_code_used
microgifter_pairing_interrupted
microgifter_connection_not_found
microgifter_connection_inactive
microgifter_entitlement_missing
microgifter_entitlement_signature_invalid
microgifter_entitlement_key_unknown
microgifter_entitlement_expired
microgifter_entitlement_device_mismatch
microgifter_entitlement_connection_mismatch
microgifter_capability_unsupported
microgifter_cloud_offline
microgifter_credentials_rejected
microgifter_credential_rotation_failed
microgifter_update_not_entitled
microgifter_update_authorization_expired
microgifter_update_deferred
microgifter_duplicate_device_identity
microgifter_device_replacement_required
```

Provider implementations may include a more specific bounded reason code in the response envelope, but clients must continue to handle the stable categories above.

## Retry and idempotency rules

- Pairing exchange is idempotent by `request_id`.
- Heartbeats may be retried with a new nonce and the same logical heartbeat identity.
- Entitlement refresh replaces only a successfully verified lease.
- Credential rotation must not be retried after an ambiguous provider success without first checking status.
- Update authorization is idempotent by connection and update ID.
- Update receipts are idempotent by local receipt ID.
- Replacement completion is idempotent by replacement ID.

Transient failures use bounded backoff. Authentication, signature, device-mismatch, revoked, and unsupported-contract failures are not treated as transient.

## Retention

- Completed, failed, or expired pairing attempts are retained for up to 30 days.
- Provider connection receipts and device identity observations are retained for up to 365 days, subject to bounded row-count retention.
- Device credentials and private signing keys remain outside SQLite in the protected machine credential vault.

## Phase 6B server implementation requirement

The coordinated Microgifter server implementation must match this v1 contract exactly before production pairing is enabled. Production DNS, CDN, object storage, subscription billing, PHP endpoints, release publishing, and signing-key operations are Phase 6B responsibilities and are not embedded in the HomeServer client.
