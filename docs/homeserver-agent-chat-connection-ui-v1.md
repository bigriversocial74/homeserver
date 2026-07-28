# HomeServer Agent Chat and Connection Interface v1

Status: implementation in progress

## Scope

- Preserve the existing multi-site pairing node and Phase 6A service contract.
- Replace the current operationally dense Agent Workspace presentation with a chat-first HomeServer Agent page.
- Keep existing local thread and message persistence.
- Add a conversation sidebar with New Chat and chat-history navigation.
- Add a sticky footer composer with mode, model, goal, and bounded context controls.
- Add a Phase 6A Microgifter connection drawer using the existing local service endpoints.
- Keep local operation visibly available when Microgifter is offline, suspended, revoked, or unpaired.
- Preserve supervised approvals and existing Agent Workspace records through an expandable workspace drawer.

## Files

- Add `src/homeserver-agent-chat.js`.
- Add `src/homeserver-agent-chat.css`.
- Extend `src-tauri/src/cloud.rs` with Phase 6A local-service command bridges.
- Register the new commands in `src-tauri/src/lib.rs`.
- Replace the legacy Agent Workspace script entry in `index.html` while retaining the previous source for rollback.

## Validation

- Frontend syntax and production build.
- Rust formatting and locked Tauri compilation.
- Existing HomeServer Production Quality workflow.
- No change to the pairing node, provider contract, database migration, updater trust chain, or cloud authority boundaries.

Do not merge without David Evans's explicit approval.
