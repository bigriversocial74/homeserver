import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

const app = document.querySelector("#app");

const serviceRows = [
  ["Core service", "Coordinates HomeServer configuration and health."],
  ["Local API", "Private loopback API for the Control Center."],
  ["Local database", "Embedded operational storage for local state."],
  ["Microgifter connection", "Cloud pairing and synchronization foundation."],
  ["Backup service", "Backup and recovery foundation."],
  ["Model Center", "Optional local AI runtime management."],
];

function statusClass(state) {
  return String(state || "offline").toLowerCase().replaceAll("_", "-");
}

function render(snapshot, errorMessage = "") {
  const state = snapshot?.state || "offline";
  const apiAvailable = Boolean(snapshot?.api_available);
  const databaseState = snapshot?.database || "unknown";

  app.innerHTML = `
    <div class="shell">
      <aside class="sidebar">
        <div class="brand">
          <div class="brand-mark" aria-hidden="true">M</div>
          <div>
            <strong>Microgifter</strong>
            <span>HomeServer</span>
          </div>
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
          <span>${state}</span>
        </div>
      </aside>

      <section class="content" id="overview">
        <header class="topbar">
          <div>
            <p class="eyebrow">Private local Microgifter edge</p>
            <h1>HomeServer Overview</h1>
          </div>
          <button id="refresh-status" type="button">Refresh status</button>
        </header>

        ${errorMessage ? `<div class="notice warning">${errorMessage}</div>` : ""}

        <section class="hero-card">
          <div>
            <span class="pill ${statusClass(state)}">${state}</span>
            <h2>${snapshot?.server_name || "Microgifter HomeServer"}</h2>
            <p>${apiAvailable ? "The private local API is responding." : "The background service is not responding yet."}</p>
          </div>
          <div class="hero-meta">
            <div><span>Version</span><strong>${snapshot?.version || "0.1.0"}</strong></div>
            <div><span>API</span><strong>${apiAvailable ? "Available" : "Offline"}</strong></div>
            <div><span>Database</span><strong>${databaseState}</strong></div>
          </div>
        </section>

        <section class="metric-grid" aria-label="HomeServer metrics">
          <article><span>Cloud connection</span><strong>${snapshot?.cloud || "not paired"}</strong><small>Pairing begins in Phase 2</small></article>
          <article><span>Sync queue</span><strong>${snapshot?.pending_sync ?? 0}</strong><small>Waiting operations</small></article>
          <article><span>Last backup</span><strong>${snapshot?.last_backup || "Not created"}</strong><small>Backup engine follows</small></article>
          <article><span>Local AI</span><strong>${snapshot?.model || "Not installed"}</strong><small>Optional model runtime</small></article>
        </section>

        <section class="panel" id="services">
          <div class="panel-heading">
            <div>
              <p class="eyebrow">Phase 1 foundation</p>
              <h2>Services</h2>
            </div>
            <span>${serviceRows.length} components</span>
          </div>
          <div class="service-list">
            ${serviceRows
              .map(
                ([name, description], index) => `
                  <article class="service-row">
                    <div class="service-icon">${index + 1}</div>
                    <div><strong>${name}</strong><p>${description}</p></div>
                    <span class="service-state ${index < 3 && apiAvailable ? "running" : "planned"}">${index < 3 && apiAvailable ? "Running" : "Planned"}</span>
                  </article>`,
              )
              .join("")}
          </div>
        </section>

        <footer>
          <span>Local API: ${snapshot?.api_url || "http://127.0.0.1:47831"}</span>
          <span>Updated: ${snapshot?.last_updated_utc || "Not available"}</span>
        </footer>
      </section>
    </div>
  `;

  document.querySelector("#refresh-status")?.addEventListener("click", loadStatus);
}

async function loadStatus() {
  try {
    render(await invoke("homeserver_status"));
  } catch (error) {
    render(null, `HomeServer service unavailable: ${String(error)}`);
  }
}

render(null);
loadStatus();
