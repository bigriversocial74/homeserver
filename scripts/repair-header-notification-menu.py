from __future__ import annotations

from pathlib import Path

root = Path(__file__).resolve().parents[1]
main_path = root / "src" / "main.js"
css_path = root / "src" / "styles.css"
package_path = root / "package.json"
validator_path = root / "scripts" / "validate-notification-menu.py"

main = main_path.read_text(encoding="utf-8")
css = css_path.read_text(encoding="utf-8")
package = package_path.read_text(encoding="utf-8")

state_anchor = 'let activePage = window.location.hash.replace("#", "") || "dashboard";\n'
if 'let notificationMenuOpen = false;' not in main:
    if state_anchor not in main:
        raise SystemExit("Notification state anchor not found")
    main = main.replace(state_anchor, state_anchor + 'let notificationMenuOpen = false;\n', 1)

menu_functions = r'''
function notificationItems() {
  const items = [];
  if (!isHealthy()) items.push({ tone: "critical", icon: "system", title: "HomeServer needs attention", detail: "The local service is not reporting healthy status.", page: "system" });
  if (!isConnected()) items.push({ tone: "warning", icon: "key", title: "HomeServer is not paired", detail: "Connect a management provider to enable licensed cloud services.", page: "sync" });
  if (!lastBackup()) items.push({ tone: "warning", icon: "backup", title: "No protected backup yet", detail: "Create a verified local recovery point.", page: "backups" });
  if (modelSnapshot?.runtime?.state !== "running") items.push({ tone: "warning", icon: "model", title: "Model runtime is offline", detail: "Open Model Center to install or start Ollama.", page: "models" });
  if (updateDisplayState() === "not_configured") items.push({ tone: "info", icon: "update", title: "Release channel setup needed", detail: "Configure the signed HomeServer update source.", page: "system" });
  if (!items.length) items.push({ tone: "success", icon: "shield", title: "HomeServer is healthy", detail: "No active system alerts require attention.", page: "dashboard" });
  return items;
}

function renderNotificationMenu() {
  const items = notificationItems();
  const alertCount = items.filter((item) => item.tone !== "success").length;
  return `<div class="notification-center ${notificationMenuOpen ? "open" : ""}">
    <button type="button" class="icon-button notification-toggle" id="notification-toggle" aria-label="Notifications" aria-haspopup="menu" aria-expanded="${notificationMenuOpen ? "true" : "false"}">${icon("bell", 19)}${alertCount ? `<span class="notification-count">${Math.min(alertCount, 9)}</span>` : ""}</button>
    ${notificationMenuOpen ? `<section class="notification-dropdown" id="notification-dropdown" role="menu" aria-label="HomeServer notifications">
      <header><div><strong>Notifications</strong><span>${alertCount ? `${alertCount} item${alertCount === 1 ? "" : "s"} need attention` : "Everything looks good"}</span></div><button type="button" id="notification-close" aria-label="Close notifications">×</button></header>
      <div class="notification-list">${items.map((item) => `<button type="button" class="notification-item ${item.tone}" data-notification-page="${item.page}" role="menuitem"><span class="notification-item-icon">${icon(item.icon, 17)}</span><span><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.detail)}</small></span>${icon("arrow", 13)}</button>`).join("")}</div>
      <footer><button type="button" data-notification-page="system">View system activity</button></footer>
    </section>` : ""}
  </div>`;
}
'''
if 'function renderNotificationMenu()' not in main:
    anchor = 'function renderTopbar() {\n'
    if anchor not in main:
        raise SystemExit("Topbar anchor not found")
    main = main.replace(anchor, menu_functions + '\n' + anchor, 1)

old_bell = '    <button type="button" class="icon-button" aria-label="Notifications">${icon("bell", 19)}<span class="notification-count">3</span></button>\n'
if old_bell in main:
    main = main.replace(old_bell, '    ${renderNotificationMenu()}\n', 1)
elif '${renderNotificationMenu()}' not in main:
    raise SystemExit("Decorative notification button anchor not found")

bind_anchor = 'function bindEvents() {\n  document.querySelectorAll("[data-page]").forEach((button) => button.addEventListener("click", () => navigate(button.dataset.page)));\n'
bind_replacement = '''function bindEvents() {
  document.querySelectorAll("[data-page]").forEach((button) => button.addEventListener("click", () => navigate(button.dataset.page)));
  document.querySelector("#notification-toggle")?.addEventListener("click", (event) => {
    event.stopPropagation();
    notificationMenuOpen = !notificationMenuOpen;
    render();
  });
  document.querySelector("#notification-close")?.addEventListener("click", () => {
    notificationMenuOpen = false;
    render();
  });
  document.querySelectorAll("[data-notification-page]").forEach((button) => button.addEventListener("click", () => {
    notificationMenuOpen = false;
    navigate(button.dataset.notificationPage);
  }));
'''
if 'document.querySelector("#notification-toggle")' not in main:
    if bind_anchor not in main:
        raise SystemExit("Event-binding anchor not found")
    main = main.replace(bind_anchor, bind_replacement, 1)

navigate_anchor = '  activePage = page;\n  history.replaceState(null, "", `#${page}`);\n'
if '  notificationMenuOpen = false;\n  activePage = page;' not in main:
    if navigate_anchor not in main:
        raise SystemExit("Navigate anchor not found")
    main = main.replace(navigate_anchor, '  notificationMenuOpen = false;\n' + navigate_anchor, 1)

outside_events = r'''
document.addEventListener("click", (event) => {
  if (!notificationMenuOpen) return;
  if (event.target instanceof Element && event.target.closest(".notification-center")) return;
  notificationMenuOpen = false;
  render();
});

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape" || !notificationMenuOpen) return;
  notificationMenuOpen = false;
  render();
  document.querySelector("#notification-toggle")?.focus();
});

'''
if 'event.target.closest(".notification-center")' not in main:
    anchor = 'window.addEventListener("hashchange", () => {\n'
    if anchor not in main:
        raise SystemExit("Global event anchor not found")
    main = main.replace(anchor, outside_events + anchor, 1)

css_block = r'''
.notification-center{position:relative;display:inline-flex;align-items:center}
.notification-toggle[aria-expanded="true"]{border-color:#cdd9ea;background:#eef4ff;color:var(--blue)}
.notification-dropdown{position:absolute;top:44px;right:0;z-index:1200;width:360px;overflow:hidden;border:1px solid #dce4ef;border-radius:14px;background:#fff;box-shadow:0 22px 55px rgba(20,42,82,.18)}
.notification-dropdown>header{display:flex;align-items:flex-start;justify-content:space-between;gap:16px;padding:16px 16px 13px;border-bottom:1px solid #e8edf4}.notification-dropdown>header strong,.notification-dropdown>header span{display:block}.notification-dropdown>header strong{font-size:14px}.notification-dropdown>header span{margin-top:4px;color:var(--muted);font-size:10px}.notification-dropdown>header button{display:grid;place-items:center;width:28px;height:28px;border:1px solid #dde5ef;border-radius:8px;background:#fff;color:#66748c;font-size:18px}
.notification-list{display:grid;max-height:380px;overflow-y:auto;padding:7px}
.notification-item{display:grid;grid-template-columns:34px minmax(0,1fr) 16px;align-items:center;gap:10px;width:100%;padding:11px 10px;border:0;border-radius:10px;background:transparent;color:#24334f;text-align:left}.notification-item:hover{background:#f5f8fc}.notification-item-icon{display:grid;place-items:center;width:34px;height:34px;border-radius:10px;background:#eef3fa;color:#52637d}.notification-item>span:nth-child(2){min-width:0}.notification-item strong,.notification-item small{display:block}.notification-item strong{font-size:11px}.notification-item small{margin-top:4px;color:var(--muted);font-size:9px;line-height:1.45}.notification-item.critical .notification-item-icon{background:var(--red-soft);color:var(--red)}.notification-item.warning .notification-item-icon{background:var(--amber-soft);color:var(--amber)}.notification-item.info .notification-item-icon{background:var(--blue-soft);color:var(--blue)}.notification-item.success .notification-item-icon{background:var(--green-soft);color:var(--green)}
.notification-dropdown>footer{padding:10px 14px;border-top:1px solid #e8edf4;background:#fbfcfe}.notification-dropdown>footer button{padding:0;border:0;background:transparent;color:var(--blue);font-size:10px;font-weight:750}
'''
if '.notification-dropdown{' not in css:
    css = css.rstrip() + '\n\n' + css_block.strip() + '\n'

validator = r'''from pathlib import Path

root = Path(__file__).resolve().parents[1]
main = (root / "src" / "main.js").read_text(encoding="utf-8")
css = (root / "src" / "styles.css").read_text(encoding="utf-8")

required_main = [
    'let notificationMenuOpen = false;',
    'function notificationItems()',
    'function renderNotificationMenu()',
    'id="notification-toggle"',
    'id="notification-dropdown"',
    'data-notification-page=',
    'event.target.closest(".notification-center")',
    'event.key !== "Escape"',
]
required_css = [
    '.notification-center{position:relative',
    '.notification-dropdown{position:absolute',
    '.notification-item{display:grid',
]
for marker in required_main:
    if marker not in main:
        raise SystemExit(f"Missing notification menu contract: {marker}")
for marker in required_css:
    if marker not in css:
        raise SystemExit(f"Missing notification menu style contract: {marker}")
if 'aria-label="Notifications">${icon("bell", 19)}<span class="notification-count">3</span>' in main:
    raise SystemExit("Decorative dead notification button remains")
print("Header notifications open a bounded, keyboard-accessible dropdown menu.")
'''
validator_path.write_text(validator, encoding="utf-8")

if 'python scripts/validate-notification-menu.py' not in package:
    package = package.replace(
        'python scripts/validate-agent-chat-route.py && ',
        'python scripts/validate-agent-chat-route.py && python scripts/validate-notification-menu.py && ',
        1,
    )

main_path.write_text(main, encoding="utf-8")
css_path.write_text(css, encoding="utf-8")
package_path.write_text(package, encoding="utf-8")
print("Implemented the interactive header notification dropdown.")
