# Phase 17 — Authorized Agent Tool Runtime

## Status

- Baseline: merged Phase 16E at `main@63fd565ec1149a358f7c43651590478901f731e3`
- Initial current-state audit: **5.8/10**
- Implementation branch: `feature/phase-17-authorized-agent-tool-runtime`
- Merge status: draft and unmerged

## Purpose

Phase 17 turns the certified Phase 16 authority contracts into an executable local runtime without creating a second job, approval, privacy, or receipt system.

The runtime consumes Phase 16C wrapper jobs, revalidates Phase 16D agent authority before every tool step, and completes work through the Phase 16C completion path so Phase 16E result-egress enforcement remains mandatory.

## Authority chain

Every executable runtime step must retain the complete authority chain:

1. An active wrapper identity and connection.
2. An active scoped Phase 16B capability grant.
3. A Phase 16C wrapper job and immutable authority snapshot.
4. A Phase 16D HomeServer agent, wrapper assignment, capability binding, and current agent revision.
5. An active execution policy whose action type, risk class, approval mode, adapter, limits, and expiration match the runtime tool.
6. A low-risk, approval-free execution policy with scoped autonomy. Sensitive and proposal-gated actions remain on the retained Phase 16D proposal lifecycle and are not executed by this first runtime.
7. A Phase 16E selector and result-egress decision when private knowledge or private results are involved.

Pairing alone grants no runtime, agent, model, tool, data, action, or cross-wrapper authority.

## Tool registry

The local `agent_tool_catalog` is intentionally narrow. Each tool records:

- stable tool and adapter keys;
- semantic version;
- risk class;
- approval requirement;
- accepted wrapper-job types;
- bounded input and output schemas;
- maximum execution duration;
- lifecycle state.

The initial adapters are:

- `wrapper.status.read` — safe wrapper and connection status;
- `receipt.read` — bounded receipt summaries for the same connection;
- `audit.record` — immutable local hash-and-label audit evidence;
- `result.compose` — private local result composition followed by mandatory Phase 16E egress evaluation.

There is no shell, arbitrary process, unrestricted filesystem, credential-read, wildcard-tool, caller-supplied adapter execution, or sensitive action execution.

## Runtime plan lifecycle

A runtime plan contains one to 32 ordered steps. Each step is submitted as a normal Phase 16C wrapper job with:

- the plan agent as the job submitter;
- an exact capability and operation;
- wrapper and connection scope;
- idempotency, correlation, and predecessor causation identifiers;
- bounded expiration, attempts, result size, and execution time;
- a canonical runtime plan hash retained locally with the ordered job bindings;
- optional Phase 16E private selector and output schema.

The first step is immediately dispatchable. Later steps remain unavailable until the preceding step completes and the runtime unlocks exactly the next queued job. A second predecessor-state check fails closed if a job is ever presented out of order.

The initial Phase 17 runtime rejects wrapper-job approval IDs and approval plan hashes. Sensitive actions continue through the separate, already-certified Phase 16D `action.propose` → approval → execution lifecycle.

The persistent runtime worker leases one job at a time. Before execution it revalidates:

- the captured agent job binding;
- current agent, assignment, capability-binding, grant, and connection revisions;
- emergency-stop state;
- tool catalog state;
- agent tool restrictions;
- policy adapter and risk-class equality;
- scoped autonomy level;
- low-risk approval-free policy state;
- execution-window limits;
- ordered predecessor completion;
- tool timeout limits.

A changed, expired, suspended, revoked, cross-wrapper, over-limit, out-of-order, or mismatched authority fails closed.

## Private result boundary

Runtime adapters receive private job input only through the Phase 16C worker lease. Private input and full private results are never included in runtime snapshots, events, or receipts.

All successful adapters call the existing `wrapper_jobs::complete_job` path. That path stores the private result locally, invokes Phase 16E egress enforcement, applies result-size and token limits, writes only an approved safe projection, and creates the immutable wrapper-job receipt.

The Phase 17 runtime receipt references hashes and identifiers from the Phase 16C receipt. It never copies raw prompts, source text, credentials, local paths, private payloads, full results, or unfiltered provenance.

## Failure, cancellation, and restart behavior

- Failed authority validation or adapter execution terminates the current job and plan.
- Remaining queued or active plan jobs are cancelled through the Phase 16C cancellation contract.
- Explicit plan cancellation requires the exact `CANCEL PLAN <plan_id>` confirmation.
- Running attempts are marked interrupted only during service initialization, never during snapshot reads or retention passes.
- Durable wrapper-job states are reconciled into the narrower runtime-step state machine.
- `waiting` jobs map back to queued runtime steps; dead-letter work maps to failure.
- A completed wrapper job without its immutable Phase 17 receipt fails closed as `runtime_receipt_missing` after recovery.
- Expired plans fail closed.
- Emergency stops, agent changes, assignment changes, grant changes, policy changes, and connection-authority changes remain enforced by the retained Phase 16 reconciliation paths.

## Immutable evidence

Phase 17 adds:

- append-only runtime events;
- per-attempt records;
- immutable per-step runtime receipts;
- plan and step hashes;
- links to immutable Phase 16C job receipts;
- safe-result hashes only;
- local hash-only audit records.

## Local API

- `GET /v1/agent-runtime`
- `POST /v1/agent-runtime/policies/create`
- `POST /v1/agent-runtime/plans/create`
- `POST /v1/agent-runtime/plans/cancel`
- `POST /v1/agent-runtime/run-once`

The snapshot exposes tool, plan, step, and receipt metadata. It explicitly reports:

- `private_inputs_exposed: false`
- `private_results_exposed: false`
- `direct_tool_bypass_allowed: false`
- `phase16e_egress_required: true`

## Control Center visibility

The desktop Control Center includes an **Agent Runtime** page loaded through the existing explicit `homeserver:rendered` lifecycle. It does not add a DOM observer network.

The page uses a narrow trusted Tauri bridge because browser-originated requests are intentionally rejected by the loopback API. It shows:

- active HomeServer agents and wrapper assignments;
- runtime state, active plans, ordered steps, failures, and completion progress;
- the versioned low-risk tool catalog;
- pending Phase 16D approvals and active emergency stops without merging those sensitive actions into the low-risk runtime;
- immutable runtime receipt hashes linked to Phase 16C evidence;
- explicit privacy indicators confirming that private inputs and full private results are hidden, direct tool bypass is disabled, and Phase 16E egress remains mandatory.

Operators can refresh state, run one bounded runtime cycle, open the supervised Agent Control Center, and cancel an active runtime plan only after entering the exact `CANCEL PLAN <plan_id>` confirmation. The trusted native bridge pins cancellation evidence to the `local_control_center` actor; browser JavaScript cannot supply or replace the audit identity. No raw private job input or full result is rendered.

## Deployment

Migration `0025_authorized_agent_tool_runtime.sql` is additive and is applied automatically by the HomeServer service during startup. No Microgifter, POD, or VP3 MySQL migration is required.

## 10/10 certification gates

Phase 17 cannot be scored 10/10 or merged until the exact final head passes:

- the permanent Phase 17 migration and hostile-authority validator;
- retained Phase 13 through Phase 16E workflows;
- frontend validation and production build;
- Rust formatting;
- native HomeServer service tests;
- strict service and workspace Clippy;
- full workspace tests;
- service, cloud, backup, update, wrapper-job, agent, privacy, and runtime smoke tests;
- NSIS installer construction;
- installed LocalSystem security, backup, uninstall, and data-preservation checks;
- Windows registration verification;
- Authenticode-signed update and forced rollback;
- verified release and installer artifact uploads.

## Merge rule

Keep the pull request draft and unmerged until the exact implementation head is fully certified and David Evans gives explicit merge approval.