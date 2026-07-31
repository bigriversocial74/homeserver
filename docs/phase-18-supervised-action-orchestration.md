# Phase 18 — Supervised Action Orchestration

## Score progression

- Initial score: **5.6/10**
- Target score: **10/10**
- Baseline: `main@79cd883d37d73841d49750757ad949f38dfce4e6`

## Initial defects

Phase 17 could execute only low-risk, approval-free tools. It could not pause an ordered plan on a Phase 16D action proposal, bind that checkpoint to exact approval evidence, resume once after approval, propagate rejection or expiration into dependent steps, or link the action and runtime receipts into one immutable chain.

## Delivered architecture

Phase 18 keeps the existing certified owners intact:

- Phase 16D owns the action proposal, approval, execution, one-time approval consumption, policy-window enforcement, emergency stops, action attempt, private action result, and immutable action receipt.
- Phase 17 owns ordered plans, predecessor release, runtime steps, plan state, cancellation, and runtime receipts.
- Phase 16E remains mandatory for the completed `action.propose` wrapper job safe projection.
- Phase 18 owns only the checkpoint that binds those contracts together.

The closed catalog adds `action.supervised`, which accepts only `action.propose` jobs and always requires the separate proposal lifecycle. The low-risk Phase 17 worker still cannot execute it.

## Checkpoint evidence

Each checkpoint captures and revalidates:

- runtime plan and step identity plus the runtime plan hash;
- wrapper job, wrapper, connection, grant, and authorization decision;
- agent and assignment identities and revisions;
- Phase 16D proposal, policy, approval, proposal plan hash, and payload hash;
- policy, grant, and connection-authority revisions;
- action type, risk class, and registered adapter;
- expiration and compensation policy.

Approval resume requires the exact approval ID, plan hash, payload hash, agent revision, assignment revision, policy revision, grant revision, and connection-authority revision. The Phase 16D execution path consumes the approval exactly once.

## Failure behavior

Rejection, expiration, cancellation, emergency stop, policy change, grant change, assignment change, agent change, connection-authority change, missing evidence, or a terminated runtime plan fails closed. The current step and dependent jobs are terminated, active proposals and approvals are cancelled where applicable, and immutable failed runtime and orchestration receipts are recorded.

Proposal or checkpoint construction failures cannot strand a completed wrapper job. They write a failed Phase 17 receipt, fail the plan, and retain only hashed error evidence.

## Compensation

Compensation is catalog-bound. The first supported reversible action is `report.save`, compensated by `report.delete`. Manual rollback requires the exact `ROLLBACK ACTION <checkpoint_id>` confirmation. Automatic compensation is available only when explicitly selected and a later plan failure occurs. Compensation records target hashes, not report contents.

No generic rollback, shell, arbitrary process, unrestricted filesystem, credential read, wildcard tool, or caller-supplied adapter exists.

## Control Center

The Agent Runtime page adds supervised checkpoint cards with safe proposal previews, approval and proposal states, expiration, captured hashes, compensation status, and immutable receipt links. Operators continue to approve or reject through the separate Agent Control Center. Only supported completed actions expose a rollback control.

Private action payloads and private action results are never rendered. The Tauri bridge pins rollback audit identity to `local_control_center`.

## Certification gates

A 10/10 score requires the exact final head to pass:

- the permanent Phase 18 validator and native contract tests;
- all retained Phase 13–17 workflows;
- retained Phase 16D authority and Phase 16E privacy validators;
- frontend syntax and production build;
- Rust formatting, service tests, full workspace tests, and strict Clippy;
- service/cloud/backup/update smoke tests;
- NSIS installer, installed LocalSystem security, backup, uninstall, and registration checks;
- Authenticode update and forced rollback;
- verified installer and release-log artifacts.
