import { invoke } from "@tauri-apps/api/core";
import "./cloud-connections.css";

let snapshot = null;
let loading = false;
let actionBusy = false;
let notice = null;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function humanize(value) {
  return String(value || "unknown").replaceAll("_", " ");
}

function formatDate(value) {
  if (!value) return "Not yet";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function badge(state) {
  const normalized = String(state || "unknown").replaceAll("_", "-");
  return `<span class="cloud-connection-badge state-${escapeHtml(normalized)}">${escapeHtml(humanize(state))}</span>`;
}

function connectionCard(connection) {
  const identity = [connection.tenant_id, connection.site_id].filter(Boolean).join(" · ") || "Provider-managed site";
  return `<article class="cloud-connection-card" data-connection-id="${escapeHtml(connection.connection_id)}">
    <div class="cloud-connection-card-head">
      <div><span class="cloud-provider-label">${escapeHtml(humanize(connection.provider_key))}</span><h3>${escapeHtml(connection.display_name)}</h3><p>${escapeHtml(identity)}</p></div>
      <div class="cloud-connection-status">${connection.is_default ? '<span class="cloud-default-badge">Default</span>' : ""}${badge(connection.state)}</div>
    </div>
    <dl class="cloud-connection-details">
      <div><dt>Endpoint</dt><dd>${escapeHtml(connection.cloud_base_url)}</dd></div>
      <div><dt>Device</dt><dd class="mono">${escapeHtml(connection.device_id)}</dd></div>
      <div><dt>Pending</dt><dd>${Number(connection.pending_sync || 0)}</dd></div>
      <div><dt>Last sync</dt><dd>${escapeHtml(formatDate(connection.last_success_utc))}</dd></div>
    </dl>
    ${connection.last_error ? `<div class="cloud-connection-error">${escapeHtml(humanize(connection.last_error))}</div>` : ""}
    <div class="cloud-connection-scopes">${(connection.scopes || []).map((scope) => `<span>${escapeHtml(scope)}</span>`).join("")}</div>
    <div class="cloud-connection-actions">
      <button class="button secondary" type="button" data-cloud-action="sync" data-connection-id="${escapeHtml(connection.connection_id)}" ${actionBusy || ["revoked", "disconnected"].includes(connection.state) ? "disabled" : ""}>Sync Connection</button>
      <button class="button ghost danger" type="button" data-cloud-action="disconnect" data-connection-id="${escapeHtml(connection.connection_id)}" ${actionBusy || connection.state === "disconnected" ? "disabled" : ""}>Disconnect</button>
    </div>
  </article>`;
}

function renderPanel() {
  const connections = Array.isArray(snapshot?.connections) ? snapshot.connections : [];
  const noticeMarkup = notice ? `<div class="cloud-connection-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : "";
  return `<section class="panel cloud-connections-panel" id="cloud-connections-registry">
    <div class="panel-title cloud-connections-title">
      <div><h2>Cloud Connection Registry</h2><p>Pair this HomeServer with multiple Microgifter sites now and additional CRM providers through future adapters.</p></div>
      <div class="cloud-connection-title-actions"><span class="planned-label">${Number(snapshot?.active_connections || 0)} active · ${Number(snapshot?.pending_sync || 0)} pending</span><button class="button secondary" id="cloud-connections-refresh" type="button" ${loading || actionBusy ? "disabled" : ""}>Refresh</button><button class="button primary" id="cloud-connections-sync-all" type="button" ${loading || actionBusy || !snapshot?.active_connections ? "disabled" : ""}>Sync All</button></div>
    </div>
    ${noticeMarkup}
    <div class="cloud-connections-boundary"><strong>Connection-scoped authority</strong><span>Each provider/site has a separate device identity, Windows credential-vault entry, scope set, queue, idempotency namespace, receipts, and revocation state. HomeServer remains usable with zero cloud connections.</span></div>
    <div class="cloud-connections-layout">
      <form id="cloud-connection-pair-form" class="cloud-connection-form">
        <div><h3>Pair a Site</h3><p>The pairing token is exchanged once and is never retained.</p></div>
        <label><span>Provider</span><select id="cloud-connection-provider" required><option value="microgifter">Microgifter Cloud</option></select></label>
        <label><span>Connection name</span><input id="cloud-connection-name" minlength="1" maxlength="120" placeholder="Restaurant A" required></label>
        <label><span>Cloud URL</span><input id="cloud-connection-url" type="url" maxlength="300" value="https://microgifter.com" required></label>
        <label><span>Pairing token</span><input id="cloud-connection-token" type="password" minlength="20" maxlength="80" autocomplete="one-time-code" placeholder="Paste the one-time pairing token" required></label>
        <div class="cloud-connection-form-row"><label><span>Tenant ID <small>optional</small></span><input id="cloud-connection-tenant" maxlength="120" placeholder="Company or account"></label><label><span>Site ID <small>optional</small></span><input id="cloud-connection-site" maxlength="120" placeholder="Location or workspace"></label></div>
        <label class="cloud-default-check"><input id="cloud-connection-default" type="checkbox"><span>Make this the default connection</span></label>
        <button class="button primary" type="submit" ${actionBusy ? "disabled" : ""}>Pair Connection</button>
      </form>
      <div class="cloud-connections-list">
        ${loading && !snapshot ? '<div class="cloud-connections-empty"><strong>Loading connections…</strong></div>' : connections.length ? connections.map(connectionCard).join("") : '<div class="cloud-connections-empty"><strong>Local-only mode</strong><p>No cloud connection is required. Pair a site when this HomeServer should synchronize with Microgifter or another supported CRM.</p></div>'}
      </div>
    </div>
  </section>`;
}

function findMount() {
  const integrationGrid = document.querySelector(".integration-grid");
  if (!integrationGrid) return null;
  return integrationGrid.parentElement;
}

function mount() {
  const parent = findMount();
  if (!parent) return;
  let panel = document.querySelector("#cloud-connections-registry");
  if (!panel) {
    panel = document.createElement("div");
    panel.innerHTML = renderPanel();
    panel = panel.firstElementChild;
    const mcpPanel = parent.querySelector("#mcp-runtime");
    if (mcpPanel) parent.insertBefore(panel, mcpPanel);
    else parent.append(panel);
  } else {
    panel.outerHTML = renderPanel();
  }
  bindEvents();
  if (!snapshot && !loading) void refresh();
}

function bindEvents() {
  document.querySelector("#cloud-connection-pair-form")?.addEventListener("submit", pairConnection);
  document.querySelector("#cloud-connections-refresh")?.addEventListener("click", () => refresh(true));
  document.querySelector("#cloud-connections-sync-all")?.addEventListener("click", syncAll);
  document.querySelectorAll("[data-cloud-action]").forEach((button) => {
    button.addEventListener("click", handleConnectionAction);
  });
}

async function refresh(showNotice = false) {
  loading = true;
  mount();
  try {
    snapshot = await invoke("homeserver_cloud_connections");
    if (showNotice) notice = { kind: "info", message: "Cloud connection registry refreshed." };
  } catch (error) {
    notice = { kind: "warning", message: `Cloud connection registry unavailable: ${String(error)}` };
  } finally {
    loading = false;
    mount();
  }
}

async function pairConnection(event) {
  event.preventDefault();
  actionBusy = true;
  notice = null;
  mount();
  const request = {
    provider_key: document.querySelector("#cloud-connection-provider")?.value || "microgifter",
    display_name: document.querySelector("#cloud-connection-name")?.value?.trim() || "",
    cloud_base_url: document.querySelector("#cloud-connection-url")?.value?.trim() || "",
    pairing_code: document.querySelector("#cloud-connection-token")?.value?.trim() || "",
    tenant_id: document.querySelector("#cloud-connection-tenant")?.value?.trim() || null,
    site_id: document.querySelector("#cloud-connection-site")?.value?.trim() || null,
    make_default: Boolean(document.querySelector("#cloud-connection-default")?.checked),
  };
  try {
    const connection = await invoke("homeserver_pair_cloud_connection", { request });
    notice = { kind: "success", message: `${connection.display_name} was paired and verified.` };
    snapshot = await invoke("homeserver_cloud_connections");
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount();
  }
}

async function handleConnectionAction(event) {
  const button = event.currentTarget;
  const connectionId = button.dataset.connectionId || "";
  const action = button.dataset.cloudAction;
  if (!connectionId) return;
  if (action === "disconnect" && !window.confirm("Disconnect this cloud site and remove its stored credential? Other connections will remain active.")) return;
  actionBusy = true;
  notice = null;
  mount();
  try {
    if (action === "sync") {
      const result = await invoke("homeserver_sync_cloud_connection", { connectionId });
      notice = { kind: "success", message: `Connection sync completed: ${Number(result.accepted || 0)} accepted, ${Number(result.rejected || 0)} rejected, ${Number(result.review || 0)} for review.` };
    } else if (action === "disconnect") {
      await invoke("homeserver_disconnect_cloud_connection", { connectionId });
      notice = { kind: "success", message: "Cloud connection disconnected and its credential was removed." };
    }
    snapshot = await invoke("homeserver_cloud_connections");
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount();
  }
}

async function syncAll() {
  actionBusy = true;
  notice = null;
  mount();
  try {
    const result = await invoke("homeserver_sync_all_cloud_connections");
    notice = { kind: "success", message: `All connections synchronized: ${Number(result.processed || 0)} processed, ${Number(result.pending || 0)} still pending.` };
    snapshot = await invoke("homeserver_cloud_connections");
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount();
  }
}

const observer = new MutationObserver(() => mount());
observer.observe(document.querySelector("#app"), { childList: true, subtree: true });
window.addEventListener("hashchange", () => window.setTimeout(mount, 0));
window.addEventListener("DOMContentLoaded", () => window.setTimeout(mount, 0));
