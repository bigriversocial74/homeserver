from pathlib import Path

root = Path(__file__).resolve().parents[1]
main = (root / "src" / "main.js").read_text(encoding="utf-8")
chat = (root / "src" / "homeserver-agent-chat.js").read_text(encoding="utf-8")
css = (root / "src" / "homeserver-agent-chat.css").read_text(encoding="utf-8")

required_main = [
    '["agent", "HomeServer Agent", "integrations"]',
    'document.documentElement.classList.toggle("agent-chat-mode", activePage === "agent")',
    'app.innerHTML = `<div class="agent-chat-shell"',
    'window.dispatchEvent(new CustomEvent("homeserver-shell-health"',
    'if (activePage === "agent") {',
]
required_chat = [
    'data-homeserver-agent-host="true"',
    'host.innerHTML = renderPage()',
    'id="hs-chat-control-center"',
    'window.addEventListener("homeserver-shell-health"',
    'function applyShellHealth(detail)',
    'runtime.classList.toggle("warn"',
    'Model runtime offline',
]
required_css = [
    'html.agent-chat-mode',
    '.agent-chat-shell',
    '.hs-chat-main{position:relative',
    '.hs-chat-stream{position:absolute',
    'padding:34px max(30px,calc((100% - 980px)/2)) 230px',
    '.hs-chat-composer{position:absolute',
]
for value in required_main:
    if value not in main:
        raise SystemExit(f"Missing resilient Agent Chat shell contract: {value}")
for value in required_chat:
    if value not in chat:
        raise SystemExit(f"Missing Agent Chat workspace contract: {value}")
for value in required_css:
    if value not in css:
        raise SystemExit(f"Missing Agent Chat layout contract: {value}")

for forbidden in [
    'MutationObserver',
    'function injectNavigation()',
    'function delegatedAgentClick(event)',
    'stopImmediatePropagation()',
]:
    if forbidden in chat:
        raise SystemExit(f"Competing Agent Chat lifecycle remains: {forbidden}")

if 'Model Center unavailable:' in main and 'activePage === "models"' not in main:
    raise SystemExit("Model Center failure is still global")
if 'if (isAgentPage()) mount(true);' in chat:
    raise SystemExit("Background shell health still remounts Agent Chat")
if 'event.target.closest("[data-page]")' in main:
    raise SystemExit("Competing delegated Control Center router remains")

print("Agent Chat is isolated, full-window, independently scrollable, and resilient to optional module failures.")
