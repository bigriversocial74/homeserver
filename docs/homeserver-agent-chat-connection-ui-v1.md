# HomeServer Agent Chat and Connection Interface v1

Status: implementation complete; active-chat route stability repair running

## Scope

- Preserve the existing multi-site pairing node and Phase 6A service contract.
- Replace the current operationally dense Agent Workspace presentation with a chat-first HomeServer Agent page.
- Keep existing local thread and message persistence.
- Add a conversation sidebar with New Chat and chat-history navigation.
- Add a sticky footer composer with mode, model, goal, and bounded context controls.
- Add a Phase 6A Microgifter connection drawer using the existing local service endpoints.
- Keep local operation visibly available when Microgifter is offline, suspended, revoked, or unpaired.
- Preserve the existing supervised Agent Workspace backend and records for future drawer expansion.

## Files

- Add `src/homeserver-agent-chat.js`.
- Add `src/homeserver-agent-chat.css`.
- Extend `src-tauri/src/cloud.rs` with Phase 6A local-service command bridges.
- Register the new commands in `src-tauri/src/lib.rs`.
- Update `src/main.js` so background system refreshes do not replace an active Agent Chat canvas.
- Replace the legacy Agent Workspace script entry in `index.html` while retaining the previous source for rollback.

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

- Coordinated Cloud Connector Contract run #255 passed on the clean Agent Chat branch, confirming the existing pairing node and exact cross-repository authority contract remained intact.
- Agent Chat dependency lock normalization run #2 passed and committed only the Windows-generated lockfile update.
- Agent Chat initial-load repair run #4 passed frontend syntax, frontend checks, and the production frontend build before committing.
- Temporary workflow helpers are self-removing; the permanent Production Quality workflow remains the final validation authority.
- Full frontend, Rust, installer, security, and signed-update validation will run on the final clean branch.

## Permanent boundaries

- No new pairing node.
- No database migration.
- No change to the provider contract, entitlement trust, updater trust chain, or local-data ownership.
- HomeServer local operation remains independent of Microgifter connection state.

Do not merge without David Evans's explicit approval.
