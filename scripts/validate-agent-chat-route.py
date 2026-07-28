from pathlib import Path

root = Path(__file__).resolve().parents[1]
main = (root / "src" / "main.js").read_text(encoding="utf-8")
chat = (root / "src" / "homeserver-agent-chat.js").read_text(encoding="utf-8")

required_main = [
    '["agent", "HomeServer Agent", "integrations"]',
    'case "agent": return `<div class="homeserver-agent-route-host" data-homeserver-agent-host="true"></div>`;',
    'window.dispatchEvent(new CustomEvent("homeserver-agent-route"))',
]
required_chat = [
    'data-homeserver-agent-host="true"',
    'host.innerHTML = renderPage()',
    'bindEvents()',
    'window.addEventListener("homeserver-agent-route"',
]
for value in required_main:
    if value not in main:
        raise SystemExit(f"Missing Control Center Agent Chat route contract: {value}")
for value in required_chat:
    if value not in chat:
        raise SystemExit(f"Missing Agent Chat route lifecycle contract: {value}")

for forbidden in [
    'MutationObserver',
    'function injectNavigation()',
    'function delegatedAgentClick(event)',
    'stopImmediatePropagation()',
]:
    if forbidden in chat:
        raise SystemExit(f"Competing Agent Chat lifecycle remains: {forbidden}")

if 'event.target.closest("[data-page]")' in main:
    raise SystemExit("Competing delegated Control Center router remains")
if 'window.location.hash === "#agent" && app.querySelector' in main:
    raise SystemExit("Legacy dual-canvas Agent Chat render guard remains")

print("Agent Chat uses one deterministic Control Center route lifecycle.")
