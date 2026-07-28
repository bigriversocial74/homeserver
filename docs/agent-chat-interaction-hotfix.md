# Agent Chat Interaction Hotfix

Status: validation in progress

## Reported regression

The installed Agent Chat interface rendered, but visible controls could become non-responsive after the page canvas was replaced. Reported symptoms included frozen sidebar navigation, dead Agent Chat header controls, and inactive chat-history and New Chat buttons.

## Repair

- Keep Control Center page navigation attached through one persistent capture-phase document handler.
- Keep Agent Chat click, submit, input, and keyboard controls attached through persistent delegated document handlers.
- Do not rely on listeners bound to individual elements that may be replaced by a page rerender.
- Preserve the existing pairing node, local chat persistence, Phase 6A provider endpoints, and local-first boundaries.

## Scope

Product files changed:

- `src/main.js`
- `src/homeserver-agent-chat.js`

No database migration is required.

The hotfix must pass HomeServer Production Quality and the Coordinated Cloud Connector Contract before merge. A new verified installer must then be generated for hands-on testing.
