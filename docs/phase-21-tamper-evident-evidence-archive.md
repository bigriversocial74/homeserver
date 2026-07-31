# Phase 21 — Tamper-Evident Evidence Archive

## Score progression

- Initial audit: **4.4/10**
- Final score: pending exact-head certification

## Purpose

Phases 16 through 20 create immutable authority decisions, approvals, execution receipts, scheduling evidence, and governed inference receipts. Phase 21 gives that evidence a machine-encrypted archive and export boundary that is fully verifiable by the originating HomeServer and externally verifiable by package SHA-256 without weakening or deleting the source records.

## Safety boundary

Source evidence is never deleted by Phase 21. The archive engine reads only this closed, explicitly reviewed table allowlist:

- `service_events`
- `wrapper_events`
- `wrapper_grant_events`
- `wrapper_authorization_receipts`
- `wrapper_job_events`
- `wrapper_job_execution_receipts`
- `agent_action_receipts`
- `agent_lifecycle_events`
- `private_knowledge_access_receipts`
- `agent_runtime_receipts`
- `agent_runtime_events`
- `agent_runtime_audit_records`
- `agent_supervised_action_receipts`
- `agent_supervised_compensation_receipts`
- `agent_supervised_action_events`
- `agent_schedule_event_inbox`
- `agent_schedule_receipts`
- `agent_schedule_audit_events`
- `model_provider_usage_receipts`
- `model_inference_receipts`
- `model_inference_events`

A table is not admitted by a name suffix or pattern. Any future evidence table remains excluded until code review explicitly adds it to the allowlist. The engine also rejects its own archive tables and tables associated with private results, private inputs, messages, documents, payloads, credentials, secrets, tokens, and synchronization queues. Prompts, model output, document content, operational payloads, credentials, and arbitrary database tables are never included.

## Canonical evidence

Each eligible row is converted into deterministic canonical JSON. Binary values are replaced by their SHA-256 digest rather than copied. Every archive record binds:

- source table
- exact source primary key
- source timestamp when present
- canonical field map
- per-record SHA-256
- ordinal
- cumulative chain hash

The first record chains from the previous archive manifest hash. Every later record chains from its predecessor. The manifest records the previous archive identity and hash, record and table counts, record-stream SHA-256, final chain root, policy revision, installation identity hash, application version, and privacy assertions.

## Package format

Evidence packages use the `.mgha` extension and fixed `MGHEAR01` package magic. A canonical `manifest.json` and `records.ndjson` stream are packed into a deterministic gzip-compressed tar payload. The payload is encrypted with AES-256-GCM using the Windows machine-protected HomeServer backup key with a separate Phase 21 domain-derived key.

The package header binds the archive identity, sequence, nonce, manifest hash, compressed-payload hash, and previous archive hash. HomeServer immediately decrypts and verifies every package before recording it as verified.

## Restart and idempotency

Archive requests have unique idempotency keys. A run begins in `collecting`; interrupted runs are marked failed at startup and any partial managed file is removed. A verified source row may belong to only one archive, preventing duplicate coverage after retries.

## Policies and scheduling

Archive policy revisions are immutable. The active revision controls:

- enabled state
- interval in hours
- maximum records per archive
- retained local package count
- maximum package size

The service checks for due archives from its existing bounded maintenance scheduler. Manual archive creation remains explicitly confirmed through the trusted Control Center.

## Export-gated retention

Local package pruning is fail-closed. An old archive package is eligible for pruning only when:

1. the archive is verified;
2. a trusted desktop export completed;
3. an immutable export receipt binds the destination filename and package hash; and
4. the package is beyond the active policy retention count.

The immutable archive row, source memberships, export receipt, and event chain remain in SQLite after local package pruning. Source evidence is never deleted.

## Control Center

Agent Runtime Control Center adds a Tamper-evident evidence archives section showing:

- policy revision and schedule
- unarchived record count
- archive sequence and state
- storage state
- record/table counts
- chain, manifest, and package hashes
- verification and export counts

The UI never renders managed storage paths, source row fields, prompts, or private results. Owners can create, verify, export, and update the bounded policy through trusted Tauri commands.

## Local API

- `GET /v1/evidence-archives`
- `POST /v1/evidence-archives/policies`
- `POST /v1/evidence-archives/create`
- `POST /v1/evidence-archives/verify`
- `POST /v1/evidence-archives/exports`
- `GET /v1/evidence-archives/{archive_id}/package`

All routes remain behind the fixed loopback host and trusted local-client header.

## Database

Additive HomeServer-local SQLite migration:

`database/migrations/0029_tamper_evident_evidence_archive.sql`

No Microgifter, VP3, POD, or wrapper MySQL import is required.

## Certification requirements

- hostile evidence/private-content validator
- deterministic archive-chain tests
- migration and immutable mutation tests
- Agent Workspace and Control Center frontend validation
- retained Phase 17 through Phase 20 validation
- full native service tests
- strict Windows and workspace lint
- complete HomeServer Production Quality
- verified installer, LocalSystem security, signed update, and forced rollback

## Final trust hardening

- Evidence admission is an explicit reviewed table allowlist. A future table is excluded even when its name ends in `_events`, `_receipts`, or `_audit_records` until a code review adds it.
- The seeded policy hash is the canonical SHA-256 of the complete default policy document, and every active policy is recomputed during health checks.
- Health checks verify the complete sequence of verified archive predecessor identities and manifest hashes.
- Archive idempotency, policy binding, predecessor identity, sequence, managed filename/path, encryption mode, actor, and creation timestamp are immutable at the SQLite layer.
- The desktop computes SHA-256 while streaming every exported byte, deletes a mismatched file, and records an export receipt only after the downloaded digest equals the verified package digest.
- `.mgha` contents are fully decryptable and chain-verifiable by the originating HomeServer installation. Other systems can independently verify the exported package bytes against the displayed or recorded SHA-256 without receiving the machine encryption key.
