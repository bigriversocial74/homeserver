# Agent Chat Interaction Hotfix

Status: observer-free runtime validated; logo dashboard link validation running

## Reported regressions

1. Agent Chat controls could become non-responsive after page replacement.
2. A later Model Center availability notification caused Agent Chat and navigation to freeze.
3. Agent Chat still appeared inside the Control Center shell instead of operating as a dedicated full-width chat workspace.
4. The redesigned build worked for several clicks, then froze after roughly 15–20 seconds.

## Confirmed causes

- The original Control Center and Agent Chat renderers competed for the same page canvas.
- The 30-second Control Center background refresh still replaced the entire `#app` while Agent Chat was active.
- Optional Model Center failure was promoted to a global notification and full shell render.
- The first lightweight health-event implementation still remounted Agent Chat.
- The legacy Agent Workspace remained separately loaded and could activate depending on independent module load order.
- Operational Data, Review Intelligence, Cloud Connections, and the Ollama installer still used app-wide or document-wide `MutationObserver` lifecycles.
- The Ollama installer uses a 20-second refresh window, matching the delayed freeze observed during hands-on testing.

## Final runtime repair

- Agent Chat remains a dedicated full-window application surface.
- The normal Control Center sidebar, topbar, page canvas, and footer are not rendered in Agent mode.
- Agent Chat owns its own chat-history sidebar and Control Center return action.
- The legacy Agent Workspace is no longer loaded at runtime and is deterministically disabled in source.
- All application-wide and document-wide frontend `MutationObserver` lifecycles were removed.
- Control Center now emits one explicit `homeserver:rendered` event after a completed render.
- Operational Data, Review Intelligence, Cloud Connections, and the Ollama installer respond only to explicit page render and hash events.
- Background Model Center and Ollama checks cannot replace, scan, or remount Agent Chat.
- Shell-health events update only the runtime status badge in place.
- The message canvas scrolls independently beneath the overlaid sticky footer composer.
- Existing pairing, chat persistence, Phase 6A endpoints, updater trust, and local-first authority remain unchanged.

## Navigation refinement

- The HomeServer logo in the Agent Chat sidebar returns directly to the dashboard.
- The existing Control Center return button remains available.
- Keyboard focus and hover states make the logo navigation discoverable and accessible.

## Permanent validation

The frontend validator now enforces:

- one authoritative Agent Chat runtime
- no legacy Agent Workspace script loading
- no app-wide observer network
- explicit `homeserver:rendered` lifecycle ownership
- no background Agent Chat remount
- optional-module error scoping
- full-window Agent Chat layout
- independent message scrolling
- sticky footer composer positioning
- Agent Chat logo dashboard navigation

## Product files

- `index.html`
- `src/main.js`
- `src/homeserver-agent-chat.js`
- `src/homeserver-agent-chat.css`
- `src/agent-workspace.js`
- `src/operational-data.js`
- `src/review-intelligence.js`
- `src/cloud-connections.js`
- `src/ollama-install-assistant.js`
- `scripts/validate-agent-workspace.py`
- `scripts/validate-agent-chat-route.py`
- `package.json`

No database migration is required.

The replacement installer must be generated from HomeServer Production Quality on the exact observer-free source and remain unmerged until the installed application stays responsive through repeated navigation and multiple 20- and 30-second background refresh cycles.