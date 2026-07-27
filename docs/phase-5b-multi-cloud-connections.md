# Phase 5B — Multi-Cloud Connection Foundation

## Purpose

HomeServer is a local-first product that can operate without Microgifter. This phase replaces the one-cloud architectural assumption with a connection registry that can support multiple Microgifter sites and future CRM provider adapters.

Microgifter remains the first installed provider adapter. Other providers are rejected until an audited adapter is added explicitly.

## Connection boundary

Each connection has its own:

- HomeServer connection identity
- provider key and display name
- cloud base URL
- optional tenant and site identifiers
- cloud-issued device identity
- Ed25519 signing key and bearer credential
- Windows credential-vault entry
- granted scopes
- synchronization queue
- idempotency namespace
- synchronization receipts
- event history
- default, connection, degradation, revocation, and disconnection state

The one-time pairing token is sent only to the selected provider pairing endpoint. It is exchanged for a connection-specific device credential and is never stored.

## Supported configurations

- HomeServer with no cloud connection
- HomeServer paired to one Microgifter site
- HomeServer paired to multiple Microgifter sites
- HomeServer paired to Microgifter plus future CRM adapters
- HomeServer paired only to a future non-Microgifter CRM adapter

## Migration

`database/migrations/0010_multi_cloud_connections.sql`

The migration is additive. It creates:

- `cloud_connections`
- `cloud_sync_queue`
- `cloud_sync_receipts`
- `cloud_connection_events`

When an existing singleton Microgifter connection is present and the new registry is empty, startup migrates that connection into the registry. Its existing Windows credential-vault key is referenced without exposing or rewriting the secret. Existing queued operations and receipts are copied into the new connection-scoped tables.

## Provider adapter policy

The registry contains an explicit provider allowlist. Phase 5B installs only:

- `microgifter`

A provider adapter owns its pairing, status, synchronization, request signing, response-size, and receipt-validation contract. Arbitrary endpoints and arbitrary operation names are not accepted.

## Synchronization authority

The registry continues to allow only the existing bounded HomeServer v1 operation catalog:

- `device.heartbeat`
- `local.settings.snapshot`
- `cache.refresh.request`

Commerce, payments, claims, redemption, ownership, campaign, reward, and arbitrary mutation operations remain unavailable. Microgifter Cloud remains authoritative for cloud commerce data.

Every queue and receipt lookup is scoped by both `connection_id` and `idempotency_key`. An idempotency key used for one site cannot conflict with or authorize work for another site.

## Control Center

The Integrations & Agents page includes a Cloud Connection Registry panel with:

- local-only status
- connection count and aggregate pending work
- provider selection
- connection display name
- cloud URL
- one-time pairing token
- optional tenant and site identifiers
- default-connection selection
- per-connection status, scopes, pending work, and last synchronization
- per-connection synchronization and disconnection
- synchronize-all control

## Phase 5B supervised-agent relationship

All later approval-gated agent plans must bind their immutable plan hash to:

- provider key
- `connection_id`
- tenant and site identity
- requesting MCP client
- action and sanitized arguments
- fresh-state token
- approval expiration

Local-only actions such as backup creation and local-model tests do not require a cloud connection. Cloud actions must identify exactly one connection unless the fixed action is explicitly defined as an audited multi-connection operation.
