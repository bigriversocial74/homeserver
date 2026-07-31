# Phase 19 — Authorized Scheduling and Event Triggers

## Status

- Baseline: merged Phase 18 at `main@6d88d6df8e213a479512d57773d002d7d9d67896`
- Initial current-state audit: **3.9/10**
- Implementation branch: `feature/phase-19-authorized-scheduling-event-triggers`

## Purpose

Phase 19 adds persistent time and event triggers without creating a second executor, approval system, or privacy path. The scheduler can only create a fresh Phase 17 runtime plan after revalidating the exact Phase 16 agent, assignment, connection, capability binding, grant, and execution-policy revisions captured when the schedule was created. Approval-gated steps continue through Phase 18 supervision.

## Trigger contract

Supported triggers are deliberately closed:

- one-time UTC execution;
- bounded intervals from one minute to 30 days;
- safe local events from the fixed topics:
  - `wrapper.job.completed`
  - `runtime.plan.completed`
  - `supervised.action.completed`
  - `cloud.sync.completed`

There is no cron expression interpreter, arbitrary script, shell command, external webhook, caller-selected adapter, or direct tool execution.

Each schedule defines:

- `skip`, `fire_once`, or `fail` misfire behavior;
- `skip` or single-coalesced `queue_one` overlap behavior;
- bounded debounce;
- a maximum run count;
- schedule expiration;
- an immutable hash of its private runtime-plan template;
- an immutable hash of its captured authority snapshot.

## Authority and execution

At schedule creation HomeServer captures:

- agent and agent revision;
- wrapper assignment and assignment revision;
- wrapper and connection authority revision;
- each required capability binding and binding grant revision;
- each active capability grant and grant revision;
- each action execution policy and policy revision.

At every trigger, HomeServer revalidates the exact snapshot. Changed, expired, suspended, revoked, cross-wrapper, or mismatched authority fails closed. The scheduler then calls the existing Phase 17 `create_plan` contract. It never leases jobs, invokes adapters, executes proposals, consumes approvals, or filters results directly.

Low-risk work remains in Phase 17. `action.supervised` steps remain in Phase 18. Phase 16E result egress remains mandatory through those retained paths.

## Event privacy

The local event inbox accepts only the closed topic catalog and safe metadata. Recursive forbidden-key checks reject private inputs, results, prompts, credentials, secrets, local paths, source documents, memories, and conversation data. Full event payloads are not accepted or exposed.

Private runtime-plan templates are stored in a separate private table. Schedule snapshots expose hashes and metadata only.

## Reliability

- deterministic trigger tokens provide restart-safe idempotency;
- monotonic event sequences and per-schedule cursors prevent replay;
- interrupted plan creation reconciles against the scheduler run identity;
- a recovered runtime plan is linked to the original schedule run;
- an ambiguous interruption fails closed instead of creating a second plan;
- queued overlap is bounded to one existing run;
- immutable evidence limits fail closed pending archival instead of deleting history.

## Immutable evidence

Phase 19 adds append-only safe events and audit events plus immutable run receipts. Update and delete triggers protect all three evidence tables. Each receipt binds the schedule, trigger token, authority hash, template hash, runtime plan ID/hash when available, outcome, result code, and completion time.

## Local API

- `GET /v1/agent-schedules`
- `POST /v1/agent-schedules/create`
- `POST /v1/agent-schedules/pause`
- `POST /v1/agent-schedules/resume`
- `POST /v1/agent-schedules/cancel`
- `POST /v1/agent-schedules/events/record`
- `POST /v1/agent-schedules/run-once`

The trusted desktop bridge pins operator actions to `local_control_center`.

## Deployment

Migration `0027_authorized_agent_scheduling.sql` is additive and is applied automatically during HomeServer startup. No Microgifter, VP3, or POD MySQL migration is required.

## 10/10 certification gates

Phase 19 cannot be scored 10/10 or merged until the exact final head passes:

- permanent Phase 19 migration and hostile-boundary validation;
- retained Phase 13 through Phase 18 workflows;
- frontend validation and production build;
- Rust formatting;
- native service and Phase 19 immutable-evidence tests;
- strict service and workspace Clippy;
- full workspace tests;
- service, cloud, backup, update, runtime, orchestration, and scheduling smoke tests;
- NSIS installer construction;
- installed LocalSystem security, backup, uninstall, and data-preservation checks;
- Windows registration verification;
- Authenticode-signed update and forced rollback;
- verified release and installer artifact uploads.
