# HomeServer Agent Chat and Connection Interface v1

Status: implementation complete; complete Production Quality validation running

## Scope

- Preserve the existing multi-site pairing node and Phase 6A service contract.
- Replace the current operationally dense Agent Workspace presentation with a chat-first HomeServer Agent page.
- Keep existing local thread and message persistence.
- Add a conversation sidebar with New Chat and chat-history navigation.
- Add a sticky footer composer with mode, model, goal, and bounded context controls.
- Add a Phase 6A Microgifter connection drawer using the existing local service endpoints.
- Keep local operation visibly available when Microgifter is offline, suspended, revoked, or unpaired.
- Preserve the existing supervised Agent Workspace backend and records for rollback and permanent validation compatibility.

## Files

- Add `src/homeserver-agent-chat.js`.
- Add `src/homeserver-agent-chat.css`.
- Extend `src-tauri/src/cloud.rs` with Phase 6A local-service command bridges.
- Register the new commands in `src-tauri/src/lib.rs`.
- Update `src/main.js` so background system refreshes do not replace an active Agent Chat canvas.
- Keep `src/agent-workspace.js` loaded as a disabled compatibility and rollback module while the new chat interface is authoritative.

## Implemented interface

- Chat-first full-height page.
- Persistent conversation history in the left sidebar.
- Searchable chat list.
- New Chat flow that creates a thread on first message.
- Sticky footer composer with Enter-to-send and Shift+Enter for a new line.
- Existing model, goal, mode, connection, and local dataset context controls.
- Microgifter connection drawer with Sync Code connection, lifecycle state, entitlement, merchant/site counts, capabilities, heartbeat, lease refresh, and credential rotation.
- Explicit local-first independence messaging.
- Deterministic initial workspace refresh so existing chat history loads on first open.
- Draft and composer preservation during the Control Center's periodic system-health refresh.

## Validation checkpoints

- Coordinated Cloud Connector Contract run #255 passed on the Agent Chat branch, confirming the existing pairing node and exact cross-repository authority contract remained intact.
- Agent Chat dependency lock normalization run #2 passed and committed only the Windows-generated lockfile update.
- Agent Chat initial-load repair run #4 passed frontend syntax, frontend checks, and the production frontend build before committing.
- Agent Chat route-stability repair passed JavaScript syntax, frontend checks, and the production frontend build before committing as `3b09ee8387713e6a04e23d73b66fbedd65611f5f`.
- Production Quality run #776 passed dependency locks and isolated the permanent Agent Workspace module-loading requirement.
- Agent Workspace compatibility run #4 passed the permanent frontend validator and production frontend build, then committed the new chat/legacy coexistence boundary as `6a2fae175e3662563f66f1c29653f65b8f75e67f`.
- All temporary workflow helpers removed themselves; the permanent Production Quality workflow is restored unchanged.
- Full frontend, Rust, installer, security, and signed-update validation is running on the final clean branch.

## Permanent boundaries

- No new pairing node.
- No database migration.
- No change to the provider contract, entitlement trust, updater trust chain, or local-data ownership.
- HomeServer local operation remains independent of Microgifter connection state.

Do not merge without David Evans's explicit approval.
