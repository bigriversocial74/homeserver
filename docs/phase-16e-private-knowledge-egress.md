# Phase 16E — Private Knowledge Access and Result-Egress Enforcement

## Status and score

Initial current-state score: **7.1/10**.

Phase 16C already separated private inputs/results from wrapper-safe projections, and Phase 16D bound jobs to agents, assignments, grants, policies, approvals, and emergency stops. Phase 16E adds the missing resource authority, selector lifecycle, destination-specific references, egress decision evidence, review state, revocation propagation, and deletion cleanup.

## Authority model

Private knowledge is owned and administered by the HomeServer user. A paired wrapper receives no knowledge authority by pairing, agent assignment, or job creation alone. A private-knowledge job requires all of the following to remain current:

1. Active wrapper identity and connection.
2. Active capability grant and exact grant revision.
3. Explicit private-resource selector bound to the same wrapper connection and grant.
4. Exact resource revisions and classification revisions.
5. Exact purpose hash and output schema.
6. Exact agent revision when the selector is agent-bound.
7. Approved remote-model provider when remote context is allowed.
8. Current job, approval, and action authority from Phases 16C and 16D.

## Data classifications

The canonical classes are:

- `secret`
- `private_source`
- `private_derived`
- `private_selector`
- `shared_approved`
- `wrapper_owned`
- `public`
- `safe_receipt`
- `security_metadata`

Raw Knowledge Vault documents are classified `private_source` by default. Unknown or missing classifications fail closed.

## Private selectors

A selector is connection-specific and contains explicit resources, operations, purpose, output schema, limits, citation policy, remote-model policy, approval mode, expiration, and captured revisions. Selectors cannot enumerate another wrapper, cannot use wildcard resources, and cannot survive resource revision, classification revision, grant revision, agent revision, expiration, suspension, or revocation changes.

## Local-only knowledge access

Knowledge search is performed only through the internal worker endpoint while a Phase 16C lease is active. The worker must present the exact worker ID, job ID, and lease token. Search hits are reduced to selector-authorized resources before the private result is returned to the local worker. The access receipt stores query/result hashes and counts, never the query text, source text, filenames, or local paths.

## Result-egress pipeline

Every selector-bound result passes through:

1. Existing Phase 16C field allowlist projection.
2. Phase 16E nested private-field removal.
3. Credential and bearer-material scan.
4. cross-wrapper sentinel scan.
5. local filename and path removal.
6. local resource-ID replacement with a connection-specific opaque alias.
7. source-revision and classification-revision validation.
8. result-size and schema enforcement.
9. safe provenance generation.
10. persisted egress decision, redaction hashes, private evidence hash, projection, cache record, and incident evidence when denied.

Private source text, local filenames, local paths, credentials, prompts, embeddings, conversations, raw messages, and cross-wrapper sentinels never enter wrapper snapshots or deliveries.

## Egress approval

Selectors support `preauthorized` and `per_result` modes. A per-result projection remains `pending_review`, is hidden from wrapper snapshots, and is blocked from delivery until a fresh approval is bound to the exact output hash. Rejection revokes the projection and expires its delivery.

## Revocation and deletion

Selector revocation immediately:

- increments the selector revision;
- invalidates active projections and caches;
- revokes pending egress decisions;
- expires pending deliveries;
- causes active Phase 16C jobs to fail closed during authority reconciliation.

Knowledge Vault updates invalidate resource-bound projections and caches. Deletion creates a propagation job that suspends selectors, revokes aliases, removes selector-resource bindings, invalidates projections/caches, and prevents future delivery.

## API surface

Local Control Center APIs:

- `GET /v1/privacy`
- `POST /v1/privacy/connection-snapshot`
- `GET /v1/privacy/data-classes`
- `GET /v1/privacy/resources`
- `POST /v1/privacy/resources/classify`
- `GET|POST /v1/privacy/selectors`
- `POST /v1/privacy/selectors/revoke`
- `GET /v1/privacy/egress-decisions`
- `POST /v1/privacy/egress-decisions/review`
- `POST /v1/privacy/cache/purge`
- `GET /v1/privacy/incidents`
- `POST /v1/internal/privacy/search`

## 10/10 certification gates

1. Pairing grants no private-resource authority.
2. Knowledge jobs fail without an exact active selector.
3. Wrapper A cannot use Wrapper B selectors, aliases, projections, decisions, or cache entries.
4. Resource or classification revision changes cancel queued work and invalidate projections.
5. Raw source documents, filenames, local paths, prompts, embeddings, credentials, and conversations never egress.
6. Cross-wrapper sentinels and credential material deny egress and create local incidents.
7. Citations use destination-specific opaque aliases only.
8. Per-result approval is bound to the exact output hash and blocks delivery before approval.
9. Selector revocation and source deletion invalidate pending delivery and cached projections.
10. Exact-head contract, native tests, strict lint, installer, LocalSystem, backup, signed-update, and forced-rollback certification pass.
