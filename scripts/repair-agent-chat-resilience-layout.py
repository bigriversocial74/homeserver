from pathlib import Path

root = Path(__file__).resolve().parents[1]
main_path = root / "src" / "main.js"
chat_path = root / "src" / "homeserver-agent-chat.js"
css_path = root / "src" / "homeserver-agent-chat.css"
validator_path = root / "scripts" / "validate-agent-chat-route.py"

main = main_path.read_text(encoding="utf-8")
chat = chat_path.read_text(encoding="utf-8")
css = css_path.read_text(encoding="utf-8")

old_render = '''function render() {
  const restorePending = Boolean(backupCatalog?.restore_pending || statusSnapshot?.restore_pending);
  const prefs = loadPreferences();
  document.documentElement.classList.toggle("compact-ui", Boolean(prefs.compact));
  app.innerHTML = `<div class="desktop-shell">${renderSidebar()}<main class="app-main">${renderTopbar()}<section class="page-canvas">${notice ? `<div class="notice ${notice.kind}">${escapeHtml(notice.message)}</div>` : ""}${restorePending ? `<div class="notice warning"><strong>Restore staged.</strong> Restart the HomeServer service or Windows to apply the verified database. The current database is preserved for rollback.</div>` : ""}${renderCurrentPage()}<footer class="app-footer"><span>Local API: ${escapeHtml(statusSnapshot?.api_url || "http://127.0.0.1:47831")}</span><span>Updated: ${escapeHtml(formatDate(statusSnapshot?.last_updated_utc))}</span></footer></section></main></div>`;
  bindEvents();
  if (activePage === "agent") {
    window.dispatchEvent(new CustomEvent("homeserver-agent-route"));
  }
}'''

new_render = '''function render() {
  const restorePending = Boolean(backupCatalog?.restore_pending || statusSnapshot?.restore_pending);
  const prefs = loadPreferences();
  document.documentElement.classList.toggle("compact-ui", Boolean(prefs.compact));
  document.documentElement.classList.toggle("agent-chat-mode", activePage === "agent");
  if (activePage === "agent") {
    app.innerHTML = `<div class="agent-chat-shell"><div class="homeserver-agent-route-host" data-homeserver-agent-host="true"></div></div>`;
    window.dispatchEvent(new CustomEvent("homeserver-agent-route"));
    return;
  }
  app.innerHTML = `<div class="desktop-shell">${renderSidebar()}<main class="app-main">${renderTopbar()}<section class="page-canvas">${notice ? `<div class="notice ${notice.kind}">${escapeHtml(notice.message)}</div>` : ""}${restorePending ? `<div class="notice warning"><strong>Restore staged.</strong> Restart the HomeServer service or Windows to apply the verified database. The current database is preserved for rollback.</div>` : ""}${renderCurrentPage()}<footer class="app-footer"><span>Local API: ${escapeHtml(statusSnapshot?.api_url || "http://127.0.0.1:47831")}</span><span>Updated: ${escapeHtml(formatDate(statusSnapshot?.last_updated_utc))}</span></footer></section></main></div>`;
  bindEvents();
}'''

if old_render not in main:
    raise SystemExit("Main render function anchor not found")
main = main.replace(old_render, new_render, 1)

start = main.index("async function loadAll(clearNotice = true) {")
end = main.index('\n\nwindow.addEventListener("hashchange"', start)
new_load = '''async function loadAll(clearNotice = true) {
  if (clearNotice && activePage !== "agent") notice = null;
  const results = await Promise.allSettled([
    invoke("homeserver_status"),
    invoke("homeserver_cloud_status"),
    invoke("homeserver_backups"),
    invoke("homeserver_updates"),
    invoke("homeserver_vault"),
    invoke("homeserver_semantic_vault"),
    invoke("homeserver_models"),
    invoke("homeserver_mcp"),
    invoke("homeserver_mcp_bridge_path"),
  ]);
  if (results[0].status === "rejected") {
    statusSnapshot = null;
    if (activePage === "agent") {
      window.dispatchEvent(new CustomEvent("homeserver-shell-health", { detail: { service: "offline", models: "unknown" } }));
      return;
    }
    cloudSnapshot = null;
    backupCatalog = null;
    updateStatus = null;
    vaultSnapshot = null;
    semanticSnapshot = null;
    modelSnapshot = null;
    mcpSnapshot = null;
    mcpBridgePath = null;
    notice = { kind: "warning", message: `HomeServer service unavailable: ${String(results[0].reason)}` };
    render();
    return;
  }
  statusSnapshot = results[0].value;
  cloudSnapshot = results[1].status === "fulfilled" ? results[1].value : cloudSnapshot || { state: "degraded", scopes: [], pending_sync: 0, last_error: "cloud_status_unavailable" };
  backupCatalog = results[2].status === "fulfilled" ? results[2].value : backupCatalog;
  updateStatus = results[3].status === "fulfilled" ? results[3].value : updateStatus;
  vaultSnapshot = results[4].status === "fulfilled" ? results[4].value : vaultSnapshot;
  semanticSnapshot = results[5].status === "fulfilled" ? results[5].value : semanticSnapshot;
  modelSnapshot = results[6].status === "fulfilled" ? results[6].value : modelSnapshot;
  mcpSnapshot = results[7].status === "fulfilled" ? results[7].value : mcpSnapshot;
  mcpBridgePath = results[8].status === "fulfilled" ? results[8].value : mcpBridgePath;

  const health = {
    service: "online",
    cloud: results[1].status === "fulfilled" ? "online" : "degraded",
    semantic: results[5].status === "fulfilled" ? "online" : "degraded",
    models: results[6].status === "fulfilled" ? "online" : "degraded",
    mcp: results[7].status === "fulfilled" ? "online" : "degraded",
  };
  if (activePage === "agent") {
    window.dispatchEvent(new CustomEvent("homeserver-shell-health", { detail: health }));
    return;
  }

  if (!notice && results[1].status === "rejected") notice = { kind: "warning", message: `Cloud connector unavailable: ${String(results[1].reason)}` };
  if (!notice && results[5].status === "rejected" && activePage === "knowledge") notice = { kind: "warning", message: `Semantic Knowledge Vault unavailable: ${String(results[5].reason)}` };
  if (!notice && results[6].status === "rejected" && activePage === "models") notice = { kind: "warning", message: `Model Center unavailable: ${String(results[6].reason)}` };
  if (!notice && results[7].status === "rejected" && activePage === "integrations") notice = { kind: "warning", message: `Local MCP runtime unavailable: ${String(results[7].reason)}` };
  render();
}'''
main = main[:start] + new_load + main[end:]

chat = chat.replace("let initialized = false;\n", "let initialized = false;\nlet shellHealth = { service: \"online\", models: \"unknown\" };\n", 1)

old_sidebar_end = '''      <button type="button" class="hs-chat-provider-summary" id="hs-chat-provider-summary"><span class="hs-provider-state ${providerTone(state)}"><i></i>${escapeHtml(humanize(state))}</span><strong>Microgifter</strong><small>${providerConnections().length} connection${providerConnections().length === 1 ? "" : "s"}</small></button>
    </aside>'''
new_sidebar_end = '''      <button type="button" class="hs-chat-provider-summary" id="hs-chat-provider-summary"><span class="hs-provider-state ${providerTone(state)}"><i></i>${escapeHtml(humanize(state))}</span><strong>Microgifter</strong><small>${providerConnections().length} connection${providerConnections().length === 1 ? "" : "s"}</small></button>
      <div class="hs-chat-sidebar-footer">
        <button type="button" id="hs-chat-control-center"><span>←</span><div><strong>Control Center</strong><small>Return to dashboard</small></div></button>
      </div>
    </aside>'''
if old_sidebar_end not in chat:
    raise SystemExit("Agent Chat sidebar anchor not found")
chat = chat.replace(old_sidebar_end, new_sidebar_end, 1)

old_header = '''      <header class="hs-chat-header"><div><strong>${escapeHtml(thread?.title || "New chat")}</strong><span>${thread ? `Updated ${escapeHtml(relativeDate(thread.updated_at_utc))}` : "Private local conversation"}</span></div><div class="hs-chat-header-actions"><span class="hs-runtime-state">${escapeHtml(humanize(workspace?.model_runtime_state || "loading"))}</span><button type="button" id="hs-chat-refresh" ${loading ? "disabled" : ""}>↻</button><button type="button" id="hs-chat-open-connections">Connections</button></div></header>'''
new_header = '''      <header class="hs-chat-header"><div><strong>${escapeHtml(thread?.title || "New chat")}</strong><span>${thread ? `Updated ${escapeHtml(relativeDate(thread.updated_at_utc))}` : "Private local conversation"}</span></div><div class="hs-chat-header-actions"><span class="hs-runtime-state ${shellHealth.models === "degraded" ? "warn" : ""}">${escapeHtml(shellHealth.models === "degraded" ? "Model runtime offline" : humanize(workspace?.model_runtime_state || "loading"))}</span><button type="button" id="hs-chat-refresh" title="Refresh Agent Chat" ${loading ? "disabled" : ""}>↻</button><button type="button" id="hs-chat-open-connections">Connections</button></div></header>'''
if old_header not in chat:
    raise SystemExit("Agent Chat header anchor not found")
chat = chat.replace(old_header, new_header, 1)

bind_anchor = '  document.querySelector("#hs-chat-new")?.addEventListener("click", startNewChat);\n'
bind_replacement = '  document.querySelector("#hs-chat-control-center")?.addEventListener("click", () => { window.location.hash = "#dashboard"; });\n' + bind_anchor
if bind_anchor not in chat:
    raise SystemExit("Agent Chat bind anchor not found")
chat = chat.replace(bind_anchor, bind_replacement, 1)

end_anchor = 'window.addEventListener("homeserver-agent-route", () => window.setTimeout(() => mount(true), 0));\n'
health_listener = '''window.addEventListener("homeserver-shell-health", (event) => {
  shellHealth = { ...shellHealth, ...(event.detail || {}) };
  if (isAgentPage()) mount(true);
});
'''
if end_anchor not in chat:
    raise SystemExit("Agent Chat route event anchor not found")
chat = chat.replace(end_anchor, health_listener + end_anchor, 1)

replacements = {
'.hs-chat-page{display:grid;grid-template-columns:280px minmax(0,1fr);min-height:calc(100vh - 106px);height:calc(100vh - 106px);margin:-4px;color:#111827;background:#fff;border:1px solid #e5e7eb;border-radius:20px;overflow:hidden;box-shadow:0 18px 48px rgba(15,23,42,.06)}': '.hs-chat-page{display:grid;grid-template-columns:300px minmax(0,1fr);width:100%;height:100vh;min-height:100vh;margin:0;color:#111827;background:#fff;overflow:hidden}',
'.hs-chat-sidebar{display:flex;flex-direction:column;min-width:0;padding:18px 14px 14px;border-right:1px solid #e5e7eb;background:#f8fafc}': '.hs-chat-sidebar{display:flex;flex-direction:column;min-width:0;height:100vh;padding:18px 14px 14px;border-right:1px solid #e5e7eb;background:#f8fafc;overflow:hidden}',
'.hs-chat-main{display:flex;min-width:0;min-height:0;flex-direction:column;background:#fff}': '.hs-chat-main{position:relative;display:block;min-width:0;height:100vh;min-height:0;overflow:hidden;background:#fff}',
'.hs-chat-header{display:flex;align-items:center;justify-content:space-between;gap:14px;min-height:68px;padding:12px 18px;border-bottom:1px solid #edf0f4;background:rgba(255,255,255,.97)}': '.hs-chat-header{position:absolute;inset:0 0 auto 0;z-index:20;display:flex;align-items:center;justify-content:space-between;gap:14px;height:68px;padding:12px 22px;border-bottom:1px solid #edf0f4;background:rgba(255,255,255,.94);backdrop-filter:blur(16px)}',
'.hs-chat-stream{flex:1;min-height:0;overflow-y:auto;padding:28px max(28px,calc((100% - 900px)/2));scroll-behavior:smooth;background:linear-gradient(#fff,#fbfcfe)}': '.hs-chat-stream{position:absolute;inset:68px 0 0 0;overflow-y:auto;padding:34px max(30px,calc((100% - 980px)/2)) 230px;scroll-behavior:smooth;background:linear-gradient(#fff,#fbfcfe)}',
'.hs-chat-composer{position:sticky;bottom:0;z-index:5;padding:10px max(18px,calc((100% - 930px)/2)) 14px;border-top:1px solid #edf0f4;background:linear-gradient(rgba(255,255,255,.9),#fff 22%);backdrop-filter:blur(14px)}': '.hs-chat-composer{position:absolute;inset:auto 0 0 0;z-index:30;padding:18px max(24px,calc((100% - 980px)/2)) 20px;background:linear-gradient(180deg,rgba(255,255,255,0),rgba(255,255,255,.94) 18%,#fff 42%);backdrop-filter:blur(16px)}',
}
for old, new in replacements.items():
    if old not in css:
        raise SystemExit(f"CSS anchor not found: {old[:48]}")
    css = css.replace(old, new, 1)

css += '''
html.agent-chat-mode,html.agent-chat-mode body,html.agent-chat-mode #app{width:100%;height:100%;overflow:hidden}
.agent-chat-shell,.homeserver-agent-route-host{width:100%;height:100vh;min-height:100vh;overflow:hidden;background:#fff}
.hs-chat-sidebar-footer{margin-top:10px;padding-top:10px;border-top:1px solid #e2e8f0}
.hs-chat-sidebar-footer button{display:flex;align-items:center;gap:10px;width:100%;padding:10px;border:0;border-radius:11px;background:transparent;color:#475569;text-align:left;cursor:pointer}
.hs-chat-sidebar-footer button:hover{background:#e9eef5;color:#0f172a}.hs-chat-sidebar-footer button>span{display:grid;place-items:center;width:30px;height:30px;border:1px solid #dbe3ee;border-radius:9px;background:#fff}.hs-chat-sidebar-footer strong,.hs-chat-sidebar-footer small{display:block}.hs-chat-sidebar-footer strong{font-size:11px}.hs-chat-sidebar-footer small{margin-top:3px;color:#94a3b8;font-size:9px}
.hs-runtime-state.warn{border-color:#fed7aa;background:#fff7ed;color:#c2410c}
@media(max-width:820px){.hs-chat-page{grid-template-columns:220px minmax(0,1fr)}.hs-chat-sidebar{padding:12px 10px}.hs-chat-stream{padding-left:18px;padding-right:18px}.hs-chat-composer{padding-left:16px;padding-right:16px}.hs-chat-composer-footer small{display:none}}
@media(max-width:640px){.hs-chat-page{grid-template-columns:1fr}.hs-chat-sidebar{display:none}.hs-chat-header{padding:10px 14px}.hs-chat-header-actions .hs-runtime-state{display:none}.hs-chat-stream{padding-bottom:210px}.hs-chat-composer-tools{padding-bottom:2px}}
'''

validator = '''from pathlib import Path

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
if 'event.target.closest("[data-page]")' in main:
    raise SystemExit("Competing delegated Control Center router remains")

print("Agent Chat is isolated, full-window, independently scrollable, and resilient to optional module failures.")
'''

main_path.write_text(main, encoding="utf-8")
chat_path.write_text(chat, encoding="utf-8")
css_path.write_text(css, encoding="utf-8")
validator_path.write_text(validator, encoding="utf-8")
print("Repaired optional-module resilience and redesigned Agent Chat layout.")
