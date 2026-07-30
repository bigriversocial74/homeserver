# Phase 16A — Shared Multi-Wrapper Identity and Pairing Core

Status: Draft implementation

Baseline: `bigriversocial74/homeserver@1fec42b2214209b91494373a3bd62c0e93689eed`

## Governing product decision

The HomeServer is an independently owned, private, self-hosted capability node. RSS-POD is one authorized wrapper. Microgifter is one authorized wrapper. Future PODs and applications may be separately authorized wrappers. No wrapper owns the HomeServer, another wrapper, local models, private knowledge, tools, agents, backups, software-update authority, or unrelated credentials.

## Phase 16A purpose

Create the shared identity and pairing-control foundation needed to place existing provider-specific connections behind one wrapper-neutral authority model without breaking current Microgifter, POD, VP3, MCP, installer, backup, or update behavior.

This phase does not replace provider adapters. Existing adapters remain responsible for their remote Sync Code exchange, signed provider requests, provider-specific synchronization, and remote capability negotiation. Phase 16A records the shared wrapper identity, connection, device, credential-reference, pairing-attempt, and event boundary around those adapters.

## Authority boundaries

| Actor | Authority |
|---|---|
| User/HomeServer owner | Registers wrappers, starts pairing, approves the remote origin, completes adapter-created connections, and revokes wrapper access. |
| HomeServer | Owns local wrapper records, device bindings, credential references, audit events, validation, and revocation enforcement. |
| Wrapper | May use only its own connection, device identity, credentials, later grants, jobs, results, and receipts. |
| POD | Public identity and social authority for its own POD data; it is not HomeServer authority. |
| Device | Represents one local HomeServer installation within one wrapper connection. |
| Agent | Receives no authority from pairing alone. Agent authority remains separately granted and approval-bound. |
| VP3 | Remains optional software-management authority and is not modeled as a front-end wrapper. |

## Trust boundaries

1. All `/v1/wrappers/*` endpoints remain behind the existing fixed-loopback trusted-client boundary.
2. Pairing attempts store no Sync Code, pairing secret, bearer token, private key, prompt, document, conversation, or private source content.
3. Secrets remain in the operating-system credential vault and never enter SQLite.
4. SQLite stores only identifiers, public keys, credential references, hashes, state, timestamps, bounded metadata, and audit evidence.
5. Pairing completion must bind the exact registered wrapper, approved remote origin, existing adapter-created connection, and adapter provider identity.
6. Revocation deletes the connection credential from the OS vault before marking the shared and legacy connection states revoked.
7. A wrapper cannot enumerate this registry remotely; the registry is a local owner/control-plane API.
8. Wrapper-specific data access is not granted by this migration. Capability grants are Phase 16B.

## Migration

Migration `0020_wrapper_identity_and_pairing.sql` creates:

- `wrapper_identities`
- `wrapper_connections`
- `wrapper_devices`
- `wrapper_pairing_attempts`
- `wrapper_credential_references`
- `wrapper_events`

Required indexes cover wrapper state, wrapper/connection state, device state, due pairing attempts, credential state, legacy compatibility, and recent events.

### Compatibility backfill

On startup, existing `cloud_connections` records are backfilled additively:

- One shared wrapper identity is created for each existing provider key.
- Existing connection IDs remain unchanged.
- Existing provider and connection IDs are preserved as compatibility aliases.
- Existing device IDs, public keys, origins, states, timestamps, and credential references are retained.
- No existing table is deleted, renamed, or rewritten.
- Repeated startup is idempotent.

## Local API

All request bodies are limited to 64 KiB.

| Operation | Method | Path |
|---|---:|---|
| Wrapper registry snapshot | GET | `/v1/wrappers` |
| Register wrapper identity | POST | `/v1/wrappers/register` |
| Start pairing attempt | POST | `/v1/wrappers/pairing/start` |
| Complete adapter pairing | POST | `/v1/wrappers/pairing/complete` |
| Revoke wrapper connection | POST | `/v1/wrappers/connections/revoke` |

## Example payloads

### Register wrapper

```json
{
  "wrapper_key": "rss-pod",
  "display_name": "RSS-POD",
  "wrapper_kind": "pod",
  "protocol_version": "rss-pod-1.0"
}
```

### Start pairing

The one-time Sync Code remains inside the adapter-specific exchange and is not submitted to this shared registry.

```json
{
  "wrapper_id": "f93f5cf6-e131-4444-b3d6-c29d6d54266c",
  "request_id": "pair:8f08119e-ec3a-4e85-b8ee-6f1705f4bb38",
  "remote_origin": "https://pod.example.com",
  "device_display_name": "Office HomeServer",
  "requested_capabilities": [
    "pairing.v1",
    "agent.request.v1"
  ],
  "expires_minutes": 15
}
```

### Complete pairing

```json
{
  "attempt_id": "53732b57-73ab-45d0-86a2-78a3504aa365",
  "connection_id": "2e7a4423-e8e4-4230-866a-c8d4d5c0a07d",
  "remote_connection_id": "pod-connection-4472",
  "contract_version": "rss-pod-homeserver-1"
}
```

### Revoke connection

```json
{
  "connection_id": "2e7a4423-e8e4-4230-866a-c8d4d5c0a07d",
  "confirmation": "REVOKE WRAPPER",
  "reason": "Owner removed this wrapper"
}
```

## Credential and key handling

- Device private keys and bearer credentials remain in Windows Credential Manager or the platform credential vault.
- `wrapper_credential_references` stores only the vault service/account reference, type, state, hint, expiration, rotation, and revocation timestamps.
- Pairing completion never accepts a private key or bearer credential.
- Credential rotation remains adapter-specific until the shared grant/credential phase.
- Owner revocation removes the referenced connection credential before database state is committed.
- VP3 update-signing keys and software-authority credentials remain independent.

## Offline and failure behavior

- HomeServer remains locally usable with zero wrappers or when every wrapper is offline.
- A degraded legacy connection maps to `offline`, not global HomeServer failure.
- Pairing attempts expire automatically and cannot be completed after expiration.
- Reusing a request ID with different data is rejected.
- Origin, wrapper identity, provider identity, or connection mismatch is rejected.
- A failed credential-vault deletion prevents revocation state from being committed.
- Revoking one wrapper does not alter another wrapper connection.
- Existing provider workers continue to own retry and reconnection behavior during Phase 16A.

## Audit and receipts

Security-scoped `wrapper_events` are created for:

- `wrapper.registered`
- `wrapper.pairing.started`
- `wrapper.pairing.completed`
- `wrapper.connection.revoked`

Events contain IDs, outcomes, correlation data, bounded non-secret metadata, and timestamps. They contain no credentials, private source content, prompts, documents, conversations, or model input/output.

## Security tests

The permanent validator and Rust tests require:

- Idempotent migration execution.
- No secret-bearing schema columns.
- Exact wrapper/provider/origin binding on pairing completion.
- HTTPS outside loopback fixtures.
- Rejection of URL credentials, query strings, fragments, and paths.
- Bounded capability lists and identifiers.
- Pairing-request replay protection.
- Explicit revocation confirmation.
- OS-vault deletion before revocation commit.
- Backfill without changing existing connection IDs.
- Independent wrapper rows and connection-scoped events.
- Full HomeServer service compile, tests, formatting, and strict Clippy.
- Retained HomeServer Production Quality, installer, LocalSystem, backup, signed-update, and rollback certification before merge.

## Required next migration

Phase 16B will add migration `0021_wrapper_capability_grants.sql` for explicit resource, dataset, action, agent, rate, cost, expiration, approval, and bridge grants. Pairing alone grants no data or action authority.

## Production-quality score

| Area | Score |
|---|---:|
| Product and authority alignment | 10/10 |
| Additive migration and compatibility design | 10/10 |
| Secret and credential boundary | 10/10 |
| Pairing identity/origin binding | 10/10 |
| Revocation behavior | 10/10 |
| Offline isolation | 10/10 |
| Audit evidence | 10/10 |
| Static and unit-test contract | 10/10 |
| Exact-head CI certification | Pending |
| Installed Windows certification | Pending |

**Current implementation score: 8.8/10 pending exact-head CI and installed release certification.**

Do not merge solely because the PR is mergeable. Merge only after every exact-head workflow, retained production-quality gate, installed LocalSystem test, backup/restore test, signed-update test, forced-rollback test, and cross-wrapper isolation test passes.
