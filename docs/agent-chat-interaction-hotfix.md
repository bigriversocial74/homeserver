# Agent Chat Interaction Hotfix

Status: final resilience installer validation running

## Reported regressions

1. Agent Chat controls could become non-responsive after page replacement.
2. A later Model Center availability notification caused Agent Chat and navigation to freeze.
3. Agent Chat still appeared inside the Control Center shell instead of operating as a dedicated full-width chat workspace.

## Confirmed causes

- The original Control Center and Agent Chat renderers competed for the same page canvas.
- Even after the single-router correction, the 30-second Control Center background refresh still replaced the entire `#app` while Agent Chat was active.
- Optional Model Center failure was promoted to a global notification and full shell render instead of remaining a non-blocking module-health condition.
- The first lightweight health-event implementation still remounted Agent Chat; that final remount path has now been removed.

## Completed resilience repair

- Agent Chat renders as a dedicated full-window application surface.
- The normal Control Center sidebar, top bar, page canvas, and footer are not rendered in Agent mode.
- Agent Chat owns its own chat-history sidebar and includes a Control Center return action.
- Background Control Center refreshes never replace or remount Agent Chat.
- Optional Model Center, semantic-index, cloud, and MCP health failures are scoped to their relevant pages.
- Agent Chat receives lightweight shell-health events.
- Shell-health events update only the runtime status badge in place.
- Model runtime degradation appears as a non-blocking header status.
- The message canvas scrolls independently beneath an overlaid sticky footer composer.
- The composer, context controls, mode, goal, model, and connection controls remain visible at the bottom.
- The permanent Agent Chat validator enforces shell isolation, no background remount, optional-module resilience, full-window layout, independent scrolling, and footer composer positioning.
- Existing pairing, chat persistence, Phase 6A provider endpoints, updater trust, and local-first authority remain unchanged.

## Product files

- `src/main.js`
- `src/homeserver-agent-chat.js`
- `src/homeserver-agent-chat.css`
- `package.json`
- `scripts/validate-agent-chat-route.py`

No database migration is required.

The replacement installer must come from HomeServer Production Quality on the exact final source and remain unmerged until repeated navigation, Model Center degradation, and Agent Chat layout tests pass on the installed application.
