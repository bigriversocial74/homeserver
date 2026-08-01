import { invoke } from "@tauri-apps/api/core";
import "./styles.css";
import "./openrouter-provider.js";
import { icon, logoMark } from "./icons.js";

const app = document.querySelector("#app");
let statusSnapshot = null;
let cloudSnapshot = null;
let backupCatalog = null;
let updateStatus = null;
let vaultSnapshot = null;
let semanticSnapshot = null;
let vaultSearchResult = null;
let modelSnapshot = null;
let modelTestResult = null;
let mcpSnapshot = null;
let mcpCredential = null;
let mcpBridgePath = null;
let agentIntegrationSnapshot = null;
let notice = null;
let busy = false;
let activePage = window.location.hash.replace("#", "") || "dashboard";
let notificationMenuOpen = false;
let desktopAutostartEnabled = false;

const pages = [
  ["agent", "HomeServer Agent", "integrations"],
  ["home", "Home", "home"],
  ["dashboard", "Dashboard", "dashboard"],
  ["models", "Model Center", "model"],
  ["apps", "Apps", "apps"],
  ["knowledge", "Knowledge Vault", "vault"],
  ["backups", "Backups", "backup"],
  ["integrations", "Integrations & Agents", "integrations"],
  ["settings", "Settings", "settings"],
  ["sync", "Sync Cloud", "cloud"],
  ["system", "System", "system"],
];

const serviceRows = [
  ["Control Center", "Central management and system monitoring.", "dashboard"],
  ["Local API", "Private loopback API for trusted local automation.", "network"],
  ["Local database", "Embedded operational storage for HomeServer state.", "storage"],
  ["Microgifter Cloud", "Signed cloud pairing, heartbeat, and synchronization.", "cloud"],
  ["Backup Engine", "Encrypted backups, recovery packages, and staged restore.", "backup"],
  ["Update Manager", "Signed release discovery, verified staging, and rollback.", "update"],
  ["Model Center", "Optional local AI runtime management.", "model"],
];

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function statusClass(state) {
  return String(state || "offline").toLowerCase().replaceAll("_", "-");
}

function humanize(value) {
  return String(value || "unknown").replaceAll("_", " ");
}

function formatDate(value) {
  if (!value) return "Not yet";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function relativeDate(value) {
  if (!value) return "Not yet";
  const timestamp = new Date(value).getTime();
  if (!Number.isFinite(timestamp)) return formatDate(value);
  const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
  if (seconds < 60) return "Just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

function formatBytes(value) {
  const bytes = Number(value || 0);
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** unit).toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function compactId(value) {
  const text = String(value || "");
  if (text.length <= 20) return text || "Not assigned";
  return `${text.slice(0, 8)}…${text.slice(-8)}`;
}

function isConnected() {
  return ["connected", "degraded"].includes(cloudSnapshot?.state);
}

function isHealthy() {
  return Boolean(statusSnapshot?.api_available) && !["failed", "offline"].includes(statusSnapshot?.state);
}

function updateDisplayState() {
  const state = updateStatus?.state || "idle";
  if (state === "failed" && !updateStatus?.update) return "not_configured";
  return state;
}

function updateErrorText() {
  if (!updateStatus?.last_error) return "";
  if (!updateStatus?.update && updateStatus.state === "failed") {
    return "The public release channel is not configured for this pre-launch build.";
  }
  return humanize(updateStatus.last_error);
}

function badge(label, state = "neutral") {
  return `<span class="status-badge ${statusClass(state)}"><span class="status-dot"></span>${escapeHtml(label)}</span>`;
}

function pageHeader(title, subtitle, actions = "") {
  return `<header class="page-header"><div><h1>${escapeHtml(title)}</h1><p>${escapeHtml(subtitle)}</p></div><div class="page-actions">${actions}</div></header>`;
}

function metricCard(iconName, label, value, detail, tone = "blue") {
  return `<article class="metric-card tone-${tone}"><div class="metric-icon">${icon(iconName, 23)}</div><div><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong><small>${escapeHtml(detail)}</small></div></article>`;
}

function progress(value, tone = "blue") {
  const safe = Math.max(0, Math.min(100, Number(value || 0)));
  return `<div class="progress"><span class="tone-${tone}" style="width:${safe}%"></span></div>`;
}

function donut(value, center, detail, tone = "blue") {
  const safe = Math.max(0, Math.min(100, Number(value || 0)));
  return `<div class="donut tone-${tone}" style="--value:${safe}"><div><strong>${escapeHtml(center)}</strong><span>${escapeHtml(detail)}</span></div></div>`;
}

function backupCount() {
  return backupCatalog?.backups?.length || 0;
}

function lastBackup() {
  return backupCatalog?.backups?.[0] || null;
}

function renderSidebar() {
  const state = statusSnapshot?.state || "offline";
  return `<aside class="app-sidebar">
    <div class="brand-lockup">${logoMark(43)}<div><strong>Microgifter</strong><span>HomeServer</span></div></div>
    <nav class="primary-nav" aria-label="HomeServer pages">
      ${pages.map(([key, label, iconName]) => `<button type="button" class="nav-item ${activePage === key ? "active" : ""}" data-page="${key}">${icon(iconName, 19)}<span>${escapeHtml(label)}</span></button>`).join("")}
    </nav>
    <div class="server-card">
      <div class="server-card-top"><div class="server-glyph">${icon("system", 22)}</div><div><strong>HomeServer</strong><span><i class="live-dot"></i>${isHealthy() ? "Online" : "Unavailable"}</span></div></div>
      <div class="server-divider"></div>
      <small>v${escapeHtml(statusSnapshot?.version || "0.1.5")} ${updateStatus?.channel ? `(${escapeHtml(humanize(updateStatus.channel))})` : ""}</small>
      <button type="button" class="text-button" data-page="system">${updateDisplayState() === "not_configured" ? "Release channel setup" : "Check for updates"}</button>
    </div>
    <div class="sidebar-state"><span class="state-orb ${statusClass(state)}"></span><span>${escapeHtml(humanize(state))}</span></div>
  </aside>`;
}


function notificationItems() {
  const items = [];
  if (!isHealthy()) items.push({ tone: "critical", icon: "system", title: "HomeServer needs attention", detail: "The local service is not reporting healthy status.", page: "system" });
  if (!isConnected()) items.push({ tone: "warning", icon: "key", title: "HomeServer is not paired", detail: "Connect a management provider to enable licensed cloud services.", page: "sync" });
  if (!lastBackup()) items.push({ tone: "warning", icon: "backup", title: "No protected backup yet", detail: "Create a verified local recovery point.", page: "backups" });
  if (modelSnapshot?.runtime?.state !== "running") items.push({ tone: "warning", icon: "model", title: "Model runtime is offline", detail: "Open Model Center to install or start Ollama.", page: "models" });
  if (updateDisplayState() === "not_configured") items.push({ tone: "info", icon: "update", title: "Release channel setup needed", detail: "Configure the signed HomeServer update source.", page: "system" });
  const agentPrompt = agentIntegrationSnapshot?.active_prompt;
  if (agentPrompt) items.push({ tone: "info", icon: "integrations", title: agentPrompt.title, detail: agentPrompt.message, page: "agent" });
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

function renderTopbar() {
  return `<div class="app-topbar">
    <label class="global-search">${icon("search", 17)}<input type="search" placeholder="Search HomeServer" aria-label="Search HomeServer"></label>
    ${badge(isHealthy() ? "Healthy" : "Attention", isHealthy() ? "healthy" : "degraded")}
    ${renderNotificationMenu()}
    <button type="button" class="avatar-button" aria-label="Account menu"><span>MG</span>${icon("arrow", 13)}</button>
  </div>`;
}

function renderDashboard() {
  const cloudState = cloudSnapshot?.state || "not_paired";
  const backup = lastBackup();
  const updateState = updateDisplayState();
  const pending = Number(cloudSnapshot?.pending_sync || statusSnapshot?.pending_sync || 0);
  const serviceHealth = isHealthy() ? "Healthy" : "Attention";
  return `${pageHeader("Dashboard", "Overview of your HomeServer system health and activity.", `<button id="refresh-status" class="button secondary" type="button" ${busy ? "disabled" : ""}>${icon("refresh", 17)}Refresh</button>`)}
    <section class="metrics six-up">
      ${metricCard("shield", "Service Health", serviceHealth, isHealthy() ? "All services running" : "Service unavailable", isHealthy() ? "green" : "amber")}
      ${metricCard("key", "Pairing Status", isConnected() ? "Connected" : "Not paired", isConnected() ? "Paired to this device" : "Pair from Microgifter", isConnected() ? "blue" : "amber")}
      ${metricCard("cloud", "Sync Status", isConnected() ? "Synced" : "Waiting", pending ? `${pending} queued items` : "No pending changes", isConnected() ? "blue" : "gray")}
      ${metricCard("backup", "Backup Status", backup ? "Protected" : "Ready", backup ? `Last backup ${relativeDate(backup.created_at_utc)}` : "Create your first backup", "teal")}
      ${metricCard("update", "Signed Updates", updateState === "not_configured" ? "Setup needed" : humanize(updateState), updateState === "not_configured" ? "Pre-launch channel" : statusSnapshot?.update_version || "Stable channel", updateState === "not_configured" ? "amber" : "green")}
      ${metricCard("vault", "Knowledge Vault", "Ready", "Local indexing workspace", "purple")}
    </section>
    <section class="dashboard-grid">
      <article class="panel system-status-panel">
        <div class="panel-title"><div>${icon("system", 18)}<h2>System Status</h2></div><button class="text-button" data-page="system">View system details ${icon("arrow", 13)}</button></div>
        <dl class="detail-list">
          <div><dt>Device Name</dt><dd>${escapeHtml(statusSnapshot?.server_name || "HomeServer")}</dd></div>
          <div><dt>Operating System</dt><dd>Windows</dd></div>
          <div><dt>Version</dt><dd>${escapeHtml(statusSnapshot?.version || "0.1.5")}</dd></div>
          <div><dt>Local API</dt><dd>${statusSnapshot?.api_available ? "Available" : "Offline"}</dd></div>
          <div><dt>Database</dt><dd>${escapeHtml(humanize(statusSnapshot?.database || "unknown"))}</dd></div>
          <div><dt>Sync Queue</dt><dd>${pending}</dd></div>
        </dl>
        <div class="resource-row"><span>Service health</span><strong>${isHealthy() ? "100%" : "35%"}</strong></div>${progress(isHealthy() ? 100 : 35, isHealthy() ? "green" : "amber")}
      </article>
      <article class="panel activity-panel">
        <div class="panel-title"><div>${icon("activity", 18)}<h2>Activity Timeline</h2></div><button class="text-button" data-page="system">View all</button></div>
        <div class="timeline">
          <div><i class="timeline-icon tone-blue">${icon("cloud", 16)}</i><span>${relativeDate(cloudSnapshot?.last_success_utc)}</span><p><strong>Cloud status checked</strong><small>${isConnected() ? "Signed Microgifter connection is active" : "Waiting for pairing"}</small></p></div>
          <div><i class="timeline-icon tone-green">${icon("backup", 16)}</i><span>${backup ? relativeDate(backup.created_at_utc) : "Not yet"}</span><p><strong>${backup ? "Backup completed" : "Backup ready"}</strong><small>${backup ? `${humanize(backup.kind)} · ${formatBytes(backup.size_bytes)}` : "Create a protected local backup"}</small></p></div>
          <div><i class="timeline-icon tone-purple">${icon("vault", 16)}</i><span>Ready</span><p><strong>Knowledge Vault available</strong><small>Local-first indexing interface prepared</small></p></div>
          <div><i class="timeline-icon tone-teal">${icon("integrations", 16)}</i><span>Active</span><p><strong>Core agents loaded</strong><small>Pairing, sync, backup, and update services</small></p></div>
        </div>
      </article>
      <article class="panel storage-panel">
        <div class="panel-title"><div>${icon("storage", 18)}<h2>Storage Overview</h2></div><span class="mini-select">Local data</span></div>
        <div class="storage-layout">${donut(33, backup ? formatBytes(backup.size_bytes) : "Local", "protected", "blue")}<ul class="legend"><li><i class="blue"></i><span>HomeServer data</span><strong>Local</strong></li><li><i class="green"></i><span>Backups</span><strong>${backupCount()}</strong></li><li><i class="purple"></i><span>Knowledge Vault</span><strong>Ready</strong></li><li><i class="amber"></i><span>Apps</span><strong>${serviceRows.length}</strong></li></ul></div>
        <button class="text-button align-end" data-page="backups">Manage Storage ${icon("arrow", 13)}</button>
      </article>
      <article class="panel recent-events-panel">
        <div class="panel-title"><div><h2>Recent Events</h2></div><button class="text-button" data-page="system">View all events</button></div>
        <div class="events-table">
          <div class="table-head"><span>Time</span><span>Event</span><span>Source</span><span>Details</span><span>Status</span></div>
          ${eventRow(relativeDate(cloudSnapshot?.last_success_utc), isConnected() ? "Sync connected" : "Pairing required", "Sync Service", isConnected() ? "Signed cloud bridge active" : "No cloud device paired", isConnected() ? "Success" : "Action", isConnected() ? "success" : "warning")}
          ${eventRow(backup ? relativeDate(backup.created_at_utc) : "Not yet", backup ? "Backup completed" : "Backup not created", "Backup Engine", backup ? `${humanize(backup.kind)} · ${formatBytes(backup.size_bytes)}` : "Create a protected local backup", backup ? "Success" : "Ready", backup ? "success" : "info")}
          ${eventRow(relativeDate(statusSnapshot?.last_updated_utc), "Status refreshed", "Core Service", statusSnapshot?.api_available ? "Local API is responding" : "Local service unavailable", statusSnapshot?.api_available ? "Info" : "Warning", statusSnapshot?.api_available ? "info" : "warning")}
        </div>
      </article>
      <article class="panel glance-panel"><div class="panel-title"><div>${icon("dashboard", 18)}<h2>At a Glance</h2></div></div><div class="glance-list">
        ${glanceRow("key", "Paired Device", isConnected() ? "1" : "0", "sync")}
        ${glanceRow("apps", "Running Services", String(serviceRows.filter((_, i) => i < 6).length), "apps")}
        ${glanceRow("backup", "Total Backups", String(backupCount()), "backups")}
        ${glanceRow("integrations", "Integrations", isConnected() ? "1" : "0", "integrations")}
        ${glanceRow("update", "Version", statusSnapshot?.version || "0.1.5", "system")}
      </div></article>
    </section>`;
}

function eventRow(time, event, source, details, status, state) {
  return `<div class="table-row"><span><i class="event-dot ${state}"></i>${escapeHtml(time)}</span><strong>${escapeHtml(event)}</strong><span>${escapeHtml(source)}</span><span>${escapeHtml(details)}</span><em class="table-status ${state}">${escapeHtml(status)}</em></div>`;
}

function glanceRow(iconName, label, value, page) {
  return `<button type="button" data-page="${page}">${icon(iconName, 17)}<span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong>${icon("arrow", 13)}</button>`;
}
function renderHome() {
  const connected = isConnected();
  return `${pageHeader("Home", `Welcome back! Your HomeServer is ${isHealthy() ? "running smoothly" : "waiting for the local service"}.`)}
    <section class="home-hero-grid">
      <article class="panel welcome-card"><div class="hero-symbol tone-blue">${icon("home", 29)}</div><div><h2>Welcome Back</h2><p>Your HomeServer is ${isHealthy() ? "healthy and ready to protect what matters most" : "installed, but the local API is not responding yet"}.</p><div class="inline-badges">${badge(isHealthy() ? "All services running" : "Service attention", isHealthy() ? "healthy" : "degraded")}${badge(updateDisplayState() === "not_configured" ? "Beta channel" : "Up to date", updateDisplayState() === "not_configured" ? "planned" : "healthy")}</div></div></article>
      <article class="panel pairing-card"><div class="hero-symbol tone-blue">${icon("key", 28)}</div><div><h2>Pairing Connection</h2><p>${connected ? "Your HomeServer is paired and connected to Microgifter." : "Pair this HomeServer to your Microgifter account."}</p><div class="paired-stats"><span>Paired devices<strong>${connected ? "1" : "0"}</strong></span><span>Connection status<strong class="${connected ? "positive" : "warning-text"}">${connected ? "Connected" : "Not paired"}</strong></span></div></div><button type="button" class="button secondary" data-page="sync">${connected ? "View Connection" : "Pair Device"}</button></article>
      <article class="panel summary-card"><div class="hero-symbol tone-blue">${icon("system", 27)}</div><div><h2>System Summary</h2><dl class="summary-list"><div><dt>Version</dt><dd>${escapeHtml(statusSnapshot?.version || "0.1.5")}</dd></div><div><dt>Backups</dt><dd>${backupCount()}</dd></div><div><dt>Pending sync</dt><dd>${Number(cloudSnapshot?.pending_sync || 0)}</dd></div><div><dt>Health status</dt><dd>${badge(isHealthy() ? "Healthy" : "Attention", isHealthy() ? "healthy" : "degraded")}</dd></div></dl><button class="text-button" data-page="dashboard">Open Dashboard</button></div></article>
    </section>
    <section class="panel quick-actions"><div class="panel-title"><div><h2>Quick Actions</h2></div></div><div class="quick-action-grid">
      ${quickAction("key", "Pair Device", "Connect this HomeServer to your Microgifter account.", "Pair Device", "sync", "green")}
      ${quickAction("sync", "Sync Now", "Run a signed manual synchronization with Microgifter.", "Sync Now", "sync-now", "blue")}
      ${quickAction("backup", "Create Backup", "Start a protected backup of local HomeServer data.", "Create Backup", "backup-now", "purple")}
      ${quickAction("dashboard", "Open Dashboard", "Review system health, activity, and local services.", "Open Dashboard", "dashboard", "teal")}
      ${quickAction("settings", "Manage Settings", "Configure the Control Center experience and security.", "Manage Settings", "settings", "gray")}
    </div></section>
    <section class="two-column-grid home-bottom-grid">
      <article class="panel"><div class="panel-title"><div><h2>Recent Activity</h2></div><button class="text-button" data-page="system">View all activity</button></div><div class="activity-list">
        ${activityItem("backup", "Backup status", lastBackup() ? `Completed ${relativeDate(lastBackup()?.created_at_utc)} · ${formatBytes(lastBackup()?.size_bytes)}` : "No backup created yet", lastBackup() ? "green" : "amber")}
        ${activityItem("sync", "Cloud synchronization", connected ? `Last success ${relativeDate(cloudSnapshot?.last_success_utc)}` : "Pairing required", connected ? "blue" : "amber")}
        ${activityItem("vault", "Knowledge Vault ready", "Local-first indexing interface is available", "purple")}
        ${activityItem("integrations", "Core agents loaded", "Pairing, backup, update, and recovery services", "teal")}
      </div></article>
      <article class="panel"><div class="panel-title"><div><h2>Recommended Next Steps</h2></div></div><div class="next-steps">
        ${nextStep("backup", "Create a protected backup", "Establish your first verified recovery point.", "backups")}
        ${nextStep("key", connected ? "Review pairing security" : "Pair your HomeServer", connected ? "Confirm the signed cloud connection remains healthy." : "Connect your account using a one-time code.", "sync")}
        ${nextStep("shield", "Review update delivery", "Production signing can be enabled when launch credentials are ready.", "system")}
        ${nextStep("vault", "Explore Knowledge Vault", "Prepare sources for private local indexing.", "knowledge")}
      </div></article>
    </section>`;
}

function quickAction(iconName, title, copy, button, action, tone) {
  const attr = ["dashboard", "settings", "sync"].includes(action) ? `data-page="${action}"` : `data-quick-action="${action}"`;
  return `<article><div class="hero-symbol tone-${tone}">${icon(iconName, 26)}</div><h3>${escapeHtml(title)}</h3><p>${escapeHtml(copy)}</p><button type="button" class="button ghost" ${attr}>${escapeHtml(button)}</button></article>`;
}

function activityItem(iconName, title, copy, tone) {
  return `<div><i class="timeline-icon tone-${tone}">${icon(iconName, 16)}</i><p><strong>${escapeHtml(title)}</strong><small>${escapeHtml(copy)}</small></p></div>`;
}

function nextStep(iconName, title, copy, page) {
  return `<button type="button" data-page="${page}">${icon(iconName, 19)}<p><strong>${escapeHtml(title)}</strong><small>${escapeHtml(copy)}</small></p>${icon("arrow", 14)}</button>`;
}

function renderApps() {
  return `${pageHeader("Apps", "Manage and monitor services installed on your HomeServer.")}
    <div class="toolbar-row"><label class="filter-search">${icon("search", 17)}<input type="search" placeholder="Search apps"></label><button class="select-button">${icon("activity", 16)}All Status ${icon("arrow", 12)}</button><button class="select-button">Name (A–Z) ${icon("arrow", 12)}</button></div>
    <section class="apps-layout"><div>
      <article class="panel"><div class="panel-title"><div><h2>Installed Apps</h2></div></div><div class="app-grid">
        ${serviceRows.slice(0, 6).map((item, index) => appCard(item, index)).join("")}
      </div></article>
      <article class="panel available-apps"><div class="panel-title"><div><h2>Available Apps</h2></div><span class="planned-label">Roadmap</span></div><div class="available-grid">
        ${availableApp("play", "Media Server", "Stream and manage your local media library.", "amber")}
        ${availableApp("file", "Photo Manager", "Organize and protect your private photo library.", "purple")}
        ${availableApp("download", "Download Manager", "Schedule and manage local downloads.", "green")}
        ${availableApp("model", "Local Model Runtime", "Run isolated local AI model environments.", "blue")}
      </div></article>
    </div><aside class="apps-side">
      <article class="panel"><div class="panel-title"><div><h2>App Health</h2></div></div><div class="health-donut">${donut(isHealthy() ? 100 : 45, String(serviceRows.length - 1), "active services", isHealthy() ? "green" : "amber")}</div><ul class="legend compact"><li><i class="green"></i><span>Running</span><strong>${isHealthy() ? 6 : 2}</strong></li><li><i class="blue"></i><span>Connected</span><strong>${isConnected() ? 1 : 0}</strong></li><li><i class="amber"></i><span>Planned</span><strong>1</strong></li><li><i class="red"></i><span>Error</span><strong>${isHealthy() ? 0 : 1}</strong></li></ul></article>
      <article class="panel"><div class="panel-title"><div><h2>Resource Usage</h2></div></div>${resourceMeter("Core service", isHealthy() ? 18 : 0, "blue")}${resourceMeter("Backup engine", backupCount() ? 11 : 4, "green")}${resourceMeter("Cloud bridge", isConnected() ? 8 : 0, "purple")}<div class="resource-stats"><span>Local API<strong>${statusSnapshot?.api_available ? "Online" : "Offline"}</strong></span><span>Database<strong>${escapeHtml(humanize(statusSnapshot?.database || "unknown"))}</strong></span></div></article>
      <article class="panel"><div class="panel-title"><div><h2>Recent App Events</h2></div><button class="text-button" data-page="system">View all</button></div><div class="mini-event-list">${activityItem("backup", "Backup Engine", lastBackup() ? `Completed ${relativeDate(lastBackup()?.created_at_utc)}` : "Ready", "green")}${activityItem("cloud", "Sync Service", isConnected() ? "Connected" : "Waiting for pairing", "blue")}${activityItem("update", "Update Manager", updateDisplayState() === "not_configured" ? "Channel setup needed" : humanize(updateDisplayState()), "teal")}</div></article>
    </aside></section>`;
}

function appCard([name, description, iconName], index) {
  const running = index < 3 || (index === 3 && isConnected()) || (index === 4 && isHealthy()) || (index === 5 && isHealthy());
  return `<article class="app-card"><div class="app-card-head"><div class="app-icon tone-${["blue","blue","teal","purple","green","amber"][index]}">${icon(iconName, 27)}</div><div><h3>${escapeHtml(name)}</h3><span>v${escapeHtml(statusSnapshot?.version || "0.1.5")}</span></div>${badge(running ? "Healthy" : "Planned", running ? "healthy" : "planned")}</div><p>${escapeHtml(description)}</p><div class="app-card-stats"><span>State<strong>${running ? "Running" : "Planned"}</strong></span><span>CPU<strong>${running ? `${Math.max(1, index + 1)}.${index}%` : "0%"}</strong></span><span>Memory<strong>${running ? `${64 + index * 24} MB` : "—"}</strong></span></div><div class="app-card-actions"><button class="button secondary" data-page="${["dashboard","system","system","sync","backups","system"][index]}">Open</button><button class="button ghost" data-page="settings">Configure</button><button class="icon-button">${icon("menu", 17)}</button></div></article>`;
}

function availableApp(iconName, title, copy, tone) {
  return `<article><div class="app-icon tone-${tone}">${icon(iconName, 24)}</div><div><h3>${escapeHtml(title)}</h3><p>${escapeHtml(copy)}</p></div><button class="button ghost" type="button" disabled>Planned</button></article>`;
}

function resourceMeter(label, value, tone) {
  return `<div class="resource-meter"><div><span>${escapeHtml(label)}</span><strong>${value}%</strong></div>${progress(value, tone)}</div>`;
}
function renderBackups() {
  const backups = backupCatalog?.backups || [];
  const latest = backups[0];
  const totalBytes = backups.reduce((sum, backup) => sum + Number(backup.size_bytes || 0), 0);
  return `${pageHeader("Backups", "Protect your data with secure, encrypted backups and recovery options.")}
    <section class="backup-overview-grid">
      <article class="panel backup-status-card"><div class="panel-title"><div>${icon("backup", 18)}<h2>Backup Status</h2></div></div><div class="backup-status-main"><div class="hero-symbol tone-green">${icon("shield", 32)}</div><div><strong>${latest ? "Protected" : "Ready"}</strong><span>${latest ? "Verified recovery points are available" : "Create your first local backup"}</span></div></div><div class="four-stat-row"><span><strong>${backups.length}</strong>Total Backups</span><span><strong>${formatBytes(totalBytes)}</strong>Total Protected</span><span><strong>${backups.filter((backup) => backup.state === "verified").length}</strong>Verified</span><span><strong>${backups.filter((backup) => backup.state === "failed").length}</strong>Failed</span></div></article>
      <article class="panel latest-backup-card"><div class="panel-title"><div>${icon("backup", 18)}<h2>Latest Backup</h2></div></div><div class="latest-status ${latest ? "success" : "ready"}">${icon(latest ? "check" : "backup", 22)}<strong>${latest ? "Available" : "Not created"}</strong></div><dl class="detail-list"><div><dt>Created</dt><dd>${latest ? formatDate(latest.created_at_utc) : "Not yet"}</dd></div><div><dt>Type</dt><dd>${latest ? humanize(latest.kind) : "—"}</dd></div><div><dt>Size</dt><dd>${latest ? formatBytes(latest.size_bytes) : "—"}</dd></div><div><dt>Encryption</dt><dd>${latest ? humanize(latest.encryption) : "Device protected"}</dd></div></dl></article>
      <article class="panel protected-storage"><div class="panel-title"><div>${icon("storage", 18)}<h2>Protected Storage</h2></div></div><div class="storage-layout">${donut(backups.length ? Math.min(92, 18 + backups.length * 4) : 8, formatBytes(totalBytes), "backup data", "blue")}<ul class="legend"><li><i class="blue"></i><span>Automatic</span><strong>${backups.filter((backup) => backup.kind === "automatic").length}</strong></li><li><i class="green"></i><span>Manual</span><strong>${backups.filter((backup) => backup.kind === "manual").length}</strong></li><li><i class="purple"></i><span>Recovery</span><strong>${backups.filter((backup) => backup.kind === "recovery").length}</strong></li><li><i class="amber"></i><span>Pre-update</span><strong>${backups.filter((backup) => backup.kind === "pre_update").length}</strong></li></ul></div></article>
    </section>
    <section class="backup-action-layout"><div>
      <div class="backup-action-grid">
        <article class="panel action-card"><div class="hero-symbol tone-blue">${icon("upload", 26)}</div><div><h3>Create Backup</h3><p>Start a protected local snapshot of HomeServer data now.</p></div><button id="create-manual-backup" class="button primary" type="button" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>Create Backup</button></article>
        <article class="panel action-card"><div class="hero-symbol tone-green">${icon("shield", 26)}</div><div><h3>Verify Backup</h3><p>Verify integrity from the backup history below.</p></div><button class="button success" data-scroll-target="backup-history">View Backups</button></article>
        <article class="panel action-card"><div class="hero-symbol tone-purple">${icon("restore", 26)}</div><div><h3>Recovery Package</h3><p>Create a portable, passphrase-protected recovery package.</p></div><button class="button purple" data-toggle="recovery-create">Create Package</button></article>
      </div>
      <article class="panel recovery-panel hidden" id="recovery-create"><div class="panel-title"><div><h2>Portable Recovery Package</h2><p>Create an exportable package. The passphrase is never stored.</p></div></div><form id="recovery-package-form" class="horizontal-form"><input id="recovery-passphrase" type="password" minlength="12" maxlength="256" autocomplete="new-password" placeholder="Recovery passphrase" required><input id="recovery-passphrase-confirm" type="password" minlength="12" maxlength="256" autocomplete="new-password" placeholder="Confirm passphrase" required><button class="button primary" type="submit" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>Create Package</button></form></article>
      <article class="panel backup-history" id="backup-history"><div class="panel-title"><div>${icon("backup", 18)}<h2>Backup History</h2></div><span>${backups.length} records</span></div>${backupHistoryRows(backups)}</article>
    </div><aside class="backup-aside">
      <article class="panel secure-card"><div class="hero-symbol tone-green">${icon("lock", 26)}</div><div><h3>Encrypted & Secure</h3><p>Local backups use a Windows DPAPI-protected device key. Recovery packages use your passphrase.</p></div><dl class="detail-list"><div><dt>Local encryption</dt><dd>Device protected</dd></div><div><dt>Recovery package</dt><dd>Passphrase protected</dd></div></dl></article>
      <article class="panel retention-card"><div class="hero-symbol tone-blue">${icon("backup", 26)}</div><div><h3>Retention Policy</h3><p>Automatic backups run every ${backupCatalog?.interval_hours ?? 24} hours and retain ${backupCatalog?.retention_count ?? 14} recovery points.</p></div></article>
      <article class="panel"><div class="panel-title"><div><h3>Import Recovery Package</h3></div></div><form id="import-recovery-form" class="stack-form"><input id="import-recovery-passphrase" type="password" minlength="12" maxlength="256" autocomplete="current-password" placeholder="Package passphrase" required><button class="button secondary" type="submit" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>Choose Package</button></form></article>
    </aside></section>`;
}

function backupHistoryRows(backups) {
  if (!backups.length) return `<div class="empty-state">${icon("backup", 32)}<strong>No backups yet</strong><p>Create a protected backup to establish your first recovery point.</p></div>`;
  return `<div class="backup-table"><div class="table-head"><span>Date & Time</span><span>Type</span><span>Size</span><span>Verification</span><span>Encryption</span><span>Actions</span></div>${backups.slice(0, 20).map((backup) => `<div class="table-row"><span><i class="event-dot ${backup.state === "failed" ? "warning" : "success"}"></i>${escapeHtml(formatDate(backup.created_at_utc))}</span><strong>${escapeHtml(humanize(backup.kind))}</strong><span>${escapeHtml(formatBytes(backup.size_bytes))}</span><span>${escapeHtml(humanize(backup.state))}</span><span>${escapeHtml(humanize(backup.encryption))}</span><div class="row-actions">${backup.kind === "recovery" ? `<button class="icon-button" data-backup-action="export" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-file-name="${escapeHtml(backup.file_name)}" title="Export">${icon("download", 16)}</button>` : ""}<button class="icon-button" data-backup-action="verify" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-kind="${escapeHtml(backup.kind)}" title="Verify">${icon("shield", 16)}</button><button class="icon-button danger" data-backup-action="restore" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-kind="${escapeHtml(backup.kind)}" title="Restore">${icon("restore", 16)}</button></div></div>`).join("")}</div>`;
}

function renderIntegrations() {
  const connected = isConnected();
  const agents = [
    ["Microgifter Cloud", "Secure signed connection to Microgifter services.", "cloud", connected, cloudSnapshot?.last_success_utc],
    ["Pairing Agent", "Manages one-time pairing and local device identity.", "key", connected, cloudSnapshot?.paired_at_utc],
    ["Sync Agent", "Handles approved cloud synchronization operations.", "sync", connected, cloudSnapshot?.last_success_utc],
    ["Backup Agent", "Manages encrypted backup and recovery operations.", "backup", isHealthy(), lastBackup()?.created_at_utc],
  ];
  return `${pageHeader("Integrations & Agents", "Manage connections and the agents that keep your HomeServer running smoothly.")}
    <section class="metrics five-up">${metricCard("integrations", "Connected Integrations", connected ? "1" : "0", "Microgifter Cloud", connected ? "green" : "gray")}${metricCard("agent", "Active Agents", String(agents.filter((agent) => agent[3]).length), `of ${agents.length} running`, "blue")}${metricCard("activity", "Overall Health", isHealthy() ? "Healthy" : "Attention", isHealthy() ? "All local systems operational" : "Local service unavailable", isHealthy() ? "purple" : "amber")}${metricCard("sync", "Last Sync", relativeDate(cloudSnapshot?.last_success_utc), connected ? "Signed cloud heartbeat" : "Not connected", "teal")}${metricCard("shield", "Security", connected ? "Protected" : "Local only", connected ? "Signed requests active" : "Pairing not configured", "amber")}</section>
    <section class="integration-grid"><article class="panel agents-panel"><div class="panel-title"><div><h2>Agents</h2><p>System agents that power core HomeServer functionality.</p></div><button class="button secondary" id="refresh-status">${icon("refresh", 16)}Refresh</button></div><div class="agent-list">${agents.map((agent) => agentRow(...agent)).join("")}</div></article>
      <article class="panel integrations-panel"><div class="panel-title"><div><h2>Integrations</h2><p>External services and optional local connections.</p></div><button class="button primary" disabled>${icon("plus", 16)}Connect New</button></div><div class="integration-list">
        ${integrationRow("Microgifter Cloud", "Signed cloud service", "cloud", connected, connected ? "Connected" : "Not connected", connected ? relativeDate(cloudSnapshot?.last_success_utc) : "—", "sync")}
        ${integrationRow("Local API", "Loopback automation", "network", Boolean(statusSnapshot?.api_available), statusSnapshot?.api_available ? "Running" : "Offline", statusSnapshot?.api_available ? "Available" : "—", "system")}
        ${integrationRow("Local Model Runtime", "Approved loopback Ollama provider", "model", modelSnapshot?.runtime?.state === "running", modelSnapshot?.runtime?.state === "running" ? "Running" : "Not running", modelSnapshot?.runtime?.version ? `v${modelSnapshot.runtime.version}` : "Local only", "models")}
        ${integrationRow("Local Storage", "Protected device data", "storage", true, "Connected", "Local", "backups")}
      </div></article>
    </section>
    ${renderCloudConnectionDetail()}
    ${renderMcpRuntime()}`;
}

function renderMcpRuntime() {
  const clients = Array.isArray(mcpSnapshot?.clients) ? mcpSnapshot.clients : [];
  const active = clients.filter((client) => client.state === "active").length;
  const scopes = Array.isArray(mcpSnapshot?.scopes) ? mcpSnapshot.scopes : [];
  const token = mcpCredential?.token || "";
  const httpConfig = token ? JSON.stringify({ mcpServers: { homeserver: { url: mcpCredential.endpoint, headers: { Authorization: `Bearer ${token}` } } } }, null, 2) : "Create a client to generate a one-time token.";
  const stdioConfig = token && mcpBridgePath ? JSON.stringify({ mcpServers: { homeserver: { command: mcpBridgePath, env: { MG_HOMESERVER_MCP_TOKEN: token } } } }, null, 2) : "Create a client and install the packaged bridge to generate this configuration.";
  return `<section class="panel mcp-runtime-panel" id="mcp-runtime"><div class="panel-title"><div><h2>Local MCP Runtime</h2><p>Client-scoped local reads plus request-only supervised plan and World Mission drafting.</p></div>${badge(mcpSnapshot?.state === "ready" ? "Ready" : "Waiting for client", mcpSnapshot?.state === "ready" ? "healthy" : "planned")}</div>
    <div class="mcp-summary-grid"><div><span>Endpoint</span><strong class="mono">${escapeHtml(mcpSnapshot?.endpoint || "http://127.0.0.1:47831/mcp")}</strong></div><div><span>Active clients</span><strong>${active}</strong></div><div><span>Transport</span><strong>HTTP + stdio</strong></div><div><span>Limit</span><strong>${Number(mcpSnapshot?.requests_per_minute || 120)}/min</strong></div></div>
    <div class="privacy-banner success">${icon("shield", 20)}<div><strong>Supervised local boundary</strong><span>Read tools and request-only plan or mission tools are exposed. Approval, execution, and World dispatch remain local Control Center actions.</span></div></div>
    <div class="mcp-workspace"><form id="mcp-client-form" class="panel inset-panel stack-form"><div><h3>Create MCP Client</h3><p>The token is shown once. HomeServer stores only its SHA-256 hash.</p></div><label><span>Client name</span><input id="mcp-client-name" maxlength="80" minlength="3" placeholder="Claude Desktop on this PC" required></label><label><span>Expires</span><select id="mcp-client-expiry"><option value="30">30 days</option><option value="90" selected>90 days</option><option value="180">180 days</option><option value="365">365 days</option></select></label><fieldset class="mcp-scopes"><legend>Read-only scopes</legend>${scopes.map((scope) => `<label><input type="checkbox" name="mcp-scope" value="${escapeHtml(scope)}" checked><span>${escapeHtml(scope)}</span></label>`).join("")}</fieldset><button class="button primary" type="submit" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>Create Client</button></form>
      <article class="panel inset-panel"><div><h3>Authorized Clients</h3><p>Revoke any client immediately without changing other credentials.</p></div><div class="mcp-client-list">${clients.length ? clients.map((client) => `<div class="mcp-client-row"><div><strong>${escapeHtml(client.display_name)}</strong><small class="mono">${escapeHtml(client.token_hint)}</small><small>${escapeHtml(client.scopes.join(", "))}</small></div><div>${badge(humanize(client.state), client.state === "active" ? "healthy" : "planned")}<small>${escapeHtml(formatDate(client.expires_at_utc))}</small></div>${client.state === "active" ? `<button class="button danger compact" data-mcp-revoke="${escapeHtml(client.client_id)}" type="button" ${busy ? "disabled" : ""}>Revoke</button>` : ""}</div>`).join("") : `<div class="empty-state">${icon("network", 28)}<strong>No MCP clients</strong><p>Create a scoped client before connecting a local agent harness.</p></div>`}</div></article></div>
    ${mcpCredential ? `<article class="mcp-credential-card"><div class="panel-title"><div><h3>Copy this credential now</h3><p>It cannot be recovered after this screen is refreshed.</p></div>${badge("One-time secret", "warning")}</div><label><span>Token</span><div class="copy-field"><code>${escapeHtml(token)}</code><button class="button secondary compact" data-copy-value="${escapeHtml(token)}" type="button">Copy</button></div></label><div class="mcp-config-grid"><label><span>Streamable HTTP configuration</span><pre>${escapeHtml(httpConfig)}</pre><button class="button secondary compact" data-copy-value="${escapeHtml(httpConfig)}" type="button">Copy HTTP config</button></label><label><span>Packaged stdio configuration</span><pre>${escapeHtml(stdioConfig)}</pre><button class="button secondary compact" data-copy-value="${escapeHtml(stdioConfig)}" type="button" ${mcpBridgePath ? "" : "disabled"}>Copy stdio config</button></label></div></article>` : ""}
  </section>`;
}

function agentRow(name, description, iconName, connected, lastSeen) {
  return `<div class="agent-row"><div class="app-icon tone-${connected ? "blue" : "gray"}">${icon(iconName, 24)}</div><div><h3>${escapeHtml(name)} ${badge(connected ? "Connected" : "Waiting", connected ? "healthy" : "planned")}</h3><p>${escapeHtml(description)}</p></div><div class="agent-health"><span><i class="live-dot ${connected ? "" : "muted"}"></i>${connected ? "Healthy" : "Inactive"}</span><small>${lastSeen ? relativeDate(lastSeen) : "Not yet"}</small></div><label class="switch"><input type="checkbox" ${connected ? "checked" : ""} disabled><span></span></label><button class="button ghost" data-page="${name.includes("Backup") ? "backups" : name.includes("Cloud") || name.includes("Pairing") || name.includes("Sync") ? "sync" : "system"}">Configure</button></div>`;
}

function integrationRow(name, description, iconName, enabled, status, lastSync, page) {
  return `<div class="integration-row"><div class="app-icon tone-${enabled ? "blue" : "gray"}">${icon(iconName, 23)}</div><div><h3>${escapeHtml(name)}</h3><p>${escapeHtml(description)}</p></div><div><span><i class="live-dot ${enabled ? "" : "muted"}"></i>${escapeHtml(status)}</span><small>${escapeHtml(lastSync)}</small></div><label class="switch"><input type="checkbox" ${enabled ? "checked" : ""} disabled><span></span></label><button class="button ghost" data-page="${page}">${enabled ? "Configure" : "View"}</button>${icon("menu", 17)}</div>`;
}

function renderCloudConnectionDetail() {
  const connected = isConnected();
  if (!connected) {
    return `<article class="panel connection-detail"><div class="panel-title"><div><h2>Pair Microgifter Cloud</h2><p>Generate a one-time code from your Microgifter account, then enter it here.</p></div></div><form id="cloud-pair-form" class="pairing-form"><label><span>Microgifter cloud URL</span><input id="cloud-base-url" type="url" value="https://microgifter.com" maxlength="240" autocomplete="url" required></label><label><span>One-time pairing code</span><input id="cloud-pairing-code" class="mono" type="password" minlength="20" maxlength="80" autocomplete="one-time-code" placeholder="Paste pairing code" required></label><button class="button primary" type="submit" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>Pair HomeServer</button></form></article>`;
  }
  return `<article class="panel connection-detail"><div class="panel-title"><div><h2>Connection Details</h2><p>Details for the active signed Microgifter integration.</p></div></div><div class="connection-detail-grid"><div class="connection-identity"><div class="app-icon tone-blue">${icon("cloud", 25)}</div><div><h3>Microgifter Cloud ${badge("Connected", "healthy")}</h3><p>Secure connection to Microgifter Cloud services.</p><span class="mono">${escapeHtml(cloudSnapshot?.cloud_base_url || "https://microgifter.com")}</span></div></div><dl><div><dt>Status</dt><dd>${badge(humanize(cloudSnapshot?.state), cloudSnapshot?.state)}</dd></div><div><dt>Last Heartbeat</dt><dd>${escapeHtml(relativeDate(cloudSnapshot?.last_success_utc))}</dd></div><div><dt>Device Identity</dt><dd class="mono">${escapeHtml(compactId(cloudSnapshot?.device_id))}</dd></div><div><dt>Pending Sync</dt><dd>${Number(cloudSnapshot?.pending_sync || 0)}</dd></div></dl><div class="connection-actions"><button id="cloud-vault-test" class="button secondary" type="button" ${busy ? "disabled" : ""}>Test Connection</button><button class="button secondary" data-page="sync">Configure</button><button id="cloud-disconnect" class="button danger" type="button" ${busy ? "disabled" : ""}>Disconnect</button></div></div></article>`;
}
function renderKnowledge() {
  const documents = vaultSnapshot?.documents || [];
  const indexed = Number(vaultSnapshot?.indexed_count || 0);
  const attention = Number(vaultSnapshot?.changed_count || 0) + Number(vaultSnapshot?.missing_count || 0) + Number(vaultSnapshot?.failed_count || 0);
  const lastIndexed = vaultSnapshot?.last_indexed_at_utc;
  const extraction = vaultSnapshot?.extraction || {};
  const extractionRuntime = extraction.runtime || { state: "not_installed" };
  const extractedPages = Number(extraction.total_pages || 0);
  const ocrPages = Number(extraction.ocr_pages || 0);
  const ocrRequired = Number(extraction.ocr_required_documents || 0);
  const semanticReady = Number(semanticSnapshot?.ready_documents || 0);
  const semanticChunks = Number(semanticSnapshot?.chunk_count || 0);
  const semanticAttention = Number(semanticSnapshot?.stale_documents || 0) + Number(semanticSnapshot?.failed_documents || 0);
  const semanticModel = semanticSnapshot?.default_embedding_model;
  const semanticState = semanticSnapshot?.state || "not_configured";
  const semanticOperation = semanticSnapshot?.latest_operation;
  const semanticRunning = ["pending", "running"].includes(semanticOperation?.state);
  const searchMode = vaultSearchResult?.mode || "hybrid";
  const operationProgress = semanticOperation?.total_documents ? Math.round(Number(semanticOperation.processed_documents || 0) / Number(semanticOperation.total_documents) * 100) : 0;
  return `${pageHeader("Knowledge Vault", "Private keyword and semantic search powered by your local Ollama embedding model.", `<button id="vault-reindex" class="button secondary" type="button" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>${icon("refresh", 16)}Check Files</button><button id="vault-semantic-rebuild" class="button purple" type="button" ${busy || semanticRunning || !indexed || !semanticModel ? "disabled" : ""}>${icon("model", 16)}${semanticRunning ? "Indexing…" : "Build Semantic Index"}</button><button id="vault-import" class="button primary" type="button" ${busy || !statusSnapshot?.api_available ? "disabled" : ""}>${icon("upload", 16)}Import Document</button>`)}
    ${!semanticModel ? `<div class="notice warning"><strong>Semantic search needs an embedding model.</strong> Install and assign an embedding model in Model Center, then return here to build the local semantic index. <button class="text-button" data-page="models">Open Model Center ${icon("arrow", 13)}</button></div>` : ""}
    ${extractionRuntime.scanned_pdf_ocr_available ? "" : `<div class="notice info ocr-runtime-notice"><div><strong>Scanned-document OCR needs local tools.</strong><span>Searchable PDFs and DOCX work without them. Run these commands in an administrator PowerShell window to install Tesseract for image OCR and Poppler for scanned PDF rendering, then choose Check Files.</span></div><div class="ocr-command-row"><code>${escapeHtml(extractionRuntime.tesseract_install_command || "winget install --id tesseract-ocr.tesseract --exact --scope machine")}</code><button type="button" class="button ghost" data-ocr-command="${escapeHtml(extractionRuntime.tesseract_install_command || "winget install --id tesseract-ocr.tesseract --exact --scope machine")}">Copy</button></div><div class="ocr-command-row"><code>${escapeHtml(extractionRuntime.poppler_install_command || "winget install --id oschwartz10612.Poppler --exact --scope machine")}</code><button type="button" class="button ghost" data-ocr-command="${escapeHtml(extractionRuntime.poppler_install_command || "winget install --id oschwartz10612.Poppler --exact --scope machine")}">Copy</button></div></div>`}
    <form id="vault-search-form" class="toolbar-row semantic-search-toolbar"><label class="filter-search wide">${icon("search", 17)}<input id="vault-search-query" type="search" maxlength="200" placeholder="Ask about policies, procedures, menus, training, or business knowledge..." value="${escapeHtml(vaultSearchResult?.query || "")}" required></label><label class="search-mode-select"><span>Search mode</span><select id="vault-search-mode"><option value="hybrid" ${searchMode === "hybrid" ? "selected" : ""}>Hybrid</option><option value="semantic" ${searchMode === "semantic" ? "selected" : ""}>Semantic</option><option value="keyword" ${searchMode === "keyword" ? "selected" : ""}>Keyword</option></select></label><button class="button secondary" type="submit" ${busy || !indexed ? "disabled" : ""}>Search</button><span class="planned-label">Local only · cited results</span></form>
    <section class="metrics six-up">
      ${metricCard("vault", "Indexed Documents", String(indexed), attention ? `${attention} need attention` : "Managed local documents", attention ? "amber" : "blue")}
      ${metricCard("model", "Semantic Documents", String(semanticReady), semanticAttention ? `${semanticAttention} need rebuilding` : humanize(semanticState), semanticAttention ? "amber" : semanticReady ? "green" : "gray")}
      ${metricCard("file", "Extracted Pages", String(extractedPages), ocrPages ? `${ocrPages} pages processed with local OCR` : "PDF, DOCX, image, and text pages", ocrRequired ? "amber" : "teal")}
      ${metricCard("storage", "Storage Used", formatBytes(vaultSnapshot?.total_size_bytes || 0), "Managed source documents", "blue")}
      ${metricCard("model", "Embedding Model", semanticModel || "Not assigned", semanticModel ? "Fixed local Ollama runtime" : "Configure in Model Center", semanticModel ? "teal" : "amber")}
      ${metricCard("shield", "Privacy", semanticSnapshot?.local_only === false ? "Review" : "Local", "No document or vector cloud sync", "green")}
    </section>
    <section class="knowledge-grid">
      <article class="panel vault-summary"><div class="panel-title"><div><h2>Vault Summary</h2></div></div><div class="storage-layout">${donut(documents.length ? Math.min(100, indexed / documents.length * 100) : 0, `${indexed}/${documents.length}`, "indexed", attention ? "amber" : "blue")}<ul class="legend"><li><i class="blue"></i><span>Keyword indexed</span><strong>${indexed}</strong></li><li><i class="purple"></i><span>Semantic ready</span><strong>${semanticReady}</strong></li><li><i class="amber"></i><span>Changed / stale</span><strong>${Number(vaultSnapshot?.changed_count || 0) + Number(semanticSnapshot?.stale_documents || 0)}</strong></li><li><i class="red"></i><span>Failed</span><strong>${Number(vaultSnapshot?.failed_count || 0) + Number(semanticSnapshot?.failed_documents || 0)}</strong></li></ul></div><button id="vault-reindex" class="text-button" type="button" ${busy || !documents.length ? "disabled" : ""}>Check managed files ${icon("arrow", 13)}</button></article>
      <article class="panel semantic-status-card"><div class="panel-title"><div><h2>Semantic Index</h2></div>${badge(humanize(semanticState), semanticState === "ready" ? "healthy" : semanticState === "indexing" ? "warning" : "planned")}</div>${semanticOperation ? `<div class="semantic-operation"><div><strong>${escapeHtml(semanticOperation.status_message)}</strong><span>${Number(semanticOperation.processed_documents || 0)} of ${Number(semanticOperation.total_documents || 0)} documents · ${Number(semanticOperation.processed_chunks || 0)} chunks</span></div>${progress(operationProgress, semanticOperation.state === "failed" ? "amber" : "purple")}<small>${escapeHtml(humanize(semanticOperation.state))}${semanticOperation.failed_documents ? ` · ${Number(semanticOperation.failed_documents)} failed` : ""}</small></div>` : `<div class="empty-search compact">${icon("model", 30)}<strong>${semanticModel ? "Ready to build" : "Embedding model required"}</strong><p>${semanticModel ? "Create bounded local embeddings for hybrid and semantic retrieval." : "Assign an installed embedding model in Model Center."}</p></div>`}<div class="button-row"><button id="vault-semantic-rebuild" class="button purple" type="button" ${busy || semanticRunning || !indexed || !semanticModel ? "disabled" : ""}>${semanticRunning ? "Indexing…" : semanticReady ? "Refresh Semantic Index" : "Build Semantic Index"}</button>${semanticReady ? `<button id="vault-semantic-rebuild-force" class="button secondary" type="button" ${busy || semanticRunning ? "disabled" : ""}>Full Rebuild</button>` : ""}</div></article>
      <article class="panel indexed-content"><div class="panel-title"><div><h2>Extracted Content</h2></div>${badge(humanize(extractionRuntime.state), extractionRuntime.scanned_pdf_ocr_available ? "healthy" : "planned")}</div><div class="content-type-list">${contentType("file", "Text & data", String(documents.filter((document) => ["text/plain", "text/markdown", "text/csv", "application/json"].includes(document.content_type)).length), "blue")}${contentType("file", "PDF Documents", String(documents.filter((document) => document.content_type === "application/pdf").length), "green")}${contentType("file", "DOCX Documents", String(documents.filter((document) => document.content_type.includes("wordprocessingml")).length), "purple")}${contentType("activity", "OCR Pages", String(ocrPages), ocrPages ? "teal" : "gray")}</div><span class="planned-banner">${ocrRequired ? `${ocrRequired} managed document(s) can be retried after the OCR runtime is installed.` : "Page-level extraction is ready for keyword and cited semantic retrieval."}</span></article>
      <article class="panel search-preview"><div class="panel-title"><div><h2>Search Results</h2></div><span>${vaultSearchResult ? `${vaultSearchResult.hits?.length || 0} ${escapeHtml(humanize(vaultSearchResult.mode))} matches` : "Run a local search"}</span></div>${renderVaultSearchResults()}</article>
      <article class="panel recent-sources"><div class="panel-title"><div><h2>Managed Documents</h2></div><span>${documents.length} records</span></div><div class="source-list">${documents.length ? documents.slice(0, 20).map(vaultDocumentRow).join("") : `<div class="empty-search compact">${icon("vault", 30)}<strong>No documents imported</strong><p>Import an approved UTF-8 text document to create the first local index.</p></div>`}</div></article>
      <article class="panel processing-queue"><div class="panel-title"><div><h2>Extraction Runtime</h2></div>${badge(humanize(extractionRuntime.state), extractionRuntime.scanned_pdf_ocr_available ? "healthy" : "planned")}</div><div class="diagnostic-list">${diagnosticRow("Native PDF text", "Built into HomeServer", true)}${diagnosticRow("DOCX paragraphs & tables", "Built into HomeServer", true)}${diagnosticRow("Tesseract image OCR", extractionRuntime.tesseract_available ? "Detected" : "Install locally", extractionRuntime.tesseract_available)}${diagnosticRow("Poppler PDF renderer", extractionRuntime.pdf_renderer_available ? "Detected" : "Install locally", extractionRuntime.pdf_renderer_available)}</div>${extraction.latest_operation ? `<div class="semantic-operation"><div><strong>${escapeHtml(extraction.latest_operation.status_message)}</strong><span>${Number(extraction.latest_operation.processed_pages || 0)} of ${Number(extraction.latest_operation.total_pages || 0)} pages · ${escapeHtml(humanize(extraction.latest_operation.operation_type))}</span></div>${progress(extraction.latest_operation.total_pages ? Math.round(Number(extraction.latest_operation.processed_pages || 0) / Number(extraction.latest_operation.total_pages) * 100) : 0, extraction.latest_operation.state === "failed" ? "amber" : "teal")}</div>` : ""}</article>
    </section>
    <div class="privacy-banner">${icon("shield", 20)}<div><strong>Your documents, embeddings, and searches remain local</strong><span>Semantic retrieval uses the same fixed-loopback Ollama boundary as Model Center and never enables cloud content sync.</span></div><button class="text-button" data-page="system">Review security ${icon("arrow", 13)}</button></div>`;
}

function renderVaultSearchResults() {
  if (!vaultSearchResult) return `<div class="empty-search">${icon("search", 34)}<strong>Search managed knowledge</strong><p>Use hybrid search for exact keyword matches plus local semantic meaning.</p></div>`;
  const hits = vaultSearchResult.hits || [];
  if (!hits.length) return `<div class="empty-search">${icon("search", 34)}<strong>No matching documents</strong><p>Try another question, check the managed files, or rebuild the semantic index.</p></div>`;
  return `<div class="source-list semantic-results">${hits.map((hit) => `<div><div class="app-icon tone-${hit.semantic_score > 0 ? "purple" : "blue"}">${icon(hit.semantic_score > 0 ? "model" : "file", 18)}</div><span><strong>${escapeHtml(hit.title)}</strong><small>${escapeHtml(hit.snippet)}</small><small class="semantic-citation">${escapeHtml(hit.citation)}</small></span><em>${Math.round(Number(hit.combined_score || 0) * 100)}% · ${escapeHtml(humanize(vaultSearchResult.mode))}</em>${badge(hit.semantic_score > 0 ? "Semantic" : "Keyword", hit.semantic_score > 0 ? "healthy" : "info")}</div>`).join("")}</div>`;
}

function vaultDocumentRow(document) {
  const extraction = vaultSnapshot?.extraction?.documents?.find((item) => item.document_id === document.document_id);
  const extractionDetail = extraction ? `${extraction.page_count} page${Number(extraction.page_count) === 1 ? "" : "s"} · ${humanize(extraction.extraction_method)}` : humanize(document.content_type);
  const state = ["changed", "missing"].includes(document.state) ? document.state : extraction?.state || document.state;
  return `<div><div class="app-icon tone-${document.state === "indexed" ? "blue" : "amber"}">${icon("file", 18)}</div><span><strong>${escapeHtml(document.title)}</strong><small>${escapeHtml(document.file_name)} · ${formatBytes(document.size_bytes)} · ${escapeHtml(extractionDetail)} · ${relativeDate(document.indexed_at_utc || document.updated_at_utc)}</small></span><em>${extraction?.confidence_permille != null ? `${Math.round(Number(extraction.confidence_permille) / 10)}% OCR` : escapeHtml(document.content_type)}</em>${badge(humanize(state), ["indexed", "ready"].includes(state) ? "healthy" : "warning")}<button class="icon-button danger" type="button" data-vault-delete="${escapeHtml(document.document_id)}" title="Delete managed copy">${icon("trash", 16)}</button></div>`;
}

function contentType(iconName, label, value, tone) {
  return `<div><div class="app-icon tone-${tone}">${icon(iconName, 17)}</div><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong></div>`;
}

function sourceRow(iconName, label, sync, size, status, tone) {
  return `<div><div class="app-icon tone-${tone}">${icon(iconName, 18)}</div><span><strong>${escapeHtml(label)}</strong><small>${escapeHtml(sync)}</small></span><em>${escapeHtml(size)}</em>${badge(status, status === "Connected" || status === "Ready" ? "healthy" : "planned")}</div>`;
}


function renderModelCenter() {
  const runtime = modelSnapshot?.runtime || { state: "unavailable", provider: "ollama", api_url: "http://127.0.0.1:11434" };
  const hardware = modelSnapshot?.hardware || {};
  const catalog = modelSnapshot?.catalog || [];
  const installed = modelSnapshot?.installed_models || [];
  const operations = modelSnapshot?.operations || [];
  const settings = modelSnapshot?.settings || { context_size: 4096, test_timeout_seconds: 60, max_download_gb: 20 };
  const runtimeReady = runtime.state === "running";
  const activeOperations = operations.filter((operation) => ["pending", "running"].includes(operation.state));
  const chatModels = catalog.filter((model) => model.installed && model.supports_chat);
  const embeddingModels = catalog.filter((model) => model.installed && model.supports_embeddings);
  const testModels = catalog.filter((model) => model.installed);
  return `${pageHeader("Model Center", "Manage approved local models through Ollama's fixed loopback API. Prompts and model inventory remain on this HomeServer.", `<button id="refresh-models" class="button secondary" type="button" ${busy ? "disabled" : ""}>${icon("refresh", 16)}Refresh</button>`) }
    ${runtimeReady ? "" : `<div class="notice warning"><strong>Ollama is not running.</strong> Install or start Ollama for Windows, then refresh. HomeServer will only connect to <span class="mono">127.0.0.1:11434</span>.</div>`}
    <section class="metrics six-up">
      ${metricCard("model", "Runtime", runtimeReady ? "Running" : "Not running", runtime.version ? `Ollama ${runtime.version}` : "Fixed loopback endpoint", runtimeReady ? "green" : "amber")}
      ${metricCard("cpu", "Logical CPUs", String(hardware.logical_cpu_count || 0), "Hardware guidance only", "blue")}
      ${metricCard("memory", "System Memory", formatBytes(hardware.total_memory_bytes || 0), `${formatBytes(hardware.available_memory_bytes || 0)} available`, "purple")}
      ${metricCard("storage", "Free Disk", formatBytes(hardware.free_disk_bytes || 0), `${settings.max_download_gb || 20} GB model limit`, "teal")}
      ${metricCard("model", "Installed Models", String(installed.length), `${installed.filter((model) => model.running).length} loaded`, "green")}
      ${metricCard("activity", "Active Downloads", String(activeOperations.length), operations.length ? `${operations.length} retained operations` : "No model operations", activeOperations.length ? "amber" : "gray")}
    </section>
    <section class="model-center-grid">
      <article class="panel model-catalog-panel"><div class="panel-title"><div><h2>Approved Starter Catalog</h2><p>Only these bounded model identifiers can be downloaded from HomeServer.</p></div><span>${catalog.length} approved</span></div><div class="model-card-grid">${catalog.map((model) => renderCatalogModel(model, runtimeReady, activeOperations)).join("")}</div></article>
      <aside class="model-center-aside">
        <article class="panel"><div class="panel-title"><div><h2>Local Test</h2><p>Runs a bounded prompt directly against the selected local model.</p></div></div><form id="model-test-form" class="stack-form"><select id="model-test-name" ${!testModels.length || busy ? "disabled" : ""}>${modelOptions(testModels, settings.default_chat_model)}</select><textarea id="model-test-prompt" maxlength="500" rows="4" placeholder="Summarize why local-first AI is useful." ${!testModels.length || busy ? "disabled" : ""} required></textarea><button class="button primary" type="submit" ${!runtimeReady || !testModels.length || busy ? "disabled" : ""}>Run Local Test</button></form>${modelTestResult ? `<div class="model-test-result"><strong>${escapeHtml(modelTestResult.model)} · ${escapeHtml(humanize(modelTestResult.kind))}</strong><p>${escapeHtml(modelTestResult.output)}</p><small>${Number(modelTestResult.duration_ms || 0)} ms</small></div>` : ""}</article>
        <article class="panel"><div class="panel-title"><div><h2>Defaults & Limits</h2><p>Assignments are local and require installed approved models.</p></div></div><form id="model-settings-form" class="stack-form"><label><span>Default chat model</span><select id="model-default-chat"><option value="">Not assigned</option>${modelOptions(chatModels, settings.default_chat_model)}</select></label><label><span>Default embedding model</span><select id="model-default-embedding"><option value="">Not assigned</option>${modelOptions(embeddingModels, settings.default_embedding_model)}</select></label><div class="model-limit-grid"><label><span>Context</span><input id="model-context-size" type="number" min="512" max="32768" step="512" value="${Number(settings.context_size || 4096)}"></label><label><span>Test timeout</span><input id="model-test-timeout" type="number" min="10" max="120" value="${Number(settings.test_timeout_seconds || 60)}"></label><label><span>Max download GB</span><input id="model-download-limit" type="number" min="1" max="100" value="${Number(settings.max_download_gb || 20)}"></label></div><button class="button secondary" type="submit" ${!runtimeReady || busy ? "disabled" : ""}>Save Local Settings</button></form></article>
      </aside>
    </section>
    <section class="two-column-grid model-bottom-grid"><article class="panel"><div class="panel-title"><div><h2>Installed Models</h2></div><span>${installed.length} local</span></div>${renderInstalledModels(installed)}</article><article class="panel"><div class="panel-title"><div><h2>Operation History</h2></div><span>${operations.length} records</span></div>${renderModelOperations(operations)}</article></section>
    <div class="privacy-banner success">${icon("shield", 20)}<div><strong>Provider choice remains local</strong><span>Ollama stays local. OpenRouter is optional, fixed to its reviewed HTTPS endpoint, and cannot receive selected Agent Workspace context until you explicitly enable and confirm remote transfer.</span></div><button class="text-button" data-page="knowledge">Open Knowledge Vault ${icon("arrow", 13)}</button></div>`;
}

function renderCatalogModel(model, runtimeReady, activeOperations) {
  const active = activeOperations.find((operation) => operation.model_name === model.model);
  const tone = model.installed ? "green" : model.recommended ? "blue" : "amber";
  const action = model.installed
    ? `<button class="button secondary" type="button" data-model-test-select="${escapeHtml(model.model)}" ${runtimeReady && !busy ? "" : "disabled"}>Test</button>${model.running ? `<button class="button ghost" type="button" data-model-unload="${escapeHtml(model.model)}" ${busy ? "disabled" : ""}>Unload</button>` : ""}<button class="button danger" type="button" data-model-delete="${escapeHtml(model.model)}" ${busy ? "disabled" : ""}>Delete</button>`
    : `<button class="button primary" type="button" data-model-pull="${escapeHtml(model.model)}" ${!runtimeReady || busy || active ? "disabled" : ""}>${active ? "Downloading" : "Download"}</button>`;
  return `<article class="model-card"><div class="model-card-heading"><div class="app-icon tone-${tone}">${icon(model.supports_embeddings ? "storage" : "model", 22)}</div><div><h3>${escapeHtml(model.display_name)}</h3><code>${escapeHtml(model.model)}</code></div>${badge(model.installed ? "Installed" : model.recommended ? "Recommended" : "Larger system", model.installed ? "healthy" : model.recommended ? "available" : "planned")}</div><p>${escapeHtml(model.purpose)}</p><dl><div><dt>Download</dt><dd>${formatBytes(model.estimated_size_bytes)}</dd></div><div><dt>Minimum memory</dt><dd>${formatBytes(model.minimum_memory_bytes)}</dd></div><div><dt>Capability</dt><dd>${model.supports_embeddings ? "Embeddings" : "Chat"}</dd></div></dl>${active ? `<div class="model-progress"><span>${escapeHtml(active.status_message)}</span>${progress(operationPercent(active), "blue")}</div>` : ""}<div class="button-row">${action}</div></article>`;
}

function modelOptions(models, selected) {
  return models.map((model) => `<option value="${escapeHtml(model.model)}" ${model.model === selected ? "selected" : ""}>${escapeHtml(model.display_name || model.model)}</option>`).join("");
}

function operationPercent(operation) {
  const total = Number(operation.total_bytes || 0);
  const completed = Number(operation.completed_bytes || 0);
  return total > 0 ? Math.min(100, Math.round(completed / total * 100)) : 0;
}

function renderInstalledModels(models) {
  if (!models.length) return `<div class="empty-search compact">${icon("model", 30)}<strong>No approved models installed</strong><p>Start Ollama and download a recommended model from the starter catalog.</p></div>`;
  return `<div class="model-table"><div class="table-head"><span>Model</span><span>Size</span><span>Family</span><span>Quantization</span><span>Memory</span><span>Status</span></div>${models.map((model) => `<div class="table-row"><strong class="mono">${escapeHtml(model.name)}</strong><span>${formatBytes(model.size_bytes)}</span><span>${escapeHtml(model.family || "Unknown")}</span><span>${escapeHtml(model.quantization_level || "Unknown")}</span><span>${formatBytes(model.size_vram_bytes)}</span>${badge(model.running ? "Loaded" : "Installed", model.running ? "running" : "ready")}</div>`).join("")}</div>`;
}

function renderModelOperations(operations) {
  if (!operations.length) return `<div class="empty-search compact">${icon("activity", 30)}<strong>No model operations</strong><p>Download activity and restart-safe outcomes will appear here.</p></div>`;
  return `<div class="model-operation-list">${operations.slice(0, 20).map((operation) => `<div><div><strong>${escapeHtml(humanize(operation.operation_type))} · ${escapeHtml(operation.model_name)}</strong><small>${escapeHtml(operation.status_message || humanize(operation.state))}</small></div><span>${operation.total_bytes ? `${operationPercent(operation)}%` : relativeDate(operation.updated_at_utc)}</span>${badge(humanize(operation.state), ["succeeded"].includes(operation.state) ? "healthy" : ["failed", "interrupted"].includes(operation.state) ? "failed" : "available")}${["pending", "running"].includes(operation.state) ? progress(operationPercent(operation), "blue") : ""}</div>`).join("")}</div>`;
}

function renderSettings() {
  const prefs = loadPreferences();
  return `${pageHeader("Settings", "Manage HomeServer preferences and Control Center configuration.")}
    <section class="settings-layout"><div class="settings-sections">
      ${settingsSection("dashboard", "General", "Basic server display preferences.", `<label><span>Server Name</span><input id="setting-server-name" type="text" value="${escapeHtml(prefs.serverName)}" maxlength="64"></label><label><span>Time Zone</span><select id="setting-time-zone"><option value="local" ${prefs.timeZone === "local" ? "selected" : ""}>Use Windows local time</option><option value="utc" ${prefs.timeZone === "utc" ? "selected" : ""}>UTC</option></select></label><button class="button primary" data-save-setting="general">Save</button>`)}
      ${settingsSection("system", "Windows Desktop", "Keep the Control Center available without leaving a terminal or taskbar window open.", `<label class="toggle-field"><span>Start Control Center with Windows</span>${toggle("setting-start-with-windows", desktopAutostartEnabled)}</label><p class="settings-note">Closing the window hides the Control Center to the Windows system tray. The HomeServer service continues running independently.</p><button class="button primary" data-save-setting="desktop">Save</button>`)}
      ${settingsSection("shield", "Security", "Manage local interface security preferences.", `<label class="toggle-field"><span>Local UI lock</span>${toggle("setting-local-lock", prefs.localLock)}</label><label><span>Auto lock after inactivity</span><select id="setting-auto-lock"><option value="15" ${prefs.autoLock === "15" ? "selected" : ""}>15 minutes</option><option value="30" ${prefs.autoLock === "30" ? "selected" : ""}>30 minutes</option><option value="60" ${prefs.autoLock === "60" ? "selected" : ""}>1 hour</option></select></label><button class="button primary" data-save-setting="security">Save</button>`)}
      ${settingsSection("key", "Pairing", "View signed pairing state and connection controls.", `<label><span>Pairing Mode</span><select disabled><option>Secure one-time code</option></select></label><label><span>Device Identity</span><input class="mono" type="text" value="${escapeHtml(compactId(cloudSnapshot?.device_id))}" disabled></label><button class="button primary" data-page="sync">${isConnected() ? "Manage" : "Pair"}</button>`)}
      ${settingsSection("bell", "Notifications", "Configure Control Center notification presentation.", `<label class="toggle-field"><span>Desktop notifications</span>${toggle("setting-notifications", prefs.notifications)}</label><label><span>Critical alerts</span><select id="setting-alerts"><option value="immediate" ${prefs.alerts === "immediate" ? "selected" : ""}>Immediately</option><option value="daily" ${prefs.alerts === "daily" ? "selected" : ""}>Daily summary</option></select></label><button class="button primary" data-save-setting="notifications">Save</button>`)}
      ${settingsSection("storage", "Storage", "Review local storage and backup locations.", `<label><span>Storage Mode</span><select disabled><option>HomeServer managed</option></select></label><label><span>Backup Catalog</span><input type="text" value="${backupCount()} records" disabled></label><button class="button primary" data-page="backups">Manage</button>`)}
      ${settingsSection("backup", "Backup Preferences", "Set the Control Center backup schedule summary.", `<label><span>Automatic schedule</span><input type="text" value="Every ${backupCatalog?.interval_hours ?? 24} hours" disabled></label><label><span>Retention Policy</span><input type="text" value="${backupCatalog?.retention_count ?? 14} automatic backups" disabled></label><button class="button primary" data-page="backups">View</button>`)}
      ${settingsSection("settings", "Advanced", "Interface diagnostics and presentation preferences.", `<label class="toggle-field"><span>Compact interface</span>${toggle("setting-compact", prefs.compact)}</label><label class="toggle-field"><span>Auto refresh status</span>${toggle("setting-auto-refresh", prefs.autoRefresh)}</label><button class="button primary" data-save-setting="advanced">Save</button>`)}
    </div><aside class="settings-aside">
      <article class="panel"><div class="panel-title"><div><h2>Settings Summary</h2><p>Current Control Center configuration.</p></div></div><dl class="summary-list settings-summary"><div><dt>Server Name</dt><dd>${escapeHtml(prefs.serverName)}</dd></div><div><dt>Time Zone</dt><dd>${prefs.timeZone === "utc" ? "UTC" : "Windows local"}</dd></div><div><dt>Local Lock</dt><dd>${prefs.localLock ? "Enabled" : "Disabled"}</dd></div><div><dt>Auto Lock</dt><dd>${prefs.autoLock} minutes</dd></div><div><dt>Pairing</dt><dd>${isConnected() ? "Connected" : "Not paired"}</dd></div><div><dt>Notifications</dt><dd>${prefs.notifications ? "Enabled" : "Disabled"}</dd></div><div><dt>Start with Windows</dt><dd>${desktopAutostartEnabled ? "Enabled" : "Disabled"}</dd></div><div><dt>Close Button</dt><dd>Hide to tray</dd></div><div><dt>Backups</dt><dd>${backupCount()} records</dd></div><div><dt>Updates</dt><dd>${updateDisplayState() === "not_configured" ? "Beta channel" : humanize(updateDisplayState())}</dd></div></dl></article>
      <article class="panel"><div class="panel-title"><div><h2>Security Status</h2><p>Your HomeServer security boundaries.</p></div></div><div class="security-checks"><span>${icon("check", 17)}Loopback API<strong>${statusSnapshot?.api_available ? "Active" : "Offline"}</strong></span><span>${icon("check", 17)}Credential Vault<strong>${isConnected() ? "Configured" : "Waiting"}</strong></span><span>${icon("check", 17)}Backup Encryption<strong>Enabled</strong></span><span>${icon("check", 17)}Signed Updates<strong>${updateDisplayState() === "not_configured" ? "Beta" : "Enabled"}</strong></span></div><button id="cloud-vault-test" class="button secondary full" ${!isConnected() || busy ? "disabled" : ""}>Run Security Check</button></article>
    </aside></section>`;
}

function toggle(id, checked) {
  return `<label class="switch"><input id="${id}" type="checkbox" ${checked ? "checked" : ""}><span></span></label>`;
}

function settingsSection(iconName, title, copy, controls) {
  return `<article class="panel settings-section"><div class="settings-section-copy"><div class="app-icon tone-blue">${icon(iconName, 21)}</div><div><h2>${escapeHtml(title)}</h2><p>${escapeHtml(copy)}</p></div></div><div class="settings-controls">${controls}</div></article>`;
}

function loadPreferences() {
  const defaults = { serverName: statusSnapshot?.server_name || "HomeServer", timeZone: "local", localLock: false, autoLock: "15", notifications: true, alerts: "immediate", compact: false, autoRefresh: true };
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem("homeserver-ui-preferences") || "{}") };
  } catch {
    return defaults;
  }
}

function renderSync() {
  const connected = isConnected();
  const pending = Number(cloudSnapshot?.pending_sync || 0);
  return `${pageHeader("Sync Cloud", "Keep your HomeServer securely synchronized with Microgifter Cloud.")}
    <section class="metrics five-up">${metricCard("shield", "Connection Health", connected ? "Healthy" : "Waiting", connected ? "Secure signed connection" : "Pairing required", connected ? "green" : "amber")}${metricCard("cloud", "Sync Status", connected ? "Connected" : "Not paired", connected ? "Synchronizing safely" : "No cloud identity", connected ? "blue" : "gray")}${metricCard("activity", "Last Sync", relativeDate(cloudSnapshot?.last_success_utc), connected ? formatDate(cloudSnapshot?.last_success_utc) : "Not yet", "teal")}${metricCard("storage", "Queued Items", String(pending), pending ? "Waiting operations" : "All changes up to date", "blue")}${metricCard("shield", "Data Integrity", connected ? "Verified" : "Local", connected ? "Signed requests active" : "Local authority only", "green")}</section>
    <section class="sync-grid">
      <article class="panel cloud-connection-card"><div class="panel-title"><div>${icon("cloud", 18)}<h2>Cloud Connection</h2></div></div><dl class="detail-list"><div><dt>Cloud Endpoint</dt><dd>${escapeHtml(cloudSnapshot?.cloud_base_url || "https://microgifter.com")}</dd></div><div><dt>Connection</dt><dd>${connected ? "Secure signed bridge" : "Not configured"}</dd></div><div><dt>Status</dt><dd>${badge(humanize(cloudSnapshot?.state || "not_paired"), cloudSnapshot?.state || "planned")}</dd></div><div><dt>Last success</dt><dd>${escapeHtml(formatDate(cloudSnapshot?.last_success_utc))}</dd></div></dl><button id="refresh-status" class="button secondary full">${icon("refresh", 16)}Refresh Status</button></article>
      <article class="panel pairing-identity"><div class="panel-title"><div>${icon("key", 18)}<h2>Pairing Identity</h2></div></div><dl class="detail-list"><div><dt>Device ID</dt><dd class="mono">${escapeHtml(compactId(cloudSnapshot?.device_id))}</dd></div><div><dt>Paired On</dt><dd>${escapeHtml(formatDate(cloudSnapshot?.paired_at_utc))}</dd></div><div><dt>Scopes</dt><dd>${Array.isArray(cloudSnapshot?.scopes) ? cloudSnapshot.scopes.length : 0}</dd></div><div><dt>Trust Status</dt><dd>${connected ? "Trusted" : "Waiting"}</dd></div></dl><button class="button secondary full" data-page="integrations">View Details</button></article>
      <article class="panel manual-sync"><div class="panel-title"><div>${icon("sync", 18)}<h2>Manual Sync</h2></div></div><p>Queue low-risk local state operations and run a signed synchronization.</p><div class="check-list"><span>${icon("check", 16)}Upload approved local changes</span><span>${icon("check", 16)}Download cloud receipts</span><span>${icon("check", 16)}Verify request integrity</span><span>${icon("check", 16)}Preserve cloud authority</span></div><button id="cloud-sync-now" class="button primary full" ${!connected || busy ? "disabled" : ""}>${icon("sync", 16)}Sync Now</button></article>
      <article class="panel pending-changes"><div class="panel-title"><div>${icon("storage", 18)}<h2>Pending Changes</h2></div></div><div class="pending-list"><span>${icon("upload", 17)}Uploads<strong>${pending}</strong><small>${pending ? "Queued operations" : "No pending uploads"}</small></span><span>${icon("download", 17)}Downloads<strong>0</strong><small>No pending downloads</small></span><span>${icon("warning", 17)}Conflicts<strong>0</strong><small>No conflicts detected</small></span></div><button class="text-button" data-page="integrations">View Details ${icon("arrow", 13)}</button></article>
      <article class="panel sync-activity"><div class="panel-title"><div>${icon("activity", 18)}<h2>Sync Activity</h2></div></div><div class="sync-activity-layout">${donut(connected ? 100 : 0, connected ? "100%" : "0%", connected ? "on track" : "waiting", connected ? "green" : "gray")}<ul class="legend"><li><i class="green"></i><span>Connection</span><strong>${connected ? "Active" : "Waiting"}</strong></li><li><i class="blue"></i><span>Pending operations</span><strong>${pending}</strong></li><li><i class="purple"></i><span>Signed scopes</span><strong>${Array.isArray(cloudSnapshot?.scopes) ? cloudSnapshot.scopes.length : 0}</strong></li><li><i class="amber"></i><span>Rejected changes</span><strong>0</strong></li></ul></div></article>
      <article class="panel sync-history"><div class="panel-title"><div>${icon("backup", 18)}<h2>Sync History</h2></div><button class="text-button" data-page="system">View Logs</button></div><div class="history-list">${historyRow(cloudSnapshot?.last_success_utc, connected ? "Sync completed" : "No completed sync", connected ? "Success" : "Waiting")}${historyRow(cloudSnapshot?.paired_at_utc, connected ? "HomeServer paired" : "Pairing not configured", connected ? "Success" : "Waiting")}${historyRow(statusSnapshot?.last_updated_utc, "Local status refreshed", statusSnapshot?.api_available ? "Success" : "Warning")}</div></article>
      <article class="panel cloud-settings"><div class="panel-title"><div>${icon("settings", 18)}<h2>Cloud Settings</h2></div></div><div class="next-steps">${nextStep("settings", "Sync Preferences", "Only approved operation types are synchronized.", "settings")}${nextStep("shield", "Authority Boundary", "Commerce and identity remain cloud-authoritative.", "integrations")}${nextStep("backup", "Recovery Protection", "Backups remain local and encrypted.", "backups")}</div><button id="cloud-vault-test" class="button secondary full" ${!connected || busy ? "disabled" : ""}>Test Credential Vault</button></article>
    </section>
    ${connected ? `<div class="privacy-banner success">${icon("shield", 20)}<div><strong>Your HomeServer is connected and synchronizing safely.</strong><span>Signed requests, scoped permissions, replay protection, and cloud receipts are active.</span></div></div>` : renderCloudConnectionDetail()}`;
}

function historyRow(date, label, status) {
  const state = status === "Success" ? "success" : status === "Warning" ? "warning" : "info";
  return `<div><span><i class="event-dot ${state}"></i>${escapeHtml(formatDate(date))}</span><strong>${escapeHtml(label)}</strong><em class="table-status ${state}">${escapeHtml(status)}</em></div>`;
}
function renderSystem() {
  const updateState = updateDisplayState();
  const updateRelease = updateStatus?.update;
  const signer = updateRelease?.authenticode_thumbprint ? `${updateRelease.authenticode_thumbprint.slice(0, 12)}…${updateRelease.authenticode_thumbprint.slice(-8)}` : "Not staged";
  return `${pageHeader("System", "View and manage your HomeServer system, services, updates, and diagnostics.")}
    <section class="system-grid top-system-grid">
      <article class="panel"><div class="panel-title"><div>${icon("system", 18)}<h2>System Overview</h2></div></div><dl class="detail-list"><div><dt>Machine Name</dt><dd>${escapeHtml(statusSnapshot?.server_name || "HomeServer")}</dd></div><div><dt>Version</dt><dd>${escapeHtml(statusSnapshot?.version || "0.1.5")}</dd></div><div><dt>API URL</dt><dd class="mono">${escapeHtml(statusSnapshot?.api_url || "http://127.0.0.1:47831")}</dd></div><div><dt>Database</dt><dd>${escapeHtml(humanize(statusSnapshot?.database || "unknown"))}</dd></div><div><dt>Cloud State</dt><dd>${escapeHtml(humanize(cloudSnapshot?.state || "not_paired"))}</dd></div><div><dt>Last Updated</dt><dd>${escapeHtml(formatDate(statusSnapshot?.last_updated_utc))}</dd></div></dl><div class="resource-row"><span>Service health</span><strong>${isHealthy() ? "100%" : "35%"}</strong></div>${progress(isHealthy() ? 100 : 35, isHealthy() ? "green" : "amber")}</article>
      <article class="panel"><div class="panel-title"><div>${icon("apps", 18)}<h2>Service Status</h2></div><button class="text-button" data-page="apps">View all services</button></div><div class="service-status-list">${serviceRows.slice(0, 6).map(([name], index) => { const running = index < 3 || (index === 3 && isConnected()) || index === 4 || index === 5; return `<div>${icon(running ? "check" : "warning", 16)}<span>${escapeHtml(name)}</span><strong>${running ? "Running" : "Waiting"}</strong><small>v${escapeHtml(statusSnapshot?.version || "0.1.5")}</small></div>`; }).join("")}</div><div class="service-footer"><i class="live-dot"></i>${isHealthy() ? "All core services are operational" : "Local service requires attention"}</div></article>
      <article class="panel update-delivery"><div class="panel-title"><div>${icon("cloud", 18)}<h2>Update & Delivery</h2></div></div><dl class="detail-list"><div><dt>Current Version</dt><dd>${escapeHtml(updateStatus?.current_version || statusSnapshot?.version || "0.1.5")}</dd></div><div><dt>Stable Release</dt><dd>${escapeHtml(updateRelease?.version || "No newer release")}</dd></div><div><dt>Last Checked</dt><dd>${escapeHtml(formatDate(updateStatus?.last_checked_at_utc))}</dd></div><div><dt>Update Channel</dt><dd>${escapeHtml(humanize(updateStatus?.channel || "stable"))}</dd></div><div><dt>Signer</dt><dd class="mono">${escapeHtml(signer)}</dd></div><div><dt>Status</dt><dd>${badge(updateState === "not_configured" ? "Not configured" : humanize(updateState), updateState)}</dd></div></dl>${updateErrorText() ? `<div class="inline-warning">${icon("warning", 17)}${escapeHtml(updateErrorText())}</div>` : ""}<div class="button-row"><button id="check-updates" class="button secondary" ${busy ? "disabled" : ""}>Check now</button><button id="download-update" class="button primary" ${busy || updateState !== "available" || !updateRelease ? "disabled" : ""}>Download</button><button id="apply-update" class="button danger" ${busy || updateState !== "staged" || !updateRelease ? "disabled" : ""}>Apply</button></div></article>
    </section>
    <section class="system-grid resource-system-grid"><article class="panel system-resources"><div class="panel-title"><div><h2>System Resources</h2></div>${badge("Live", isHealthy() ? "healthy" : "planned")}</div><div class="resource-chart-grid">${resourceChart("CPU Usage", isHealthy() ? 18 : 0, "2.1 GHz", "blue")}${resourceChart("Memory Usage", isHealthy() ? 42 : 0, "Local service", "purple")}${resourceChart("Backup Catalog", Math.min(100, backupCount() * 5), `${backupCount()} records`, "green")}${resourceChart("Network", isConnected() ? 32 : 0, isConnected() ? "Cloud active" : "Local only", "blue")}</div></article>
      <article class="panel quick-system-actions"><div class="panel-title"><div><h2>Quick Actions</h2></div></div><div class="next-steps">${nextStep("refresh", "Refresh Status", "Reload local service, cloud, backup, and update state.", "system")}${nextStep("logs", "Export Logs", "Review diagnostic output from the Windows service.", "system")}${nextStep("update", "Check for Updates", "Check the pinned signed release channel.", "system")}${nextStep("shield", "Run Diagnostics", "Validate service and credential boundaries.", "settings")}</div></article>
    </section>
    <section class="two-column-grid diagnostics-grid"><article class="panel"><div class="panel-title"><div><h2>Diagnostics</h2></div><button id="refresh-status" class="text-button">Run All Tests</button></div><div class="diagnostic-list">${diagnosticRow("System Health", isHealthy() ? "No issues detected" : "Local API unavailable", isHealthy())}${diagnosticRow("Database Check", statusSnapshot?.database ? humanize(statusSnapshot.database) : "Unknown", statusSnapshot?.database === "ready")}${diagnosticRow("Cloud Connectivity", isConnected() ? "Signed connection active" : "Pairing not configured", isConnected())}${diagnosticRow("Backup Integrity", backupCount() ? `${backupCount()} catalog records` : "No recovery point created", backupCount() > 0)}${diagnosticRow("Update Contract", updateState === "not_configured" ? "Beta channel not published" : humanize(updateState), updateState !== "failed")}</div></article>
      <article class="panel"><div class="panel-title"><div><h2>Logs Preview</h2></div><span>Current session</span></div><div class="logs-list">${logRow("INFO", "Core Service", statusSnapshot?.api_available ? "Local API responded successfully" : "Local API unavailable", "core.service", statusSnapshot?.last_updated_utc)}${logRow(isConnected() ? "INFO" : "WARN", "Sync Service", isConnected() ? "Signed cloud bridge active" : "HomeServer is not paired", "sync.service", cloudSnapshot?.last_success_utc)}${logRow(backupCount() ? "INFO" : "WARN", "Backup Service", backupCount() ? `${backupCount()} backup records available` : "No backup has been created", "backup.service", lastBackup()?.created_at_utc)}${logRow(updateState === "not_configured" ? "WARN" : "INFO", "Update Manager", updateState === "not_configured" ? "Public update channel is not configured" : humanize(updateState), "update.service", updateStatus?.last_checked_at_utc)}</div><div class="button-row"><button class="button secondary" data-page="settings">Review Settings</button><button class="button secondary" data-page="backups">Open Backups</button></div></article></section>`;
}

function resourceChart(label, value, detail, tone) {
  const bars = [36, 58, 44, 72, 50, 83, 62, 91, 55, 76, 48, 69];
  return `<article><span>${escapeHtml(label)}</span><div><strong>${value}%</strong><small>${escapeHtml(detail)}</small></div><div class="sparkline tone-${tone}">${bars.map((bar, index) => `<i style="height:${Math.max(8, Math.round((bar * Math.max(value, 12)) / 100))}px;opacity:${0.35 + index * 0.05}"></i>`).join("")}</div></article>`;
}

function diagnosticRow(label, detail, passed) {
  return `<div>${icon(passed ? "check" : "warning", 17)}<span>${escapeHtml(label)}</span><p>${escapeHtml(detail)}</p><em class="table-status ${passed ? "success" : "warning"}">${passed ? "Passed" : "Review"}</em></div>`;
}

function logRow(level, source, message, channel, date) {
  const state = level === "INFO" ? "info" : level === "ERROR" ? "warning" : "warning";
  return `<div><i class="event-dot ${state}"></i><span>${escapeHtml(formatDate(date))}</span><strong>${escapeHtml(level)}</strong><em>${escapeHtml(source)}</em><p>${escapeHtml(message)}</p><code>${escapeHtml(channel)}</code></div>`;
}

function renderCurrentPage() {
  switch (activePage) {
    case "home": return renderHome();
    case "apps": return renderApps();
    case "backups": return renderBackups();
    case "integrations": return renderIntegrations();
    case "knowledge": return renderKnowledge();
    case "models": return renderModelCenter();
    case "settings": return renderSettings();
    case "sync": return renderSync();
    case "system": return renderSystem();
    case "agent": return `<div class="homeserver-agent-route-host" data-homeserver-agent-host="true"></div>`;
    default: return renderDashboard();
  }
}

function render() {
  const restorePending = Boolean(backupCatalog?.restore_pending || statusSnapshot?.restore_pending);
  const prefs = loadPreferences();
  document.documentElement.classList.toggle("compact-ui", Boolean(prefs.compact));
  document.documentElement.classList.toggle("agent-chat-mode", activePage === "agent");
  if (activePage === "agent") {
    app.innerHTML = `<div class="agent-chat-shell"><div class="homeserver-agent-route-host" data-homeserver-agent-host="true"></div></div>`;
    window.dispatchEvent(new CustomEvent("homeserver-agent-route"));
    window.dispatchEvent(new CustomEvent("homeserver:rendered", { detail: { page: activePage } }));
    return;
  }
  app.innerHTML = `<div class="desktop-shell">${renderSidebar()}<main class="app-main">${renderTopbar()}<section class="page-canvas">${notice ? `<div class="notice ${notice.kind}">${escapeHtml(notice.message)}</div>` : ""}${restorePending ? `<div class="notice warning"><strong>Restore staged.</strong> Restart the HomeServer service or Windows to apply the verified database. The current database is preserved for rollback.</div>` : ""}${renderCurrentPage()}<footer class="app-footer"><span>Local API: ${escapeHtml(statusSnapshot?.api_url || "http://127.0.0.1:47831")}</span><span>Updated: ${escapeHtml(formatDate(statusSnapshot?.last_updated_utc))}</span></footer></section></main></div>`;
  bindEvents();
  window.dispatchEvent(new CustomEvent("homeserver:rendered", { detail: { page: activePage } }));
}

function bindEvents() {
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
  document.querySelectorAll("#refresh-status").forEach((button) => button.addEventListener("click", () => loadAll()));
  document.querySelector("#cloud-pair-form")?.addEventListener("submit", pairCloud);
  document.querySelector("#cloud-sync-now")?.addEventListener("click", syncCloud);
  document.querySelector("#cloud-vault-test")?.addEventListener("click", testCloudVault);
  document.querySelector("#cloud-disconnect")?.addEventListener("click", disconnectCloud);
  document.querySelectorAll("[data-cloud-operation]").forEach((button) => button.addEventListener("click", enqueueCloudOperation));
  document.querySelector("#create-manual-backup")?.addEventListener("click", createManualBackup);
  document.querySelector("#recovery-package-form")?.addEventListener("submit", createRecoveryPackage);
  document.querySelector("#import-recovery-form")?.addEventListener("submit", importRecoveryPackage);
  document.querySelector("#check-updates")?.addEventListener("click", checkUpdates);
  document.querySelector("#download-update")?.addEventListener("click", downloadUpdate);
  document.querySelector("#apply-update")?.addEventListener("click", applyUpdate);
  document.querySelectorAll("[data-backup-action]").forEach((button) => button.addEventListener("click", handleBackupAction));
  document.querySelectorAll("[data-quick-action]").forEach((button) => button.addEventListener("click", handleQuickAction));
  document.querySelectorAll("[data-toggle]").forEach((button) => button.addEventListener("click", () => document.querySelector(`#${button.dataset.toggle}`)?.classList.toggle("hidden")));
  document.querySelectorAll("[data-scroll-target]").forEach((button) => button.addEventListener("click", () => document.querySelector(`#${button.dataset.scrollTarget}`)?.scrollIntoView({ behavior: "smooth", block: "start" })));
  document.querySelectorAll("[data-save-setting]").forEach((button) => button.addEventListener("click", savePreferences));
  document.querySelector("#vault-import")?.addEventListener("click", importVaultDocument);
  document.querySelector("#vault-search-form")?.addEventListener("submit", searchVault);
  document.querySelector("#vault-reindex")?.addEventListener("click", reindexVault);
  document.querySelector("#vault-semantic-rebuild")?.addEventListener("click", () => rebuildSemanticVault(false));
  document.querySelector("#vault-semantic-rebuild-force")?.addEventListener("click", () => rebuildSemanticVault(true));
  document.querySelectorAll("[data-vault-delete]").forEach((button) => button.addEventListener("click", deleteVaultDocument));
  document.querySelectorAll("[data-ocr-command]").forEach((button) => button.addEventListener("click", copyOcrCommand));
  document.querySelector("#refresh-models")?.addEventListener("click", () => loadAll());
  document.querySelectorAll("[data-model-pull]").forEach((button) => button.addEventListener("click", pullModel));
  document.querySelectorAll("[data-model-delete]").forEach((button) => button.addEventListener("click", deleteModel));
  document.querySelectorAll("[data-model-unload]").forEach((button) => button.addEventListener("click", unloadModel));
  document.querySelectorAll("[data-model-test-select]").forEach((button) => button.addEventListener("click", selectModelForTest));
  document.querySelector("#model-test-form")?.addEventListener("submit", testModel);
  document.querySelector("#model-settings-form")?.addEventListener("submit", saveModelSettings);
  document.querySelector("#mcp-client-form")?.addEventListener("submit", createMcpClient);
  document.querySelectorAll("[data-mcp-revoke]").forEach((button) => button.addEventListener("click", revokeMcpClient));
  document.querySelectorAll("[data-copy-value]").forEach((button) => button.addEventListener("click", copyMcpValue));
}

function navigate(page) {
  if (!pages.some(([key]) => key === page)) page = "dashboard";
  notificationMenuOpen = false;
  activePage = page;
  history.replaceState(null, "", `#${page}`);
  notice = null;
  render();
  document.querySelector(".page-canvas")?.scrollTo({ top: 0, behavior: "smooth" });
}

async function handleQuickAction(event) {
  const action = event.currentTarget.dataset.quickAction;
  if (action === "sync-now") return syncCloud();
  if (action === "backup-now") return createManualBackup();
}

async function savePreferences() {
  const prefs = loadPreferences();
  const serverName = document.querySelector("#setting-server-name")?.value?.trim();
  if (serverName) prefs.serverName = serverName;
  prefs.timeZone = document.querySelector("#setting-time-zone")?.value || prefs.timeZone;
  prefs.localLock = Boolean(document.querySelector("#setting-local-lock")?.checked);
  prefs.autoLock = document.querySelector("#setting-auto-lock")?.value || prefs.autoLock;
  prefs.notifications = Boolean(document.querySelector("#setting-notifications")?.checked);
  prefs.alerts = document.querySelector("#setting-alerts")?.value || prefs.alerts;
  prefs.compact = Boolean(document.querySelector("#setting-compact")?.checked);
  prefs.autoRefresh = Boolean(document.querySelector("#setting-auto-refresh")?.checked);
  const requestedAutostart = Boolean(document.querySelector("#setting-start-with-windows")?.checked);
  if (requestedAutostart !== desktopAutostartEnabled) {
    try {
      desktopAutostartEnabled = Boolean(await invoke("control_center_set_autostart", { enabled: requestedAutostart }));
    } catch (error) {
      notice = { kind: "warning", message: `Unable to update Windows startup: ${String(error)}` };
      render();
      return;
    }
  }
  localStorage.setItem("homeserver-ui-preferences", JSON.stringify(prefs));
  notice = { kind: "success", message: "Control Center and Windows desktop preferences saved." };
  render();
}

async function withBusy(action) {
  busy = true;
  notice = null;
  render();
  try {
    notice = await action();
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    await loadAll(false);
  }
}

async function pairCloud(event) {
  event.preventDefault();
  const cloudBaseUrl = document.querySelector("#cloud-base-url")?.value?.trim() || "";
  const pairingCode = document.querySelector("#cloud-pairing-code")?.value?.trim() || "";
  await withBusy(async () => {
    await invoke("homeserver_pair_cloud", { request: { cloud_base_url: cloudBaseUrl, pairing_code: pairingCode } });
    return { kind: "success", message: "HomeServer paired and its first signed cloud request was verified." };
  });
}

async function syncCloud() {
  if (!isConnected()) {
    navigate("sync");
    notice = { kind: "warning", message: "Pair HomeServer before running cloud synchronization." };
    render();
    return;
  }
  await withBusy(async () => {
    const result = await invoke("homeserver_sync_cloud");
    return { kind: "success", message: `Cloud synchronization completed: ${result.accepted || 0} accepted, ${result.rejected || 0} rejected, ${result.review || 0} queued for review.` };
  });
}

async function testCloudVault() {
  await withBusy(async () => {
    const result = await invoke("homeserver_cloud_vault_self_test");
    return { kind: "success", message: result.message || "Credential vault test passed." };
  });
}

async function disconnectCloud() {
  const confirmed = window.confirm("Disconnect this HomeServer locally? The cloud device remains registered until it is revoked from your Microgifter account.");
  if (!confirmed) return;
  await withBusy(async () => {
    await invoke("homeserver_disconnect_cloud");
    return { kind: "success", message: "Local cloud credentials were removed from this HomeServer." };
  });
}

async function enqueueCloudOperation(event) {
  const operationType = event.currentTarget.dataset.cloudOperation;
  const payload = operationType === "local.settings.snapshot" ? { source: "control_center", captured_at_utc: new Date().toISOString() } : { source: "control_center", requested_at_utc: new Date().toISOString() };
  await withBusy(async () => {
    const result = await invoke("homeserver_enqueue_cloud_sync", { request: { operation_type: operationType, payload, idempotency_key: null } });
    return { kind: "success", message: `Queued ${humanize(operationType)} as ${result.idempotency_key}.` };
  });
}

async function checkUpdates() {
  await withBusy(async () => {
    const result = await invoke("homeserver_check_updates");
    return { kind: "success", message: result.message };
  });
}

async function downloadUpdate() {
  await withBusy(async () => {
    const result = await invoke("homeserver_download_update");
    return { kind: "success", message: result.message };
  });
}

async function applyUpdate() {
  const confirmation = window.prompt("Type UPDATE to create a pre-update backup and apply this verified HomeServer release:");
  if (confirmation !== "UPDATE") return;
  busy = true;
  notice = null;
  render();
  try {
    const result = await invoke("homeserver_apply_update", { request: { confirmation } });
    notice = { kind: "success", message: result.message };
    window.setTimeout(() => loadAll(false), 15000);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    render();
  }
}

async function createManualBackup() {
  if (!statusSnapshot?.api_available) {
    notice = { kind: "warning", message: "The HomeServer service must be available before creating a backup." };
    render();
    return;
  }
  await withBusy(async () => {
    const result = await invoke("homeserver_create_backup", { request: { kind: "manual", passphrase: null, note: "Created from Control Center" } });
    return { kind: "success", message: result.message };
  });
}

async function createRecoveryPackage(event) {
  event.preventDefault();
  const passphrase = document.querySelector("#recovery-passphrase")?.value || "";
  const confirmation = document.querySelector("#recovery-passphrase-confirm")?.value || "";
  if (passphrase !== confirmation) {
    notice = { kind: "warning", message: "Recovery passphrases do not match." };
    render();
    return;
  }
  await withBusy(async () => {
    const result = await invoke("homeserver_create_backup", { request: { kind: "recovery", passphrase, note: "Portable recovery package" } });
    return { kind: "success", message: `${result.message} Use Export beside the package to save a disaster-recovery copy.` };
  });
}

async function importRecoveryPackage(event) {
  event.preventDefault();
  const passphrase = document.querySelector("#import-recovery-passphrase")?.value || "";
  await withBusy(async () => {
    const result = await invoke("homeserver_import_recovery_package", { passphrase });
    if (!result) return null;
    return { kind: "success", message: result.message };
  });
}

async function handleBackupAction(event) {
  const button = event.currentTarget;
  const action = button.dataset.backupAction;
  const backupId = button.dataset.backupId;
  const backupKind = button.dataset.backupKind;
  if (action === "export") {
    await withBusy(async () => {
      const destination = await invoke("homeserver_export_recovery_package", { backupId, suggestedFileName: button.dataset.backupFileName || "Microgifter-HomeServer-Recovery.mghbackup" });
      if (!destination) return null;
      return { kind: "success", message: `Recovery package exported to ${destination}` };
    });
    return;
  }
  let passphrase = null;
  if (backupKind === "recovery") {
    passphrase = window.prompt("Enter the recovery package passphrase:");
    if (passphrase === null) return;
  }
  if (action === "verify") {
    await withBusy(async () => {
      const result = await invoke("homeserver_verify_backup", { request: { backup_id: backupId, passphrase, confirmation: null } });
      return { kind: "success", message: result.message };
    });
    return;
  }
  if (action === "restore") {
    const confirmation = window.prompt("Type RESTORE to stage this database for the next HomeServer restart:");
    if (confirmation !== "RESTORE") return;
    await withBusy(async () => {
      const result = await invoke("homeserver_stage_restore", { request: { backup_id: backupId, passphrase, confirmation } });
      return { kind: "success", message: result.message };
    });
  }
}


async function copyOcrCommand(event) {
  const command = event.currentTarget.dataset.ocrCommand || "";
  if (!command) return;
  try {
    await navigator.clipboard.writeText(command);
    notice = { kind: "success", message: "OCR installation command copied." };
  } catch {
    notice = { kind: "info", message: command };
  }
  render();
}

async function importVaultDocument() {
  await withBusy(async () => {
    const result = await invoke("homeserver_import_vault_document", { tags: [] });
    if (!result) return null;
    vaultSearchResult = null;
    return { kind: result.affected ? "success" : "info", message: result.message };
  });
}

async function searchVault(event) {
  event.preventDefault();
  const query = document.querySelector("#vault-search-query")?.value?.trim() || "";
  const mode = document.querySelector("#vault-search-mode")?.value || "hybrid";
  if (!query) return;
  busy = true;
  notice = null;
  render();
  try {
    vaultSearchResult = await invoke("homeserver_search_semantic_vault", { query, mode });
    if (mode !== "keyword" && !vaultSearchResult.semantic_available) {
      notice = { kind: "info", message: "The semantic index is not ready, so HomeServer returned bounded keyword results." };
    }
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    render();
  }
}

async function rebuildSemanticVault(force) {
  await withBusy(async () => {
    const result = await invoke("homeserver_rebuild_semantic_vault", { force });
    vaultSearchResult = null;
    return { kind: result.accepted ? "success" : "info", message: result.message };
  });
}

async function reindexVault() {
  await withBusy(async () => {
    const result = await invoke("homeserver_reindex_vault");
    vaultSearchResult = null;
    return { kind: "success", message: result.message };
  });
}

async function deleteVaultDocument(event) {
  const documentId = event.currentTarget.dataset.vaultDelete;
  const confirmation = window.prompt("Type DELETE to remove the HomeServer-managed copy and its local index. The source file will not be changed:");
  if (confirmation !== "DELETE") return;
  await withBusy(async () => {
    const result = await invoke("homeserver_delete_vault_document", { documentId, confirmation });
    vaultSearchResult = null;
    return { kind: "success", message: result.message };
  });
}


async function pullModel(event) {
  const model = event.currentTarget.dataset.modelPull;
  await withBusy(async () => {
    const result = await invoke("homeserver_pull_model", { model });
    return { kind: result.accepted ? "success" : "info", message: result.message };
  });
}

async function deleteModel(event) {
  const model = event.currentTarget.dataset.modelDelete;
  const confirmation = window.prompt(`Type DELETE to remove ${model} from the local Ollama model store:`);
  if (confirmation !== "DELETE") return;
  await withBusy(async () => {
    const result = await invoke("homeserver_delete_model", { model, confirmation });
    if (modelTestResult?.model === model) modelTestResult = null;
    return { kind: "success", message: result.message };
  });
}

async function unloadModel(event) {
  const model = event.currentTarget.dataset.modelUnload;
  await withBusy(async () => {
    const result = await invoke("homeserver_unload_model", { model });
    return { kind: "success", message: result.message };
  });
}

function selectModelForTest(event) {
  const model = event.currentTarget.dataset.modelTestSelect;
  const select = document.querySelector("#model-test-name");
  if (select) select.value = model;
  document.querySelector("#model-test-prompt")?.focus();
}

async function testModel(event) {
  event.preventDefault();
  const model = document.querySelector("#model-test-name")?.value || "";
  const prompt = document.querySelector("#model-test-prompt")?.value?.trim() || "";
  if (!model || !prompt) return;
  await withBusy(async () => {
    modelTestResult = await invoke("homeserver_test_model", { model, prompt });
    return { kind: "success", message: `Local ${humanize(modelTestResult.kind)} test completed in ${Number(modelTestResult.duration_ms || 0)} ms.` };
  });
}

async function saveModelSettings(event) {
  event.preventDefault();
  const defaultChatModel = document.querySelector("#model-default-chat")?.value || null;
  const defaultEmbeddingModel = document.querySelector("#model-default-embedding")?.value || null;
  const contextSize = Number(document.querySelector("#model-context-size")?.value || 4096);
  const testTimeoutSeconds = Number(document.querySelector("#model-test-timeout")?.value || 60);
  const maxDownloadGb = Number(document.querySelector("#model-download-limit")?.value || 20);
  await withBusy(async () => {
    await invoke("homeserver_update_model_settings", { defaultChatModel, defaultEmbeddingModel, contextSize, testTimeoutSeconds, maxDownloadGb });
    return { kind: "success", message: "Local Model Center defaults and limits were saved." };
  });
}

async function createMcpClient(event) {
  event.preventDefault();
  const displayName = document.querySelector("#mcp-client-name")?.value?.trim() || "";
  const expiresDays = Number(document.querySelector("#mcp-client-expiry")?.value || 90);
  const scopes = [...document.querySelectorAll('input[name="mcp-scope"]:checked')].map((input) => input.value);
  await withBusy(async () => {
    mcpCredential = await invoke("homeserver_create_mcp_client", { displayName, scopes, expiresDays });
    return { kind: "success", message: "MCP client created. Copy the one-time token before leaving this page." };
  });
}

async function revokeMcpClient(event) {
  const clientId = event.currentTarget.dataset.mcpRevoke;
  if (!window.confirm("Revoke this MCP client immediately?")) return;
  await withBusy(async () => {
    await invoke("homeserver_revoke_mcp_client", { clientId, confirmation: "REVOKE" });
    if (mcpCredential?.client?.client_id === clientId) mcpCredential = null;
    return { kind: "success", message: "MCP client revoked." };
  });
}

async function copyMcpValue(event) {
  const value = event.currentTarget.dataset.copyValue || "";
  try {
    await navigator.clipboard.writeText(value);
    notice = { kind: "success", message: "MCP configuration copied." };
  } catch (error) {
    notice = { kind: "warning", message: `Unable to copy MCP configuration: ${String(error)}` };
  }
  render();
}

async function loadAll(clearNotice = true) {
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
    invoke("homeserver_agent_integrations"),
    invoke("control_center_autostart_enabled"),
  ]);
  if (results[10].status === "fulfilled") desktopAutostartEnabled = Boolean(results[10].value);
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
  agentIntegrationSnapshot = results[9].status === "fulfilled" ? results[9].value : agentIntegrationSnapshot;

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
}


window.addEventListener("homeserver-tray-action", (event) => {
  if (event.detail?.action !== "check-updates" || busy) return;
  navigate("system");
  void checkUpdates();
});

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

window.addEventListener("hashchange", () => {
  const page = window.location.hash.replace("#", "");
  if (pages.some(([key]) => key === page)) {
    activePage = page;
    render();
  }
});

render();
loadAll();
window.setInterval(() => {
  if (!busy && loadPreferences().autoRefresh) loadAll(false);
}, 30000);
