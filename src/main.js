import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

const app = document.querySelector("#app");
let statusSnapshot = null;
let connectionSnapshot = null;
let notice = null;
let busy = false;

const serviceRows = [
  ["Core service", "Coordinates HomeServer configuration and health."],
  ["Local API", "Private loopback API for the Control Center."],
  ["Local database", "Embedded operational storage for local state."],
  ["Microgifter connection", "Signed cloud pairing and retry-safe synchronization."],
  ["Backup service", "Backup and recovery foundation."],
  ["Model Center", "Optional local AI runtime management."],
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

function connectionPanel(connection) {
  const state = connection?.state || "not_paired";
  if (state === "not_paired") {
    return `
      <section class="panel connection-panel" id="connection">
        <div class="panel-heading">
          <div>
            <p class="eyebrow">Owner-approved setup</p>
            <h2>Connect to Microgifter</h2>
          </div>
          <span>Not paired</span>
        </div>
        <p class="panel-copy">Create a one-time HomeServer pairing code in your Microgifter account, then enter it here. The device token and signing key stay in the Windows credential vault.</p>
        <form id="pair-form" class="pair-form">
          <label>
            <span>Microgifter cloud URL</span>
            <input id="cloud-base-url" name="cloud_base_url" type="url" value="https://microgifter.com" required autocomplete="url">
          </label>
          <label>
            <span>One-time pairing code</span>
            <input id="pairing-code" name="pairing_code" type="text" minlength="20" maxlength="80" required autocomplete="off" spellcheck="false" placeholder="Paste the code from Microgifter">
          </label>
          <button class="primary-button" type="submit" ${busy ? "disabled" : ""}>${busy ? "Connecting…" : "Connect HomeServer"}</button>
        </form>
        <div class="security-note"><strong>Security boundary:</strong> HomeServer cannot approve payments, claims, redemptions, ownership changes, or other cloud-authoritative commerce actions.</div>
      </section>`;
  }

  const scopes = Array.isArray(connection?.scopes) ? connection.scopes : [];
  return `
    <section class="panel connection-panel" id="connection">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">Signed device connection</p>
          <h2>Microgifter Connection</h2>
        </div>
        <span class="pill ${statusClass(state)}">${escapeHtml(humanize(state))}</span>
      </div>
      <div class="connection-grid">
        <div><span>Cloud</span><strong>${escapeHtml(connection?.cloud_base_url || "Unknown")}</strong></div>
        <div><span>Device ID</span><strong class="mono">${escapeHtml(connection?.device_id || "Unknown")}</strong></div>
        <div><span>Paired</span><strong>${escapeHtml(formatDate(connection?.paired_at_utc))}</strong></div>
        <div><span>Last verified</span><strong>${escapeHtml(formatDate(connection?.last_success_utc))}</strong></div>
      </div>
      ${connection?.last_error ? `<div class="notice warning">Connection warning: ${escapeHtml(humanize(connection.last_error))}</div>` : ""}
      <div class="scope-list" aria-label="Device scopes">
        ${scopes.map((scope) => `<span>${escapeHtml(scope)}</span>`).join("") || "<span>No active scopes</span>"}
      </div>
      <div class="connection-actions">
        <button id="sync-now" class="primary-button" type="button" ${busy ? "disabled" : ""}>Sync now</button>
        <button id="queue-settings" type="button" ${busy ? "disabled" : ""}>Queue local settings receipt</button>
        <button id="disconnect-cloud" class="danger-button" type="button" ${busy ? "disabled" : ""}>Disconnect locally</button>
      </div>
      <p class="muted-note">Disconnecting removes the local credential. Revoke the registered device in Microgifter to invalidate it centrally.</p>
    </section>`;
}

function render() {
  const snapshot = statusSnapshot;
  const connection = connectionSnapshot;
  const state = snapshot?.state || "offline";
  const apiAvailable = Boolean(snapshot?.api_available);
  const databaseState = snapshot?.database || "unknown";
  const cloudState = connection?.state || snapshot?.cloud || "not_paired";

  app.innerHTML = `
    <div class="shell">
      <aside class="sidebar">
        <div class="brand">
          <div class="brand-mark" aria-hidden="true">M</div>
          <div><strong>Microgifter</strong><span>HomeServer</span></div>
        </div>
        <nav aria-label="HomeServer sections">
          <a class="active" href="#overview">Overview</a>
          <a href="#services">Services</a>
          <a href="#connection">Microgifter Connection</a>
          <a href="#agents">Agents</a>
          <a href="#models">Model Center</a>
          <a href="#knowledge">Knowledge Vault</a>
          <a href="#backups">Backups</a>
          <a href="#updates">Updates</a>
          <a href="#support">Logs & Support</a>
        </nav>
        <div class="sidebar-footer">
          <span class="status-dot ${statusClass(state)}"></span>
          <span>${escapeHtml(humanize(state))}</span>
        </div>
      </aside>

      <section class="content" id="overview">
        <header class="topbar">
          <div><p class="eyebrow">Private local Microgifter edge</p><h1>HomeServer Overview</h1></div>
          <button id="refresh-status" type="button" ${busy ? "disabled" : ""}>Refresh status</button>
        </header>

        ${notice ? `<div class="notice ${notice.kind}">${escapeHtml(notice.message)}</div>` : ""}

        <section class="hero-card">
          <div>
            <span class="pill ${statusClass(state)}">${escapeHtml(humanize(state))}</span>
            <h2>${escapeHtml(snapshot?.server_name || "Microgifter HomeServer")}</h2>
            <p>${apiAvailable ? "The private local API is responding." : "The background service is not responding yet."}</p>
          </div>
          <div class="hero-meta">
            <div><span>Version</span><strong>${escapeHtml(snapshot?.version || "0.1.0")}</strong></div>
            <div><span>API</span><strong>${apiAvailable ? "Available" : "Offline"}</strong></div>
            <div><span>Database</span><strong>${escapeHtml(humanize(databaseState))}</strong></div>
          </div>
        </section>

        <section class="metric-grid" aria-label="HomeServer metrics">
          <article><span>Cloud connection</span><strong>${escapeHtml(humanize(cloudState))}</strong><small>Signed and revocable</small></article>
          <article><span>Sync queue</span><strong>${snapshot?.pending_sync ?? 0}</strong><small>Not completed until accepted</small></article>
          <article><span>Last backup</span><strong>${escapeHtml(snapshot?.last_backup || "Not created")}</strong><small>Phase 3 foundation</small></article>
          <article><span>Local AI</span><strong>${escapeHtml(snapshot?.model || "Not installed")}</strong><small>Optional model runtime</small></article>
        </section>

        ${connectionPanel(connection)}

        <section class="panel" id="services">
          <div class="panel-heading">
            <div><p class="eyebrow">Local runtime</p><h2>Services</h2></div>
            <span>${serviceRows.length} components</span>
          </div>
          <div class="service-list">
            ${serviceRows
              .map(([name, description], index) => {
                const running = index < 3 ? apiAvailable : index === 3 && cloudState === "connected";
                return `<article class="service-row">
                  <div class="service-icon">${index + 1}</div>
                  <div><strong>${escapeHtml(name)}</strong><p>${escapeHtml(description)}</p></div>
                  <span class="service-state ${running ? "running" : "planned"}">${running ? "Running" : index === 3 ? escapeHtml(humanize(cloudState)) : "Planned"}</span>
                </article>`;
              })
              .join("")}
          </div>
        </section>

        <footer>
          <span>Local API: ${escapeHtml(snapshot?.api_url || "http://127.0.0.1:47831")}</span>
          <span>Updated: ${escapeHtml(formatDate(snapshot?.last_updated_utc))}</span>
        </footer>
      </section>
    </div>`;

  bindEvents();
}

function bindEvents() {
  document.querySelector("#refresh-status")?.addEventListener("click", loadAll);
  document.querySelector("#pair-form")?.addEventListener("submit", pairCloud);
  document.querySelector("#sync-now")?.addEventListener("click", syncNow);
  document.querySelector("#queue-settings")?.addEventListener("click", queueSettings);
  document.querySelector("#disconnect-cloud")?.addEventListener("click", disconnectCloud);
}

async function withBusy(action, successMessage) {
  busy = true;
  notice = null;
  render();
  try {
    await action();
    notice = successMessage ? { kind: "success", message: successMessage } : null;
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    await loadAll(false);
  }
}

async function pairCloud(event) {
  event.preventDefault();
  const cloudBaseUrl = document.querySelector("#cloud-base-url")?.value.trim();
  const pairingCode = document.querySelector("#pairing-code")?.value.trim();
  await withBusy(async () => {
    connectionSnapshot = await invoke("homeserver_pair", {
      request: { cloud_base_url: cloudBaseUrl, pairing_code: pairingCode },
    });
  }, "HomeServer paired and its signed cloud connection was verified.");
}

async function syncNow() {
  await withBusy(async () => {
    const result = await invoke("homeserver_sync_now");
    notice = {
      kind: "success",
      message: `Sync processed ${result.processed} operation(s): ${result.accepted} accepted, ${result.rejected} rejected, ${result.review} awaiting review.`,
    };
  });
}

async function queueSettings() {
  await withBusy(async () => {
    await invoke("homeserver_enqueue_sync", {
      request: {
        operation_type: "local.settings.snapshot",
        payload: {
          server_name: statusSnapshot?.server_name || "Microgifter HomeServer",
          version: statusSnapshot?.version || "0.1.0",
          source: "control_center",
        },
        idempotency_key: null,
      },
    });
    await invoke("homeserver_sync_now");
  }, "The local settings receipt was queued and submitted to Microgifter.");
}

async function disconnectCloud() {
  if (!window.confirm("Disconnect this HomeServer locally? The device should also be revoked from your Microgifter account.")) return;
  await withBusy(async () => {
    connectionSnapshot = await invoke("homeserver_disconnect");
  }, "Local cloud credentials were removed.");
}

async function loadAll(clearNotice = true) {
  if (clearNotice) notice = null;
  try {
    [statusSnapshot, connectionSnapshot] = await Promise.all([
      invoke("homeserver_status"),
      invoke("homeserver_connection"),
    ]);
  } catch (error) {
    statusSnapshot = null;
    connectionSnapshot = null;
    notice = { kind: "warning", message: `HomeServer service unavailable: ${String(error)}` };
  }
  render();
}

render();
loadAll();
