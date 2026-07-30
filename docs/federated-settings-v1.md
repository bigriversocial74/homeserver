# VP3/HomeServer Federated Settings v1

## Purpose

VP3 and HomeServer expose the same setting names and categories without creating two competing authorities or copying private HomeServer state into the cloud.

## Authority model

Every catalog entry declares exactly one authority:

- `vp3` — cloud, licensing, subscription, and account policy. HomeServer receives a read-only local mirror.
- `homeserver` — private local behavior. VP3 may show descriptive metadata but cannot overwrite the local value.
- `shared` — a non-secret preference that either surface may update through optimistic revisions.

There is no last-write-wins fallback. A stale revision produces a conflict receipt, and an unsynchronized HomeServer value remains locally dirty until acknowledged or explicitly resolved.

## Data boundary

Only cataloged `non_secret` values may synchronize. The catalog must never contain:

- passwords or API keys;
- Stripe or provider credentials;
- private keys or enrollment credentials;
- files, prompts, conversations, models, or MCP content;
- local execution payloads or customer application data.

The HomeServer device credential remains in the operating-system credential vault and is used only as bearer authentication for the outbound settings request.

## Signed snapshot

VP3 returns a short-lived Ed25519-signed document using the dedicated HomeServer lease key. The document binds:

- schema version;
- account and device identity;
- maximum cloud revision;
- deterministic snapshot hash;
- complete non-secret setting list;
- issue and expiration timestamps.

HomeServer verifies the pinned key ID, Ed25519 signature, document hash, lifetime, account/device identity, and exact wrapper/document equality before any merge.

## Merge rules

1. VP3-owned values replace the local read-only mirror after signature verification.
2. HomeServer-owned values are written locally and pushed only from the activated device.
3. Shared values use the last acknowledged cloud revision as `expected_revision`.
4. An acknowledged update clears the local dirty flag.
5. A conflict preserves the local value and records the current cloud revision and conflict reason.
6. Replayed request IDs return the current signed snapshot without reapplying writes.

## Local endpoints

- `GET /v1/federated-settings`
- `POST /v1/federated-settings/update`
- `POST /v1/federated-settings/sync`

The Tauri bridge exposes matching commands to the Control Center Settings interface.

## Certification boundary

Phase 15 is mergeable only after the exact PR head passes the permanent Phase 15 contract plus complete HomeServer Production Quality, including native builds, workspace tests and lint, API smoke tests, NSIS installation, LocalSystem security, signed update, and forced rollback.
