# HomeServer Agent Chat and Connection Interface v1

Status: chat-first frontend committed; dependency lock normalization running

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

## Validation

- Frontend syntax and production build.
- Rust formatting and locked Tauri compilation.
- Existing HomeServer Production Quality workflow.
- No change to the pairing node, provider contract, database migration, updater trust chain, or cloud authority boundaries.

Do not merge without David Evans's explicit approval.
