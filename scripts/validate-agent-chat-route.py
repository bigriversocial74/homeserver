from pathlib import Path

root = Path(__file__).resolve().parents[1]
index = (root / "index.html").read_text(encoding="utf-8")
main = (root / "src" / "main.js").read_text(encoding="utf-8")
chat = (root / "src" / "homeserver-agent-chat.js").read_text(encoding="utf-8")
css = (root / "src" / "homeserver-agent-chat.css").read_text(encoding="utf-8")
legacy = (root / "src" / "agent-workspace.js").read_text(encoding="utf-8")
durable = (root / "src" / "durable-activity-ui.js").read_text(encoding="utf-8")
observer_modules = {
    "Operational Data": (root / "src" / "operational-data.js").read_text(encoding="utf-8"),
    "Review Intelligence": (root / "src" / "review-intelligence.js").read_text(encoding="utf-8"),
    "Cloud Connections": (root / "src" / "cloud-connections.js").read_text(encoding="utf-8"),
    "Ollama installer": (root / "src" / "ollama-install-assistant.js").read_text(encoding="utf-8"),
}

required_main = [
    '["agent", "HomeServer Agent", "integrations"]',
    'document.documentElement.classList.toggle("agent-chat-mode", activePage === "agent")',
    'app.innerHTML = `<div class="agent-chat-shell"',
    'window.dispatchEvent(new CustomEvent("homeserver-shell-health"',
    'window.dispatchEvent(new CustomEvent("homeserver:rendered"',
]
required_chat = [
    'data-homeserver-agent-host="true"',
    'host.innerHTML = renderPage()',
    'id="hs-chat-control-center"',
    'id="hs-chat-logo-home"',
    'window.addEventListener("homeserver-shell-health"',
    'function applyShellHealth(detail)',
    'runtime.classList.toggle("warn"',
    'Model runtime offline',
    'function scheduleMount(force = false)',
    'void refreshAll({ initial: true })',
    'const generation = ++refreshGeneration',
    'window.addEventListener("homeserver-agent-route", () => scheduleMount())',
]
required_css = [
    'html.agent-chat-mode',
    '.agent-chat-shell',
    '.hs-chat-main{position:relative',
    '.hs-chat-stream{position:absolute',
    'padding:34px max(30px,calc((100% - 980px)/2)) 230px',
    '.hs-chat-composer{position:absolute',
]
required_durable = [
    'window.addEventListener("homeserver:rendered", queueInject)',
    'const host = document.querySelector(".hs-chat-stream")',
    'if (!current && !host) return false',
    'current.outerHTML = markup',
    'host.insertAdjacentHTML("afterbegin", markup)',
    'loadError = String(error?.message || error || "activity history unavailable")',
    'if (!injected && isAgentPage() && attempt < 20)',
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
for value in required_durable:
    if value not in durable:
        raise SystemExit(f"Missing durable Agent activity lifecycle contract: {value}")

if '/src/agent-workspace.js' in index:
    raise SystemExit("Legacy Agent Workspace is still loaded beside Agent Chat")
if '/src/durable-activity-ui.js' not in index:
    raise SystemExit("Durable Agent activity UI is not loaded")
if 'const legacyAgentWorkspaceDisabled = true;' not in legacy:
    raise SystemExit("Legacy Agent Workspace is not deterministically disabled")
if 'MutationObserver' in legacy:
    raise SystemExit("Legacy Agent Workspace observer remains")
if 'MutationObserver' in durable:
    raise SystemExit("Durable Agent activity still races the Agent route through an app-wide observer")
if 'current.hasAttribute("data-durable-activity-card")' in durable:
    raise SystemExit("Durable Agent activity still refuses to repaint an existing card")

for name, source in observer_modules.items():
    if 'MutationObserver' in source:
        raise SystemExit(f"{name} still watches the complete application DOM")
    if 'homeserver:rendered' not in source:
        raise SystemExit(f"{name} is not bound to the explicit render lifecycle")

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
if 'window.addEventListener("hashchange"' in chat:
    raise SystemExit("Agent Chat still competes with the shell hash router")
if 'window.setTimeout(() => mount(true), 0)' in chat:
    raise SystemExit("Agent Chat still force-mounts through duplicate timer paths")
if 'event.target.closest("[data-page]")' in main:
    raise SystemExit("Competing delegated Control Center router remains")

print("HomeServer uses one coalesced Agent route lifecycle with a repaintable durable activity card and no duplicate app-wide observer network.")
