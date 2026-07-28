# Agent Chat Interaction Hotfix

Status: deterministic route lifecycle repair running

## Reported regression

The installed Agent Chat interface rendered, but visible controls became non-responsive after the first navigation or chat click. The first interaction worked, then sidebar, header, chat-history, and New Chat controls froze.

## Root cause under repair

The Control Center and Agent Chat were both attempting to own and replace the same `.page-canvas`. Agent Chat also watched the complete application subtree with a `MutationObserver`. After the first route change, the two render lifecycles could overwrite each other and invalidate the next interaction.

## Repair

- Register HomeServer Agent as a normal Control Center route.
- Give the Control Center sole ownership of the application shell and page canvas.
- Give Agent Chat ownership only of its dedicated route host.
- Remove the app-wide Agent Chat `MutationObserver`.
- Remove competing capture-phase delegated routers and `stopImmediatePropagation()` navigation.
- Rebind page-specific controls after each deterministic render.
- Preserve the existing pairing node, local chat persistence, Phase 6A provider endpoints, and local-first boundaries.

## Scope

Product files changed:

- `src/main.js`
- `src/homeserver-agent-chat.js`
- `package.json`
- `scripts/validate-agent-chat-route.py`

No database migration is required.

The repair must pass frontend syntax, permanent frontend validation, production frontend build, HomeServer Production Quality, and the Coordinated Cloud Connector Contract before a replacement installer is provided.
