# Agent Chat Interaction Hotfix

Status: final single-router validation running

## Reported regression

The installed Agent Chat interface rendered, but visible controls became non-responsive after the first navigation or chat click. The first interaction worked, then sidebar, header, chat-history, and New Chat controls froze.

## Confirmed root cause

The Control Center and Agent Chat both attempted to own and replace the same `.page-canvas`. Agent Chat also watched the complete application subtree with a `MutationObserver`. After the first route change, the competing render lifecycles could overwrite each other and invalidate the next interaction.

## Completed repair

- Registered HomeServer Agent as a normal Control Center route.
- Gave the Control Center sole ownership of the application shell and page canvas.
- Gave Agent Chat ownership only of its dedicated route host.
- Removed the app-wide Agent Chat `MutationObserver`.
- Removed competing capture-phase delegated routers and `stopImmediatePropagation()` navigation.
- Restored normal page-specific event binding after each deterministic render.
- Added a permanent Agent Chat route-lifecycle validator to the frontend checks.
- Preserved the existing pairing node, local chat persistence, Phase 6A provider endpoints, and local-first boundaries.

## Scope

Product files changed:

- `src/main.js`
- `src/homeserver-agent-chat.js`
- `package.json`
- `scripts/validate-agent-chat-route.py`

No database migration is required.

The replacement installer must come from the final HomeServer Production Quality run on this exact source and remain unmerged until repeated hands-on navigation succeeds.
