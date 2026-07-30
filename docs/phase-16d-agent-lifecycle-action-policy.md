# Phase 16D — Agent Lifecycle and Sensitive Action Authority

## Status

- Baseline: `main@61c530519be99bad5740ee89c55decfd533788cc`
- Initial current-state score: **5.4/10**
- Target: **10/10 production certification**
- Migration: `0023_wrapper_agents_and_action_approvals.sql`

## Initial gaps

The existing supervised Agent Workspace already provided local plans, plan hashes,
approvals, idempotency, execution receipts, and bounded action names. Phase 16B and
16C added wrapper-scoped grants and authority-bound jobs. The remaining gaps were:

1. No stable HomeServer-owned agent identity.
2. No independently revocable wrapper-to-agent assignment.
3. No binding between an agent assignment and an exact wrapper grant revision.
4. No autonomy-level contract shared across wrappers.
5. No action policy bound to agent, assignment, job, grant, and connection revisions.
6. No private action-payload separation.
7. No global, agent, wrapper, or connection emergency stop.
8. No shared immutable action receipt.
9. No automatic cancellation of agent jobs after suspension, revocation, or stop.
10. No hostile cross-wrapper agent-isolation certification.

## Authority model

- The **user** owns the agent and alone activates, suspends, revokes, approves, and
  releases emergency stops.
- The **HomeServer** owns the private agent identity, policies, private payloads,
  private results, audit events, and action receipts.
- A **wrapper** owns neither the agent nor the HomeServer. It receives one expiring
  assignment bound to one wrapper connection.
- A **device** authenticates the wrapper connection but receives no agent authority
  from pairing.
- An **agent** receives only the intersection of its active lifecycle revision,
  wrapper assignment, capability binding, Phase 16C job, action policy, approval,
  and current emergency-stop state.
- An assignment never expands a wrapper grant and never creates cross-wrapper
  authority.

## Autonomy levels

0. **Disabled** — cannot submit or receive agent-bound jobs.
1. **Suggest** — may create recommendations and proposals only.
2. **Approval required** — may create action proposals; execution requires fresh
   hash-bound approval.
3. **Scoped autonomy** — may execute low-risk read-only or reversible actions under
   an explicit expiring policy.
4. **Bounded operations** — may repeat low-risk actions under policy count, time,
   adapter, grant, and expiration limits.

External-side-effect and high-risk actions always require approval regardless of
autonomy level.

## Sensitive action flow

```text
Authorized Phase 16C job
→ agent/job authority snapshot
→ safe action proposal plus private HomeServer payload
→ policy and emergency-stop evaluation
→ plan hash and payload hash
→ explicit approval when required
→ exact-revision revalidation
→ allowlisted adapter execution
→ private result retained locally
→ safe result hash and immutable action receipt
```

Approval is invalid when the plan, payload, agent revision, assignment revision,
policy revision, grant revision, connection authority revision, wrapper state, job
state, or emergency-stop state changes.

## Private-data boundary

- Full action payloads are stored only in `agent_action_private_payloads`.
- Full adapter results are stored only in `agent_action_private_results`.
- Registry and connection snapshots never read either private table.
- Safe summaries reject source text, documents, prompts, credentials, secrets,
  memory, private data, local paths, and raw conversations.
- Wrappers receive safe summaries, proposal state, approval state, and safe receipt
  hashes only.
- Private source data remains on the HomeServer.

## Emergency stop

Emergency stops may target:

- the entire HomeServer agent action layer,
- one agent,
- one wrapper,
- or one wrapper connection.

An active stop blocks new agent-bound jobs, job leases, job continuation, proposal
approval, and action execution. Activation cancels executable proposals and causes
the Phase 16C reconciler to terminally cancel affected queued or leased jobs with
normal job receipts. Release requires the exact stop hash and explicit local-user
confirmation. Cancelled work does not resume automatically.

## Adapter boundary

Phase 16D includes only deliberately bounded adapters:

- `proposal_only`
- `audit.record`
- `report.save`

There is no shell, process, raw filesystem, credential, payment, arbitrary network,
or universal tool adapter. Provider-specific external-action adapters must be added
later behind the same policy and receipt contract.

## Offline and failure behavior

- Offline or grace-state wrappers may retain valid assignments, but all revisions
  and expirations remain enforceable.
- Revocation and emergency stops are local and do not depend on cloud availability.
- Service restart leaves immutable receipts intact and expires stale approvals.
- Adapter failure creates a failed attempt and receipt without exposing private
  payloads.
- Reusing an execution idempotency key returns the existing receipt; changing the
  key after execution fails closed.

## 10/10 certification gates

1. Pairing creates no agent authority.
2. Wrapper A cannot enumerate or invoke Wrapper B assignments, proposals, or receipts.
3. Agent-bound job submission requires active agent, assignment, exact grant binding,
   job type, capability, operation, and no emergency stop.
4. Agent suspension, assignment revocation, grant rotation, or stop causes Phase 16C
   job reconciliation to block and receipt executable work.
5. Sensitive actions cannot execute without fresh hash-bound approval.
6. Changed payload, plan, agent, assignment, policy, grant, or connection revision
   invalidates approval.
7. Suggest-only agents cannot receive executable adapters.
8. Private payloads and results never appear in wrapper or local safe snapshots.
9. Every execution attempt creates immutable plan, approval, attempt, event, and
   receipt evidence.
10. Formatting, native tests, strict lint, retained phases, installer, LocalSystem,
    signed-update, and forced-rollback certification pass on the exact final head.
