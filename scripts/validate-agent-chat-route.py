from pathlib import Path

root = Path(__file__).resolve().parents[1]
index = (root / "index.html").read_text(encoding="utf-8")
main = (root / "src" / "main.js").read_text(encoding="utf-8")
chat = (root / "src" / "homeserver-agent-chat.js").read_text(encoding="utf-8")
css = (root / "src" / "homeserver-agent-chat.css").read_text(encoding="utf-8")
shared = (root / "src" / "shared-sidebar.js").read_text(encoding="utf-8")
shared_css = (root / "src" / "shared-sidebar.css").read_text(encoding="utf-8")
legacy = (root / "src" / "agent-workspace.js").read_text(encoding="utf-8")
durable = (root / "src" / "durable-activity-ui.js").read_text(encoding="utf-8")
activity = (root / "crates" / "homeserver-service" / "src" / "activity.rs").read_text(encoding="utf-8")
tauri_agent = (root / "src-tauri" / "src" / "agent.rs").read_text(encoding="utf-8")
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
required_shared = [
    'import { logoMark } from "./icons.js"',
    'const AGENT_SIDEBAR_MODE = "chat-only"',
    'const PRIMARY_NAV_ORDER = ["agent", "home", "dashboard", "knowledge", "apps", "integrations"]',
    'const SYSTEM_NAV_ORDER = ["models", "backups", "sync", "settings", "system"]',
    'return window.location.hash.replace("#", "") || "dashboard"',
    'data-shared-chat-rename',
    'data-shared-chat-delete',
    'action: "rename_thread"',
    'action: "delete_thread"',
    'invoke("homeserver_create_agent_goal", { request })',
    'function removeSidebarFooters()',
    '.app-sidebar > .server-card',
    '.app-sidebar > .sidebar-state',
    '.hs-chat-sidebar > .hs-chat-provider-summary',
    '.hs-chat-sidebar > .hs-chat-sidebar-footer',
    'function createNavigationGroup(',
    'function reorderMainSidebarNavigation()',
    '"primary-nav-main"',
    '"primary-nav-system"',
    'nav.replaceChildren(primaryGroup, systemGroup)',
    'nav.dataset.navigationOrder = "primary-system-v1"',
    'function removeUnexpectedControlCenterUi()',
    '.agent-chat-shell > .app-sidebar:not(.hs-chat-sidebar)',
    'sidebar.dataset.agentSidebarMode = AGENT_SIDEBAR_MODE',
    'sidebar.replaceChildren(createBrand(), chatSection)',
    'reorderMainSidebarNavigation()',
    'document.querySelector(\'[data-homeserver-agent-host="true"]\')',
    'observer.observe(host, { childList: true, subtree: true })',
    'window.addEventListener("homeserver:rendered", scheduleDecorate)',
    'mode: AGENT_SIDEBAR_MODE',
    'footerSections: "removed"',
    'primaryNavigation: [...PRIMARY_NAV_ORDER]',
    'systemNavigation: [...SYSTEM_NAV_ORDER]',
    'window.__HOMESERVER_AGENT_SIDEBAR_CHAT_ONLY_V2__ = true',
]
required_shared_css = [
    '.app-sidebar>.server-card',
    '.app-sidebar>.sidebar-state',
    '.hs-chat-sidebar>.hs-chat-provider-summary',
    '.hs-chat-sidebar>.hs-chat-sidebar-footer',
    'display:none!important',
    '.app-sidebar>.primary-nav{display:flex;flex:1;min-height:0;flex-direction:column',
    '.primary-nav-main,.primary-nav-system{display:grid;gap:4px}',
    '.primary-nav-system{margin-top:auto;padding-top:14px;border-top:1px solid #e5e7eb}',
    '.shared-agent-sidebar.app-sidebar',
    '.shared-chat-row',
    '.shared-chat-actions',
]
required_activity = [
    '.route("/v1/agent/threads/rename", post(rename_agent_thread))',
    '.route("/v1/agent/threads/delete", post(delete_agent_thread))',
    'fn rename_thread_record(',
    'fn delete_thread_record(',
    'request.confirmation == "DELETE"',
    "ON DELETE CASCADE",
]
required_tauri_agent = [
    'Some("rename_thread") => post_json("/v1/agent/threads/rename", &request).await',
    'Some("delete_thread") => post_json("/v1/agent/threads/delete", &request).await',
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
for value in required_shared:
    if value not in shared:
        raise SystemExit(f"Missing ordered, footer-free shared sidebar contract: {value}")
for value in required_shared_css:
    if value not in shared_css:
        raise SystemExit(f"Missing ordered, footer-free shared sidebar style contract: {value}")
for value in required_activity:
    if value not in activity:
        raise SystemExit(f"Missing Agent chat service contract: {value}")
for value in required_tauri_agent:
    if value not in tauri_agent:
        raise SystemExit(f"Missing Agent chat desktop bridge contract: {value}")

if '/src/agent-workspace.js' in index:
    raise SystemExit("Legacy Agent Workspace is still loaded beside Agent Chat")
if '/src/durable-activity-ui.js' not in index:
    raise SystemExit("Durable Agent activity UI is not loaded")
if '/src/shared-sidebar.js' not in index:
    raise SystemExit("Shared HomeServer sidebar is not loaded")
if 'const legacyAgentWorkspaceDisabled = true;' not in legacy:
    raise SystemExit("Legacy Agent Workspace is not deterministically disabled")
if 'MutationObserver' in legacy:
    raise SystemExit("Legacy Agent Workspace observer remains")
if 'MutationObserver' in durable:
    raise SystemExit("Durable Agent activity still races the Agent route through an app-wide observer")
if 'current.hasAttribute("data-durable-activity-card")' in durable:
    raise SystemExit("Durable Agent activity still refuses to repaint an existing card")
if 'observer.observe(document.body' in shared or 'observer.observe(document.querySelector("#app")' in shared:
    raise SystemExit("Shared sidebar watches the complete application DOM instead of the Agent host")

for forbidden in [
    'const MENU_ITEMS',
    'function reorderPrimaryNavigation()',
    'function createNavigation()',
    'function createServerCard()',
    'function createSidebarState()',
    'data-shared-page',
    'menu: MENU_ITEMS',
    'shared-sidebar-lower',
    'const provider = sidebar.querySelector("#hs-chat-provider-summary")',
]:
    if forbidden in shared:
        raise SystemExit(f"Shared sidebar still composes removed UI: {forbidden}")

for forbidden in [
    '.shared-sidebar-navigation',
    '.shared-sidebar-server-card',
    '.shared-sidebar-lower{display:grid',
    '.shared-agent-sidebar .sidebar-state{margin-top:0}',
]:
    if forbidden in shared_css:
        raise SystemExit(f"Removed sidebar layout style remains: {forbidden}")

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

print("HomeServer renders Agent, Home, Dashboard, Knowledge Vault, Apps, and Integrations in the primary sidebar group, with Model Center, Backups, Sync Cloud, Settings, and System pinned to the bottom.")
