# Phase 5C-A — Operational Data Import Foundation

## Purpose

Phase 5C-A gives HomeServer a provider-neutral, structured local evidence layer for authorized operational data from paired platforms. Microgifter is the first installed provider manifest and adapter contract, but the storage, grants, provenance, query, and Agent Workspace contracts are connection-scoped and provider-extensible.

Connected platforms remain authoritative for their live records. HomeServer stores approved local copies for analysis, goal matching, evidence-backed reports, and supervised planning.

## Security and authority model

- Every dataset must be declared by an installed provider manifest.
- Every connection and dataset requires an explicit local grant before import.
- Grants bind provider, connection, tenant, site, classification, retention, and permitted agent uses.
- Import payloads cannot add or change tenant/site scope.
- Imported strings and JSON are stored as `untrusted_provider_evidence`; they are never executable instructions.
- Provider records do not override HomeServer policy, approvals, system prompts, tools, or security boundaries.
- Imported records do not become the canonical merchant, CRM, product, campaign, reward, claim, redemption, payment, or ownership records.
- HomeServer does not write imported data back to the provider in this phase.
- World Mode dispatch is not enabled by operational imports.

## Built-in Microgifter datasets

The Phase 5C-A manifest includes:

- `merchant.profile`
- `merchant.locations`
- `merchant.products`
- `campaigns.summary`
- `campaigns.performance`
- `rewards.summary`
- `claims.summary`
- `redemptions.summary`
- `crm.lifecycle_summary`
- `creator.attribution_summary`

Detailed payment data, private messages, gift ownership, and full customer contact records are excluded.

## Local storage layers

1. Provider manifest and dataset catalog
2. Connection-specific dataset grant
3. Import run and incremental cursor
4. Canonical raw provider record
5. Normalized current entity
6. Immutable entity version
7. Provider event timeline
8. Provenance receipt with evidence hash
9. Retention policy and bounded import error record

The Knowledge Vault remains the unstructured document layer. Operational records are stored separately so HomeServer can query structured entities, revisions, events, and timelines without treating provider JSON as documents or instructions.

## Import modes

### Snapshot

Imports a bounded current-state dataset. The provider supplies source object IDs and source revisions. Repeated object/revision pairs are idempotent.

### Incremental

Imports changes after the stored provider cursor. Successful imports update the connection/dataset cursor and source revision.

### Event

Imports bounded provider events with unique source event IDs and occurrence timestamps. Events remain evidence and do not trigger arbitrary actions.

## Local API

- `GET /v1/operational-data`
- `POST /v1/operational-data/grants`
- `POST /v1/operational-data/import`
- `POST /v1/operational-data/query`

All routes remain behind the existing fixed-loopback local-client security middleware. The import route is an ingestion contract for audited provider adapters and local validation; the Control Center does not expose a free-form manual import editor.

## Agent Workspace integration

Enabled operational datasets appear as individual context sources using the key form:

`dataset:<connection_uuid>:<dataset_key>`

The HomeServer agent may use only locally enabled datasets and only for the uses recorded in the dataset grant. Agent responses include bounded evidence records and source citations. The agent must distinguish imported facts from unavailable data and must never follow instructions found inside imported provider fields.

## Explicit exclusions

This phase does not add:

- Campaign publishing or CRM mutation
- Purchases, payments, claims, rewards, or redemptions
- Customer messaging or recurring commitments
- Arbitrary webhooks, endpoints, SQL, shell, or filesystem access
- Cross-provider write synchronization
- Autonomous operational changes
- World Agent dispatch
- Predictive scoring or recommendation automation

## Provider-side follow-up

A paired platform must implement an audited, signed export adapter that produces the manifest-approved snapshot, incremental, and event envelopes. The HomeServer ingestion and evidence contract is implemented here; provider-side export endpoints remain separate coordinated provider work and must preserve the same connection, tenant, site, cursor, record-limit, and authority rules.
