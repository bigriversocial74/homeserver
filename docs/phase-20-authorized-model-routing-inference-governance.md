# Phase 20 — Authorized Model Routing and Inference Governance

## Score progression

- Initial audit: **6.2/10**
- Final score: pending exact-head certification

## Purpose

Phase 20 turns the existing Ollama and optional OpenRouter providers into bounded inference adapters behind one HomeServer-owned authority layer. A model provider never decides whether a prompt may run, which context may leave the device, whether fallback is allowed, or whether an agent has authority.

## Authority model

Every governed inference binds:

- actor type and actor identity
- local Control Center or exact agent assignment
- agent and assignment revisions
- wrapper and connection identity
- connection authority revision
- policy ID, revision, and hash
- exact purpose and purpose hash
- data classification
- optional Phase 16E private-resource selector
- ordered providers and allowed models
- prompt hash and caller-supplied context hash
- input, output-token, request, token, and spending limits
- idempotency key

Authority is revalidated before every provider attempt and again before a result is committed. A policy, agent, assignment, connection, selector, provider configuration, model restriction, or emergency-stop change fails closed.

## Provider behavior

### Ollama

- fixed loopback adapter at `127.0.0.1:11434`
- installed local chat models only
- no remote context transfer
- policy default or explicitly allowed model only

### OpenRouter

- fixed reviewed API endpoint
- operating-system credential vault
- provider must already be explicitly enabled for remote context
- governed requests disable OpenRouter's provider-managed model fallback
- policy must explicitly include OpenRouter
- zero-data-retention is enforced when the policy requires it
- public/safe metadata may use `public_only`
- private derived context requires an exact active Phase 16E selector with `approved_provider=openrouter`
- raw `private_source` and `secret` data are never authorized for remote inference

## Fallback

Fallback is explicit in the HomeServer policy. Each attempt receives a fresh decision hash and complete authority revalidation. A failed remote request never silently falls back to local inference, and local inference never silently falls back to a remote provider.

## Evidence and privacy

HomeServer stores:

- immutable policy authority
- request and authority hashes
- provider/model attempts and decision hashes
- bounded token and cost evidence
- private model output in a separate local-only table
- immutable terminal receipts
- append-only audit events

HomeServer snapshots and the Control Center expose only safe metadata and hashes. Prompts and private output are not exposed through governance snapshots, wrapper APIs, or receipts.

## Restart, cancellation, and retention

Reserved or running requests become interrupted on restart and receive immutable receipts. Explicit cancellation and policy revocation terminate active requests fail closed. Evidence tables do not silently delete history; retention overflow requires an archival solution.

## Local API

- `GET /v1/models/governance`
- `POST /v1/models/governance/policies`
- `POST /v1/models/governance/policies/revoke`
- `POST /v1/models/inference`
- `POST /v1/models/inference/cancel`

## Database

Additive HomeServer-local SQLite migration:

`database/migrations/0028_authorized_model_routing.sql`

The HomeServer service applies it automatically. No Microgifter, VP3, or POD MySQL import is required.

## Certification requirements

- permanent hostile-boundary validator
- migration and immutable-evidence mutation tests
- retained Phase 16D, 16E, 17, 18, and 19 validation
- frontend validation and production build
- native service tests
- strict service and workspace lint
- complete HomeServer Production Quality
- verified installer, LocalSystem security, signed update, and forced rollback
