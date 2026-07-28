# Phase 6A — Microgifter Pairing, Entitlement and Update Client Foundation

Status: backend implementation and compiler validation in progress

## Baseline

- Repository: `bigriversocial74/homeserver`
- Base: merged `main` commit `e21b0efd2834df2ef398f46f7e9b1bb039ee9083`
- Architecture reference: PR #36, `docs/microgifter-homeserver-ownership-and-update-service.md`
- Implementation branch: `feature/microgifter-entitlement-update-client-v1-20260728`
- Dependency locks normalized against the permanent Windows Production Quality environment.

## Existing architecture audited

- `crates/homeserver-service/src/cloud_registry.rs` already owns the multi-provider connection registry, one-time pairing exchange, per-connection credentials, Ed25519 request signing, synchronization queues, receipts, and provider isolation.
- `database/migrations/0010_multi_cloud_connections.sql` is the current multi-connection persistence foundation.
- `crates/homeserver-service/src/update.rs`, `update_store.rs`, and `update_apply.rs` already own signed manifest verification, installer staging, Authenticode verification, backup, exact-version health checking, and rollback.
- `crates/machine-keyring` provides the protected machine-scoped credential boundary.
- `crates/homeserver-service/src/app.rs` composes secured loopback routers and service initialization.
- `src-tauri/src/cloud.rs`, `src-tauri/src/lib.rs`, `src/cloud-connections.js`, `src/cloud-connections.css`, and `index.html` provide the current Control Center connection UI.

## File-level implementation plan

### Service and persistence

1. Add `database/migrations/0014_microgifter_entitlement_update_client.sql`.
2. Add `crates/homeserver-service/src/microgifter_connection.rs` containing:
   - explicit Phase 6A connection states
   - capability registry
   - application error-code registry
   - provider adapter traits
   - versioned `/api/homeserver/v1/` client routes
   - Sync Code exchange and idempotent recovery
   - signed entitlement lease validation
   - entitlement refresh and offline/grace behavior
   - privacy-safe heartbeat/status payloads
   - credential rotation
   - device replacement and duplicate-device protection
   - durable local audit receipts
   - update eligibility and authorization policy
3. Extend `cloud_registry.rs` only where required to preserve compatibility and bridge existing connection records into the Phase 6A state machine.
4. Extend `update.rs` and `update_store.rs` with entitlement-aware update classes and authorization checks without weakening the existing cryptographic trust chain.
5. Register the migration, initialization, health checks, background refresh worker, and secured loopback routes in `app.rs` and `main.rs`.

### Control Center

6. Add Tauri commands for connection status, Sync Code pairing, entitlement refresh, credential rotation, update authorization, maintenance-window settings, and device replacement.
7. Extend the Integrations & Agents screen with a Microgifter connection panel that keeps local operation and unrelated providers visibly independent.
8. Expose only privacy-safe status fields; no Knowledge Vault content, prompts, conversations, local filenames, secrets, keys, or unrelated provider data.

### Contracts, fixtures, and validation

9. Add `docs/microgifter-cloud-connection-contract-v1.md` with exact routes, authentication, schemas, error codes, retry/idempotency rules, privacy requirements, and examples.
10. Add canonical capability, connection-state, and error-code registries.
11. Add deterministic mock-provider fixtures and an explicit non-production mock server.
12. Add focused Rust tests, contract validation, frontend validation, service smoke coverage, updater regression coverage, and permanent security-boundary checks.

## Permanent boundaries

- Microgifter is one provider/wrapper connection, not the HomeServer owner.
- Pairing, synchronization, entitlement, and signed updates remain separate systems.
- Pairing is never the update cryptographic trust root.
- Local operation, local data, local models, local agents, the Knowledge Vault, backups, and other wrappers remain available when Microgifter is offline, suspended, or revoked.
- Bootstrap, security, and recovery updates remain available without active paid entitlement.
- No second updater installer will be created.
- No production Microgifter endpoint secrets, DNS, CDN, object storage, billing, or PHP implementation are included.
- No merge without David Evans's explicit approval.
