# Phase 16B — Wrapper Capability Grants

## Status

- Baseline: `main@c7782e9cdda832cf42152eb3aaf49178736b1cc4`
- Branch: `feature/phase-16b-wrapper-capability-grants`
- Initial score: 4.6/10
- Final target: 10/10
- Scope: capability grants, exact scopes, approval, limits, revocation, authorization receipts, and cross-wrapper bridges
- Explicit exclusion: shared wrapper jobs and execution receipts remain Phase 16C

## Initial score: 4.6/10

Phase 16A provided production-ready wrapper identities, paired connections, devices, expiring pairing attempts, credential references, and security events. It did not yet provide a shared capability catalog or a runtime authority evaluator.

| Area | Initial score | Gap |
|---|---:|---|
| Wrapper identity binding | 10/10 | Complete in Phase 16A |
| Pairing grants zero authority | 10/10 | Complete in Phase 16A |
| Capability vocabulary | 2/10 | Provider-specific capability strings existed without one catalog |
| Grant lifecycle | 1/10 | No shared create, approval, rotation, expiration, or revocation model |
| Dataset/resource scopes | 1/10 | No common exact-scope enforcement |
| Resource limits | 2/10 | Limits existed in isolated runtimes, not wrapper grants |
| Sensitive action approval | 5/10 | Agent approvals existed but were not wrapper-grant authority |
| Cross-wrapper isolation | 7/10 | Connection isolation existed; no explicit bridge contract |
| Authorization receipts | 3/10 | Provider receipts existed, but no shared authorization decision receipt |
| Recovery and revocation fencing | 5/10 | Backups existed; queued work lacked a common grant revision fence |

Weighted baseline: **4.6/10**.

## Fix outline

1. Create one bounded capability catalog with no administrative or wildcard capabilities.
2. Bind every grant to one wrapper identity and one wrapper connection.
3. Require exact operations, exact scopes, mandatory expiration, and bounded limits.
4. Require explicit approval for high-risk grants and per-request approval for critical actions.
5. Advance a connection authority revision whenever active authority changes.
6. Record revocation fences so cached or queued work can fail closed.
7. Return safe authorization decisions and store bounded allowed/denied receipts.
8. Make cross-wrapper access impossible without a separate, expiring bridge grant.
9. Require explicit approval for every bridge and prevent same-wrapper bridge records.
10. Preserve existing Microgifter, RSS-POD, VP3, MCP, backup, installer, and updater behavior.

## Governing authority

### HomeServer authority

HomeServer is the private capability and execution authority. It owns grant evaluation, private-data filtering, resource limits, approval enforcement, authorization receipts, and revocation fences.

### Wrapper authority

A wrapper may identify itself through its paired connection and request a capability. It cannot create authority for itself, inspect another wrapper, weaken approval policy, expand its scope, or convert a safe result into source data.

### User authority

The user issues, approves, rejects, rotates, and revokes grants. Sensitive use requires a fresh plan-hash-bound approval.

### Device authority

A paired device proves possession of its connection credential. Device identity does not create data, tool, model, agent, or action authority.

### Agent authority

An agent receives no independent standing authority. Phase 16C jobs will carry the exact grant ID, grant revision, approval binding, scope, limits, and correlation identifiers evaluated here.

## Pairing grants zero authority

A successful wrapper pairing creates identity and connection records only. Migration `0021` does not backfill any capability grant. The first authorization request therefore fails closed with `grant_missing` until a user-issued grant becomes active.

The grant registry exposes `pairing_implies_authority: false` as an explicit contract value.

## Capability catalog

The initial catalog is deliberately narrow:

- `wrapper.status.read`
- `settings.read`
- `settings.update`
- `knowledge.search`
- `knowledge.result.read`
- `model.inference.request`
- `agent.job.propose`
- `agent.job.read`
- `action.propose`
- `receipt.read`

Forbidden examples include `admin`, `knowledge.all`, `tools.all`, `agent.execute_any`, `cross_wrapper.read`, and any capability ending in `.all`.

Each catalog entry defines:

- Risk tier
- Minimum approval mode
- Result mode
- Whether exact scope is required
- Allowed operations
- Active/deprecated/disabled state

## Grant lifecycle

States:

`pending_approval → active → expired | revoked | superseded`

Additional state:

`suspended`

Rules:

- Every grant has `not_before_utc` and `expires_at_utc`.
- Expiration is mandatory and risk-tier bounded.
- A requested approval mode may strengthen but never weaken the catalog default.
- Rotation creates a new revision and links old/new grants.
- The old active grant remains authoritative while a sensitive rotation is pending.
- Approval supersedes the old grant atomically before activating the replacement.
- Revocation immediately revokes scopes and approvals and advances the connection authority revision.

## Exact scope policy

Scopes are exact tuples:

`scope_kind + scope_value`

Allowed kinds:

- dataset
- collection
- record
- tag
- resource

The values `*`, `all`, `any`, and `everything` are forbidden. Allowed field lists cannot contain wildcards. Filters must be JSON objects. Scope matching is exact; there is no prefix, implicit inheritance, or global fallback.

Result policies:

- `safe_result`
- `metadata_only`
- `aggregate_only`
- `proposal_only`
- `receipt_only`

Private source documents, prompts, credentials, raw memory, and unrestricted model context remain outside all wrapper responses.

## Approval policy

Approval modes:

- `none`: allowed only when the catalog permits it
- `explicit`: one approval activates the bounded grant
- `per_request`: each sensitive use requires a fresh approved plan hash

High-risk capabilities cannot request `none`. Critical capabilities default to `per_request`.

Approval records expire and are single-use when action-specific. Approvals bind to:

- Grant or bridge ID
- Action
- Plan hash
- Requesting user
- Deciding user
- Expiration
- Decision and consumption timestamps

## Resource limits

Every grant receives bounded limits:

- Requests per minute
- Maximum safe-result bytes
- Daily model tokens
- Concurrent jobs
- Queued jobs
- Execution seconds

Requested limits may only reduce the risk-tier ceiling. Authorization updates minute and day usage windows transactionally.

## Cross-wrapper bridge policy

Wrapper A cannot use Wrapper B’s grants. A bridge is a separate record bound to:

- Source wrapper and connection
- Target wrapper and connection
- Capability and operations
- Exact scope
- Result policy
- Approval mode
- Mandatory expiration

All bridges begin `pending_approval`. Same-wrapper bridges are rejected by both Rust validation and a database `CHECK` constraint. A bridge does not copy the target wrapper’s grant or expose its private records; it authorizes one narrowly defined inter-wrapper result path.

## Revocation and queued-work fences

Every active-authority mutation increments `wrapper_connections.grant_revision` and updates `wrapper_grant_revocation_fences`.

Phase 16C jobs must store the evaluated revision. Before execution and before result delivery, the worker must compare the stored revision to the current connection revision and fail closed if it is stale.

This invalidates:

- Cached authorizations
- Pending jobs
- Queued jobs
- Delayed result deliveries
- Replayed approvals
- Stale bridge decisions

## Authorization decisions and receipts

Allowed and denied decisions record:

- Decision ID
- Wrapper and connection
- Grant or bridge ID
- Capability and operation
- Outcome and detail code
- Grant revision
- Scope hash, never raw private data
- Result policy
- Correlation ID
- Timestamp

The response returns only safe authority metadata. It does not return grant scope filters, private source content, credentials, prompts, or hidden policy data.

## Offline and failure behavior

- Existing active grants remain locally evaluable while a wrapper is offline.
- New cloud approval is not assumed; owner approval is local.
- Expired grants and approvals fail closed.
- Missing catalog, limits, scope, connection, approval, or revision data fails closed.
- Database transaction failure does not partially consume an approval or partially increment usage.
- Backup and restore preserve grants, approvals, limits, revisions, fences, events, and receipts as part of the HomeServer SQLite database.
- Restored stale grants are expired during startup initialization before authorization is available.

## Endpoints

Local Control Center management:

- `GET /v1/wrapper-grants`
- `POST /v1/wrapper-grants/create`
- `POST /v1/wrapper-grants/rotate`
- `POST /v1/wrapper-grants/revoke`
- `POST /v1/wrapper-grants/approvals/request`
- `POST /v1/wrapper-grants/approvals/decide`
- `POST /v1/wrapper-bridges/create`
- `POST /v1/wrapper-bridges/revoke`

Authority evaluation:

- `POST /v1/wrapper-grants/authorize`
- `POST /v1/wrapper-bridges/authorize`

The service remains loopback-bound and protected by the existing HomeServer local API security middleware. Phase 16C will invoke the evaluator internally before accepting or executing wrapper jobs.

## Database migration

Migration:

`database/migrations/0021_wrapper_capability_grants.sql`

Tables:

- `wrapper_capability_catalog`
- `wrapper_capability_grants`
- `wrapper_dataset_scopes`
- `wrapper_resource_limits`
- `wrapper_bridge_grants`
- `wrapper_grant_approvals`
- `wrapper_grant_usage_windows`
- `wrapper_grant_revocation_fences`
- `wrapper_grant_events`
- `wrapper_authorization_receipts`

No manual SQL import is required. HomeServer applies the additive migration at service startup.

## Security test matrix

| Test | Required result |
|---|---|
| Pair wrapper without a grant | Authorization denied with `grant_missing` |
| Wrapper A uses Wrapper B connection | Connection/wrapper binding rejected |
| Request wildcard capability | Rejected |
| Request wildcard scope | Rejected |
| Weaken approval mode | Rejected |
| Use expired grant | Denied |
| Use revoked grant | Denied |
| Reuse consumed per-request approval | Denied |
| Exceed request rate | Denied |
| Exceed result-size limit | Denied |
| Exceed daily token limit | Denied |
| Create same-wrapper bridge | Rejected |
| Use bridge before approval | Denied |
| Use bridge with different scope | Denied |
| Revoke one wrapper grant | Other wrappers remain active |
| Restore database with stale grant | Grant expires before API readiness |
| Inspect receipt | Contains hashes and safe metadata only |
| Search response | Contains filtered result only, never source documents |

## 10/10 exit standard

Phase 16B is 10/10 only when:

1. Pairing creates zero implicit capabilities.
2. All grants are connection-bound, scoped, expiring, limited, and auditable.
3. Approval policy cannot be weakened.
4. Critical use requires a fresh plan-hash-bound approval.
5. Revocation advances a durable authority fence.
6. Allowed and denied requests produce safe receipts.
7. Cross-wrapper access fails without a separate approved bridge.
8. No source data or secrets appear in grant payloads or receipts.
9. Unit tests, migration validator, formatting, compilation, and strict Clippy pass.
10. Retained Phase 13–16A and full Production Quality installer, LocalSystem, backup, signed-update, and rollback certification pass.

## Final target: 10/10

The implementation may be called 10/10 only after the exact PR head satisfies every exit requirement above. Until then, its score remains provisional.
