from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
main_path = ROOT / "src" / "main.js"
chat_path = ROOT / "src" / "homeserver-agent-chat.js"
package_path = ROOT / "package.json"
validator_path = ROOT / "scripts" / "validate-agent-chat-route.py"

main = main_path.read_text(encoding="utf-8")
chat = chat_path.read_text(encoding="utf-8")
package = package_path.read_text(encoding="utf-8")

# Make Agent Chat a first-class Control Center route.
agent_page = '  ["agent", "HomeServer Agent", "integrations"],\n'
if agent_page not in main:
    anchor = '  ["dashboard", "Dashboard", "dashboard"],\n'
    if anchor not in main:
        raise SystemExit("Control Center page registry anchor not found")
    main = main.replace(anchor, anchor + agent_page, 1)

agent_case = '    case "agent": return `<div class="homeserver-agent-route-host" data-homeserver-agent-host="true"></div>`;\n'
if agent_case not in main:
    anchor = '    case "system": return renderSystem();\n'
    if anchor not in main:
        raise SystemExit("Control Center route switch anchor not found")
    main = main.replace(anchor, anchor + agent_case, 1)

main = main.replace('  if (window.location.hash === "#agent" && app.querySelector(\'[data-homeserver-chat-mounted="true"]\')) return;\n', "")

render_anchor = '  bindEvents();\n}\n\nfunction bindEvents()'
render_replacement = '  bindEvents();\n  if (activePage === "agent") {\n    window.dispatchEvent(new CustomEvent("homeserver-agent-route"));\n  }\n}\n\nfunction bindEvents()'
if render_replacement not in main:
    if render_anchor not in main:
        raise SystemExit("Control Center render lifecycle anchor not found")
    main = main.replace(render_anchor, render_replacement, 1)

main, delegated_count = re.subn(
    r'\ndocument\.addEventListener\("click", \(event\) => \{\n'
    r'  if \(!\(event\.target instanceof Element\)\) return;\n'
    r'  const control = event\.target\.closest\("\[data-page\]"\);\n'
    r'  if \(!control\) return;\n'
    r'  event\.preventDefault\(\);\n'
    r'  event\.stopImmediatePropagation\(\);\n'
    r'  navigate\(control\.dataset\.page\);\n'
    r'\}, true\);\n',
    "\n",
    main,
    count=1,
)
if delegated_count == 0 and 'event.target.closest("[data-page]")' in main:
    raise SystemExit("Unable to remove competing delegated Control Center router")

# Remove the injected navigation and app-wide mutation observer from Agent Chat.
chat = chat.replace("let delegatedEventsBound = false;\n", "")
chat, inject_count = re.subn(
    r'\nfunction injectNavigation\(\) \{.*?\n\}\n\nfunction renderThreadList\(\)',
    '\nfunction renderThreadList()',
    chat,
    count=1,
    flags=re.S,
)
if inject_count == 0 and "function injectNavigation()" in chat:
    raise SystemExit("Unable to remove injected Agent Chat navigation")

mount_function = '''function mount(force = false) {
  if (!isAgentPage()) return;
  const host = document.querySelector('[data-homeserver-agent-host="true"]');
  if (!host) return;
  if (!force && host.querySelector('[data-homeserver-chat-mounted="true"]')) return;
  host.innerHTML = renderPage();
  bindEvents();
  if (!initialized && !loading) {
    initialized = true;
    void refreshAll();
  }
  window.setTimeout(() => {
    const stream = document.querySelector("#hs-chat-stream");
    if (stream) stream.scrollTop = stream.scrollHeight;
    autoSizeComposer();
  }, 0);
}

function bindEvents()'''
chat, mount_count = re.subn(
    r'function mount\(force = false\) \{.*?\n\}\n\nfunction bindEvents\(\)',
    mount_function,
    chat,
    count=1,
    flags=re.S,
)
if mount_count != 1:
    raise SystemExit("Unable to replace Agent Chat mount lifecycle")

chat, delegated_chat_count = re.subn(
    r'\nfunction delegatedAgentClick\(event\) \{.*?\nensureDelegatedEvents\(\);\n',
    "\n",
    chat,
    count=1,
    flags=re.S,
)
if delegated_chat_count == 0 and "function delegatedAgentClick(event)" in chat:
    raise SystemExit("Unable to remove competing delegated Agent Chat router")

observer_anchor = '\nconst app = document.querySelector("#app");\n'
if observer_anchor not in chat:
    raise SystemExit("Unable to locate Agent Chat app observer lifecycle")
chat = chat.split(observer_anchor, 1)[0].rstrip() + (
    '\n\nwindow.addEventListener("homeserver-agent-route", () => window.setTimeout(() => mount(true), 0));\n'
    'window.addEventListener("hashchange", () => window.setTimeout(() => mount(true), 0));\n'
    'window.setTimeout(() => mount(true), 0);\n'
)

validator = '''from pathlib import Path

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
'''
validator_path.write_text(validator, encoding="utf-8")

if "node --check src/homeserver-agent-chat.js" not in package:
    package = package.replace(
        '"check:frontend": "node --check src/main.js && ',
        '"check:frontend": "node --check src/main.js && node --check src/homeserver-agent-chat.js && ',
        1,
    )
if "python scripts/validate-agent-chat-route.py" not in package:
    package = package.replace(
        'python scripts/validate-agent-workspace.py && ',
        'python scripts/validate-agent-workspace.py && python scripts/validate-agent-chat-route.py && ',
        1,
    )

main_path.write_text(main, encoding="utf-8")
chat_path.write_text(chat, encoding="utf-8")
package_path.write_text(package, encoding="utf-8")
print("Rebuilt Agent Chat as a deterministic Control Center route.")
