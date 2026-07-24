import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

const app = document.querySelector("#app");
let statusSnapshot = null;
let backupCatalog = null;
let notice = null;
let busy = false;

const serviceRows = [
  ["Core service", "Coordinates HomeServer configuration and health."],
  ["Local API", "Private loopback API for the Control Center."],
  ["Local database", "Embedded operational storage for local state."],
  ["Microgifter connection", "Cloud pairing and synchronization foundation."],
  ["Backup service", "Encrypted backups, recovery packages, and staged restore."],
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

function formatBytes(value) {
  const bytes = Number(value || 0);
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const unit = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** unit).toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
}

function backupRows() {
  const backups = backupCatalog?.backups || [];
  if (!backups.length) {
    return `<div class="empty-state"><strong>No backups yet</strong><p>Create a protected local backup or an exportable recovery package.</p></div>`;
  }

  return backups
    .slice(0, 25)
    .map(
      (backup) => `
        <article class="backup-row">
          <div class="backup-kind">${escapeHtml(backup.kind === "recovery" ? "RP" : "BK")}</div>
          <div class="backup-copy">
            <div class="backup-title">
              <strong>${escapeHtml(humanize(backup.kind))}</strong>
              <span class="pill ${statusClass(backup.state)}">${escapeHtml(humanize(backup.state))}</span>
            </div>
            <p>${escapeHtml(backup.file_name)}</p>
            <small>${escapeHtml(formatDate(backup.created_at_utc))} · ${escapeHtml(formatBytes(backup.size_bytes))} · ${escapeHtml(humanize(backup.encryption))}</small>
            ${backup.note ? `<small class="backup-note">${escapeHtml(backup.note)}</small>` : ""}
            ${backup.failure_code ? `<small class="failure-text">${escapeHtml(humanize(backup.failure_code))}</small>` : ""}
          </div>
          <div class="backup-actions">
            <button type="button" data-backup-action="verify" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-kind="${escapeHtml(backup.kind)}" ${busy ? "disabled" : ""}>Verify</button>
            <button class="danger-button" type="button" data-backup-action="restore" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-kind="${escapeHtml(backup.kind)}" ${busy ? "disabled" : ""}>Restore</button>
          </div>
        </article>`,
    )
    .join("");
}

function render() {
  const snapshot = statusSnapshot;
  const state = snapshot?.state || "offline";
  const apiAvailable = Boolean(snapshot?.api_available);
  const databaseState = snapshot?.database || "unknown";
  const restorePending = Boolean(backupCatalog?.restore_pending || snapshot?.restore_pending);
  const retentionCount = backupCatalog?.retention_count ?? 14;
  const intervalHours = backupCatalog?.interval_hours ?? 24;

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
        ${restorePending ? `<div class="notice warning"><strong>Restore staged.</strong> Restart the HomeServer service or reboot Windows to apply the verified database. The current database will be preserved for rollback.</div>` : ""}

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
          <article><span>Cloud connection</span><strong>${escapeHtml(humanize(snapshot?.cloud || "not_paired"))}</strong><small>Pairing is developed separately</small></article>
          <article><span>Sync queue</span><strong>${snapshot?.pending_sync ?? 0}</strong><small>Waiting operations</small></article>
          <article><span>Last backup</span><strong class="metric-date">${escapeHtml(formatDate(snapshot?.last_backup))}</strong><small>${intervalHours}-hour automatic schedule</small></article>
          <article><span>Backup state</span><strong>${escapeHtml(humanize(snapshot?.backup || "ready"))}</strong><small>${retentionCount} automatic backups retained</small></article>
        </section>

        <section class="panel backup-panel" id="backups">
          <div class="panel-heading">
            <div><p class="eyebrow">Encrypted local protection</p><h2>Backups & Recovery</h2></div>
            <span>${backupCatalog?.backups?.length ?? 0} catalog records</span>
          </div>

          <div class="backup-tools">
            <article class="backup-tool-card">
              <h3>Protected local backup</h3>
              <p>Creates an encrypted SQLite snapshot using a device key stored in the Windows credential vault.</p>
              <button id="create-manual-backup" class="primary-button" type="button" ${busy || !apiAvailable ? "disabled" : ""}>Create backup</button>
            </article>
            <article class="backup-tool-card">
              <h3>Portable recovery package</h3>
              <p>Creates an exportable package protected by a passphrase that is never stored by HomeServer.</p>
              <form id="recovery-package-form">
                <input id="recovery-passphrase" type="password" minlength="12" maxlength="256" autocomplete="new-password" placeholder="Recovery passphrase" required>
                <input id="recovery-passphrase-confirm" type="password" minlength="12" maxlength="256" autocomplete="new-password" placeholder="Confirm passphrase" required>
                <button class="primary-button" type="submit" ${busy || !apiAvailable ? "disabled" : ""}>Create recovery package</button>
              </form>
            </article>
          </div>

          <div class="backup-policy">
            <span>Automatic schedule: every ${intervalHours} hours</span>
            <span>Retention: ${retentionCount} automatic backups</span>
            <span>Last automatic: ${escapeHtml(formatDate(backupCatalog?.last_automatic_backup_utc))}</span>
          </div>

          <div class="backup-list">${backupRows()}</div>
        </section>

        <section class="panel" id="services">
          <div class="panel-heading">
            <div><p class="eyebrow">Local runtime</p><h2>Services</h2></div>
            <span>${serviceRows.length} components</span>
          </div>
          <div class="service-list">
            ${serviceRows
              .map(([name, description], index) => {
                const running = index < 3 ? apiAvailable : index === 4 && apiAvailable;
                return `<article class="service-row">
                  <div class="service-icon">${index + 1}</div>
                  <div><strong>${escapeHtml(name)}</strong><p>${escapeHtml(description)}</p></div>
                  <span class="service-state ${running ? "running" : "planned"}">${running ? "Running" : "Planned"}</span>
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
  document.querySelector("#refresh-status")?.addEventListener("click", () => loadAll());
  document.querySelector("#create-manual-backup")?.addEventListener("click", createManualBackup);
  document.querySelector("#recovery-package-form")?.addEventListener("submit", createRecoveryPackage);
  document.querySelectorAll("[data-backup-action]").forEach((button) => {
    button.addEventListener("click", handleBackupAction);
  });
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

async function createManualBackup() {
  await withBusy(async () => {
    const result = await invoke("homeserver_create_backup", {
      request: { kind: "manual", passphrase: null, note: "Created from Control Center" },
    });
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
    const result = await invoke("homeserver_create_backup", {
      request: { kind: "recovery", passphrase, note: "Portable recovery package" },
    });
    return { kind: "success", message: `${result.message} File: ${result.backup.file_name}` };
  });
}

async function handleBackupAction(event) {
  const button = event.currentTarget;
  const action = button.dataset.backupAction;
  const backupId = button.dataset.backupId;
  const backupKind = button.dataset.backupKind;
  let passphrase = null;
  if (backupKind === "recovery") {
    passphrase = window.prompt("Enter the recovery package passphrase:");
    if (passphrase === null) return;
  }

  if (action === "verify") {
    await withBusy(async () => {
      const result = await invoke("homeserver_verify_backup", {
        request: { backup_id: backupId, passphrase, confirmation: null },
      });
      return { kind: "success", message: result.message };
    });
    return;
  }

  if (action === "restore") {
    const confirmation = window.prompt('Type RESTORE to stage this database for the next HomeServer restart:');
    if (confirmation !== "RESTORE") return;
    await withBusy(async () => {
      const result = await invoke("homeserver_stage_restore", {
        request: { backup_id: backupId, passphrase, confirmation },
      });
      return { kind: "success", message: result.message };
    });
  }
}

async function loadAll(clearNotice = true) {
  if (clearNotice) notice = null;
  try {
    [statusSnapshot, backupCatalog] = await Promise.all([
      invoke("homeserver_status"),
      invoke("homeserver_backups"),
    ]);
  } catch (error) {
    statusSnapshot = null;
    backupCatalog = null;
    notice = { kind: "warning", message: `HomeServer service unavailable: ${String(error)}` };
  }
  render();
}

render();
loadAll();
