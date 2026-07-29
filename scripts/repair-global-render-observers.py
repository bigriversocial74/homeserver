from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, content: str) -> None:
    (ROOT / path).write_text(content, encoding="utf-8")


def replace_once(content: str, old: str, new: str, label: str) -> str:
    if old not in content:
        if new in content:
            return content
        raise SystemExit(f"Missing repair anchor: {label}")
    return content.replace(old, new, 1)

# The legacy workspace remains in source for contract/reference validation, but it is no longer
# loaded beside the authoritative Agent Chat runtime.
index = read("index.html")
index = replace_once(
    index,
    '    <script type="module" src="/src/agent-workspace.js"></script>\n',
    "",
    "legacy Agent Workspace runtime script",
)
write("index.html", index)

# Control Center owns rendering and emits one explicit event after every completed render.
main = read("src/main.js")
main = replace_once(
    main,
    '    window.dispatchEvent(new CustomEvent("homeserver-agent-route"));\n    return;\n',
    '    window.dispatchEvent(new CustomEvent("homeserver-agent-route"));\n'
    '    window.dispatchEvent(new CustomEvent("homeserver:rendered", { detail: { page: activePage } }));\n'
    '    return;\n',
    "Agent Chat render event",
)
main = replace_once(
    main,
    '  bindEvents();\n}\n\nfunction bindEvents()',
    '  bindEvents();\n'
    '  window.dispatchEvent(new CustomEvent("homeserver:rendered", { detail: { page: activePage } }));\n'
    '}\n\nfunction bindEvents()',
    "Control Center render event",
)
write("src/main.js", main)

# The legacy implementation cannot activate through independent module ordering.
legacy = read("src/agent-workspace.js")
legacy = replace_once(
    legacy,
    'const legacyAgentWorkspaceDisabled = Boolean(window.__HOMESERVER_AGENT_CHAT_V1__);',
    'const legacyAgentWorkspaceDisabled = true;',
    "legacy workspace disable flag",
)
legacy_start = 'if (!legacyAgentWorkspaceDisabled) {\n'
if legacy_start in legacy:
    legacy = legacy.split(legacy_start, 1)[0].rstrip() + (
        '\n\n// The legacy Agent Workspace remains source-compatible for validation and rollback reference,\n'
        '// but the HomeServer Agent Chat module is the only runtime owner of the agent route.\n'
        'void legacyAgentWorkspaceDisabled;\n'
    )
write("src/agent-workspace.js", legacy)

operational = read("src/operational-data.js")
operational = replace_once(
    operational,
    '''const app = document.querySelector("#app");
if (app) {
  const observer = new MutationObserver(() => {
    injectNavigation();
    if (isOperationalPage()) mount(false);
  });
  observer.observe(app, { childList: true, subtree: true });
}

window.addEventListener("hashchange", () => window.setTimeout(() => mount(true), 0));
window.addEventListener("DOMContentLoaded", () => {
  if (initialized) return;
  initialized = true;
  window.setTimeout(() => mount(true), 0);
});
''',
    '''window.addEventListener("homeserver:rendered", () => window.setTimeout(() => mount(false), 0));
window.addEventListener("hashchange", () => window.setTimeout(() => mount(true), 0));
window.addEventListener("DOMContentLoaded", () => {
  if (initialized) return;
  initialized = true;
  window.setTimeout(() => mount(true), 0);
});
''',
    "Operational Data observer lifecycle",
)
write("src/operational-data.js", operational)

review = read("src/review-intelligence.js")
review = replace_once(
    review,
    '''window.addEventListener("hashchange", () => mount(true));
window.addEventListener("homeserver:rendered", () => mount());

if (!initialized) {
  initialized = true;
  const observer = new MutationObserver(() => mount());
  observer.observe(document.documentElement, { childList: true, subtree: true });
  window.setTimeout(() => mount(), 0);
}
''',
    '''window.addEventListener("hashchange", () => window.setTimeout(() => mount(true), 0));
window.addEventListener("homeserver:rendered", () => window.setTimeout(() => mount(false), 0));

if (!initialized) {
  initialized = true;
  window.setTimeout(() => mount(false), 0);
}
''',
    "Review Intelligence observer lifecycle",
)
write("src/review-intelligence.js", review)

cloud = read("src/cloud-connections.js")
cloud = replace_once(
    cloud,
    '''const app = document.querySelector("#app");
if (app) {
  const observer = new MutationObserver(() => mount(false));
  observer.observe(app, { childList: true, subtree: true });
}
window.addEventListener("hashchange", () => window.setTimeout(() => mount(false), 0));
window.addEventListener("DOMContentLoaded", () => window.setTimeout(() => mount(false), 0));
''',
    '''window.addEventListener("homeserver:rendered", () => window.setTimeout(() => mount(false), 0));
window.addEventListener("hashchange", () => window.setTimeout(() => mount(false), 0));
window.addEventListener("DOMContentLoaded", () => window.setTimeout(() => mount(false), 0));
''',
    "Cloud Connections observer lifecycle",
)
write("src/cloud-connections.js", cloud)

ollama = read("src/ollama-install-assistant.js")
ollama = replace_once(
    ollama,
    '''const app = document.querySelector("#app");
if (app) {
  new MutationObserver(queueMount).observe(app, { childList: true, subtree: true });
}
window.addEventListener("hashchange", queueMount);
queueMount();
''',
    '''window.addEventListener("homeserver:rendered", queueMount);
window.addEventListener("hashchange", queueMount);
queueMount();
''',
    "Ollama assistant observer lifecycle",
)
write("src/ollama-install-assistant.js", ollama)

workspace_validator = read("scripts/validate-agent-workspace.py")
workspace_validator = replace_once(
    workspace_validator,
    '''require(
    "index.html",
    "/src/agent-workspace.js",
    "Agent Workspace frontend module is not loaded",
)
''',
    '''require(
    "index.html",
    "/src/homeserver-agent-chat.js",
    "Authoritative HomeServer Agent Chat frontend module is not loaded",
)
''',
    "Agent Workspace runtime validation",
)
write("scripts/validate-agent-workspace.py", workspace_validator)

route_validator = '''from pathlib import Path

root = Path(__file__).resolve().parents[1]
index = (root / "index.html").read_text(encoding="utf-8")
main = (root / "src" / "main.js").read_text(encoding="utf-8")
chat = (root / "src" / "homeserver-agent-chat.js").read_text(encoding="utf-8")
css = (root / "src" / "homeserver-agent-chat.css").read_text(encoding="utf-8")
legacy = (root / "src" / "agent-workspace.js").read_text(encoding="utf-8")
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

if '/src/agent-workspace.js' in index:
    raise SystemExit("Legacy Agent Workspace is still loaded beside Agent Chat")
if 'const legacyAgentWorkspaceDisabled = true;' not in legacy:
    raise SystemExit("Legacy Agent Workspace is not deterministically disabled")
if 'MutationObserver' in legacy:
    raise SystemExit("Legacy Agent Workspace observer remains")

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
if 'event.target.closest("[data-page]")' in main:
    raise SystemExit("Competing delegated Control Center router remains")

print("HomeServer uses one explicit render lifecycle with no app-wide observer network.")
'''
write("scripts/validate-agent-chat-route.py", route_validator)

print("Removed delayed global observer network and legacy Agent Workspace runtime race.")
