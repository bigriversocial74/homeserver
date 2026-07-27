# Phase 5B Agent Workspace, supervised approvals, and World Mission foundation

## Purpose

The HomeServer Agent Workspace is the prompt-first local control plane for private analysis, saved goals, connected-site context, supervised plans, approval decisions, bounded execution, World Mission drafts, and durable receipts.

Connected platforms remain authoritative for their live operational records. HomeServer stores local goals, conversation context, plans, approvals, reports, mission drafts, and execution evidence. It does not silently become the master record for Microgifter, a CRM, or another paired provider.

## Current data context

This Phase 5B slice can use:

- HomeServer system and backup state
- Cloud connection metadata and synchronization status
- Knowledge Vault keyword, semantic, or hybrid retrieval
- Installed local model inventory and the configured chat model
- Saved goals
- Agent threads, plans, approvals, reports, missions, and receipts

The Agent Workspace visibly marks operational platform imports as unavailable until Phase 5C. Initial snapshots, incremental cursors, normalized business entities, operational events, historical metrics, and goal-to-business-result analysis are not fabricated from connection metadata.

## Agent modes

The interface supports five explicit modes:

- **Ask** — explain currently available local context.
- **Analyze** — compare available context and identify evidence or gaps.
- **Plan** — prepare a structured recommendation or supervised action request.
- **Dispatch Draft** — prepare a bounded World Mission without dispatching it.
- **Execute Request** — request a plan that still requires a separate local approval and execution action.

A prompt never grants authority by itself.

## Supervised plan lifecycle

Plans use the following lifecycle:

```text
draft
awaiting_approval
approved
executing
completed
failed
rejected
cancelled
expired
```

Every executable plan contains:

- Requesting actor type and identity
- Optional thread and goal
- Exact action and sanitized arguments
- Optional connection target
- Selected dataset keys
- Risk level
- Fresh-state token
- Expiration
- Immutable plan hash

The local Control Center is the only approval surface in this phase. MCP clients can request, inspect, list, or cancel their own unexecuted work. They cannot approve or execute plans.

## One-use approval contract

An approval is bound to the exact plan hash and expiration. Before execution, HomeServer validates:

- The plan is still approved
- The approval has not expired
- The approval has not been consumed
- The approval hash still matches the plan
- The target connection identity and relevant state have not materially changed
- The action remains in the installed closed-world allowlist

If material target state changes, HomeServer rotates the fresh-state token and plan hash and returns the plan to `awaiting_approval`.

Execution consumes the approval exactly once. Idempotency records and durable receipts prevent a repeated request from silently running the same plan again.

## Initial bounded executors

Only these action types are installed:

- `backup.create`
- `model.health_test`
- `cloud.sync_connection`
- `cloud.sync_all`
- `report.save`

The cloud actions reuse the existing signed, connection-scoped synchronization contract. That contract continues to allow only the existing low-risk operations:

- `device.heartbeat`
- `local.settings.snapshot`
- `cache.refresh.request`

This phase does not install arbitrary shell execution, unrestricted filesystem access, model deletion, software installation, commerce writes, payments, claims, redemption, rewards, campaign publishing, CRM mutation, bulk messages, or recurring commitments.

## World Mode and World Missions

World Mode is the future interactive operating state in which an authorized World Agent can use tools, skills, tasks, and approved knowledge to converse with avatars and agents, act on conversations, manage commitments, close conversations properly, and schedule follow-up.

This Phase 5B slice creates the local foundation only:

- World Mission drafts
- World tasks
- World conversation lifecycle records
- Conversation commitments
- Follow-ups
- Mission events
- World receipts

A World Mission draft includes an objective, World Agent identity, optional Microgifter connection, allowed operations, prohibited operations, limits, disclosure policy, and expiration.

Allowed draft operations include discovery, Store Canvas visits, questions, comparisons, information requests, recommendation preparation, follow-up scheduling, and conversation closure.

The following remain explicitly prohibited:

- Purchase or payment
- Claim or redemption
- Sharing a private profile
- Accepting a recurring commitment
- Publishing a campaign
- Bulk messaging

World Mission dispatch is not installed in this slice. Saving a mission creates a local draft and task record only. No World Agent is sent into the World Canvas.

## MCP boundary

The supervised MCP surface is request-only. It may expose tools to:

- Prompt the HomeServer agent
- Submit a supervised plan
- Read or list the requesting client’s plans
- Cancel the requesting client’s unexecuted plan
- Draft or inspect a World Mission
- Read execution receipts produced by that client’s plans

MCP has no approval, execution, or World Mission dispatch tool. The local Control Center remains the human authority boundary.

## Receipts and auditability

Every bounded execution records:

- Plan ID and approval ID
- Immutable plan hash
- Action type and connection target
- Idempotency key
- Start and completion times
- Completion or failure state
- Bounded result code
- Sanitized summary and result payload

Receipts are local evidence. They do not replace the connected provider’s canonical business receipt or transaction record.

## Next phase

Phase 5C adds Operational Data Intelligence:

- Provider data manifests
- Initial snapshots
- Incremental synchronization cursors
- Structured operational events
- Normalized entities and provenance
- Historical metrics
- Goal matching
- Insights, forecasts, recommendations, and scheduled reports

Once that layer is validated, Agent Workspace prompts can analyze real imported platform datasets rather than connection metadata alone. A later World Mode phase can then add approved dispatch, interactive avatar conversations, tools, commitments, proper closure, follow-up, and World receipts through the Pairing Node.
