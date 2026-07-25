import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

const app = document.querySelector("#app");
let statusSnapshot = null;
let cloudSnapshot = null;
let backupCatalog = null;
let updateStatus = null;
let notice = null;
let busy = false;

const serviceRows = [
  ["Core service", "Coordinates HomeServer configuration and health."],
  ["Local API", "Private loopback API for the Control Center."],
  ["Local database", "Embedded operational storage for local state."],
  ["Microgifter connection", "Signed cloud pairing, heartbeat, and synchronization."],
  ["Backup service", "Encrypted backups, recovery packages, and staged restore."],
  ["Update service", "Signed release discovery, verified staging, and automatic rollback."],
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

function compactId(value) {
  const text = String(value || "");
  if (text.length <= 20) return text || "Not assigned";
  return `${text.slice(0, 8)}…${text.slice(-8)}`;
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
          <div class="backup-kind">${escapeHtml(backup.kind === "recovery" ? "RP" : backup.kind === "pre_update" ? "UP" : "BK")}</div>
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
            ${
              backup.kind === "recovery"
                ? `<button type="button" data-backup-action="export" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-file-name="${escapeHtml(backup.file_name)}" ${busy ? "disabled" : ""}>Export</button>`
                : ""
            }
            <button type="button" data-backup-action="verify" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-kind="${escapeHtml(backup.kind)}" ${busy ? "disabled" : ""}>Verify</button>
            <button class="danger-button" type="button" data-backup-action="restore" data-backup-id="${escapeHtml(backup.backup_id)}" data-backup-kind="${escapeHtml(backup.kind)}" ${busy ? "disabled" : ""}>Restore</button>
          </div>
        </article>`,
    )
    .join("");
}

function cloudPanel(apiAvailable) {
  const state = cloudSnapshot?.state || "not_paired";
  const paired = Boolean(cloudSnapshot?.device_id) && state !== "not_paired";
  const scopes = Array.isArray(cloudSnapshot?.scopes) ? cloudSnapshot.scopes : [];
  const canOperate = apiAvailable && !busy;

  return `
    <section class="panel connection-panel" id="connection">
      <div class="panel-heading">
        <div><p class="eyebrow">Signed cloud bridge</p><h2>Microgifter Connection</h2></div>
        <span class="pill ${statusClass(state)}">${escapeHtml(humanize(state))}</span>
      </div>

      <div class="connection-summary">
        <div><span>Connection</span><strong>${escapeHtml(humanize(state))}</strong></div>
        <div><span>Device identity</span><strong class="mono">${escapeHtml(compactId(cloudSnapshot?.device_id))}</strong></div>
        <div><span>Last cloud success</span><strong>${escapeHtml(formatDate(cloudSnapshot?.last_success_utc))}</strong></div>
        <div><span>Pending sync</span><strong>${Number(cloudSnapshot?.pending_sync || 0)}</strong></div>
      </div>

      ${cloudSnapshot?.last_error ? `<div class="notice warning"><strong>Connection warning:</strong> ${escapeHtml(humanize(cloudSnapshot.last_error))}</div>` : ""}

      ${
        paired
          ? `
            <div class="connection-layout">
              <article class="connection-card">
                <p class="eyebrow">Authorized device</p>
                <h3>Paired HomeServer</h3>
                <p>This installation signs every cloud request with its local Ed25519 key. The bearer token and signing key remain in the Windows credential vault.</p>
                <dl class="connection-details">
                  <div><dt>Cloud</dt><dd>${escapeHtml(cloudSnapshot?.cloud_base_url || "Unknown")}</dd></div>
                  <div><dt>Paired</dt><dd>${escapeHtml(formatDate(cloudSnapshot?.paired_at_utc))}</dd></div>
                </dl>
                <div class="scope-list">
                  ${scopes.length ? scopes.map((scope) => `<span>${escapeHtml(scope)}</span>`).join("") : "<span>No scopes returned</span>"}
                </div>
              </article>

              <article class="connection-card operation-card">
                <p class="eyebrow">Controlled synchronization</p>
                <h3>Queue & Sync</h3>
                <p>Only the declared low-risk operations below can be queued locally. Commerce, payments, claims, redemption, and ownership remain cloud-authoritative.</p>
                <div class="operation-grid">
                  <button type="button" data-cloud-operation="local.settings.snapshot" ${canOperate ? "" : "disabled"}>Queue settings snapshot</button>
                  <button type="button" data-cloud-operation="cache.refresh.request" ${canOperate ? "" : "disabled"}>Queue cache refresh</button>
                </div>
                <div class="connection-actions">
                  <button id="cloud-vault-test" type="button" ${canOperate ? "" : "disabled"}>Test credential vault</button>
                  <button id="cloud-sync-now" class="primary-button" type="button" ${canOperate ? "" : "disabled"}>Sync now</button>
                  <button id="cloud-disconnect" class="danger-button" type="button" ${canOperate ? "" : "disabled"}>Disconnect locally</button>
                </div>
              </article>
            </div>`
          : `
            <div class="connection-layout pairing-layout">
              <article class="connection-card">
                <p class="eyebrow">Owner-approved setup</p>
                <h3>Pair this HomeServer</h3>
                <p>Create a one-time pairing code from your Microgifter account, then enter it here. Pairing generates a new local signing key and stores credentials in the operating-system vault.</p>
                <form id="cloud-pair-form" class="connection-form">
                  <label>
                    <span>Microgifter cloud URL</span>
                    <input id="cloud-base-url" type="url" value="https://microgifter.com" maxlength="240" autocomplete="url" required>
                  </label>
                  <label>
                    <span>One-time pairing code</span>
                    <input id="cloud-pairing-code" class="mono" type="password" minlength="20" maxlength="80" autocomplete="one-time-code" placeholder="Paste pairing code" required>
                  </label>
                  <button class="primary-button" type="submit" ${canOperate ? "" : "disabled"}>Pair HomeServer</button>
                </form>
              </article>
              <article class="connection-card boundary-card">
                <p class="eyebrow">Authority boundary</p>
                <h3>Local execution, cloud authority</h3>
                <ul>
                  <li>Local keys, models, files, integrations, automations, backups, and diagnostics stay local.</li>
                  <li>Identity, payments, purchases, campaigns, rewards, wallets, gifts, claims, and redemption stay cloud-authoritative.</li>
                  <li>Every accepted synchronization operation receives a durable, idempotent cloud receipt.</li>
                </ul>
              </article>
            </div>`
      }

      <div class="security-note"><strong>Connector boundary:</strong> the HomeServer cannot use this channel to originate or finalize commerce mutations. Signed requests, scoped permissions, nonce replay protection, and cloud receipts are enforced independently by both sides.</div>
    </section>`;
}

function updatePanel(apiAvailable) {
  const state = updateStatus?.state || "idle";
  const release = updateStatus?.update;
  const canCheck = apiAvailable && !busy && !["downloading", "applying"].includes(state);
  const canDownload = apiAvailable && !busy && state === "available" && release;
  const canApply = apiAvailable && !busy && state === "staged" && release;
  const signer = release?.authenticode_thumbprint
    ? `${release.authenticode_thumbprint.slice(0, 12)}…${release.authenticode_thumbprint.slice(-8)}`
    : "Not staged";

  return `
    <section class="panel update-panel" id="updates">
      <div class="panel-heading">
        <div><p class="eyebrow">Verified Windows delivery</p><h2>Signed Updates</h2></div>
        <span class="pill ${statusClass(state)}">${escapeHtml(humanize(state))}</span>
      </div>

      <div class="update-summary">
        <div><span>Installed version</span><strong>${escapeHtml(updateStatus?.current_version || statusSnapshot?.version || "0.1.0")}</strong></div>
        <div><span>Stable release</span><strong>${escapeHtml(release?.version || "No newer release")}</strong></div>
        <div><span>Last checked</span><strong>${escapeHtml(formatDate(updateStatus?.last_checked_at_utc))}</strong></div>
        <div><span>Authenticode signer</span><strong class="mono">${escapeHtml(signer)}</strong></div>
      </div>

      ${updateStatus?.last_error ? `<div class="notice warning">Update warning: ${escapeHtml(humanize(updateStatus.last_error))}</div>` : ""}
      ${state === "applying" || updateStatus?.apply_pending ? `<div class="notice warning"><strong>Update application is active.</strong> The Windows service will stop, install the verified release, and either pass health verification or restore the previous binaries automatically.</div>` : ""}
      ${state === "rolled_back" ? `<div class="notice warning"><strong>Automatic rollback completed.</strong> The attempted release did not pass health verification, so the previous HomeServer binaries were restored.</div>` : ""}
      ${state === "succeeded" ? `<div class="notice success"><strong>Update verified.</strong> The installed release restarted and passed the loopback health and version checks.</div>` : ""}

      <div class="update-release-card">
        <div>
          <p class="eyebrow">Release integrity</p>
          <h3>${release ? `Microgifter HomeServer ${escapeHtml(release.version)}` : "Stable release channel"}</h3>
          <p>${escapeHtml(release?.release_notes || "Check the pinned Microgifter release channel for a newer signed HomeServer installer.")}</p>
        </div>
        <dl>
          <div><dt>Channel</dt><dd>${escapeHtml(humanize(updateStatus?.channel || "stable"))}</dd></div>
          <div><dt>Installer</dt><dd>${escapeHtml(release?.installer_file_name || "Not downloaded")}</dd></div>
          <div><dt>Size</dt><dd>${escapeHtml(formatBytes(release?.installer_size_bytes))}</dd></div>
          <div><dt>Manifest</dt><dd>Ed25519 pinned key</dd></div>
          <div><dt>Installer</dt><dd>SHA-256 + Authenticode</dd></div>
          <div><dt>Rollback</dt><dd>Health-confirmed binary restore</dd></div>
        </dl>
      </div>

      <div class="update-actions">
        <button id="check-updates" type="button" ${canCheck ? "" : "disabled"}>Check for updates</button>
        <button id="download-update" class="primary-button" type="button" ${canDownload ? "" : "disabled"}>Download verified installer</button>
        <button id="apply-update" class="danger-button" type="button" ${canApply ? "" : "disabled"}>Apply update</button>
      </div>
      <div class="security-note"><strong>Update boundary:</strong> the Control Center cannot choose a URL, public key, installer path, hash, or signer. Those values must come from the pinned, signed release manifest. A pre-update encrypted backup is created before the helper is launched.</div>
    </section>`;
}

function render() {
  const snapshot = statusSnapshot;
  const state = snapshot?.state || "offline";
  const apiAvailable = Boolean(snapshot?.api_available);
  const databaseState = snapshot?.database || "unknown";
  const cloudState = cloudSnapshot?.state || "not_paired";
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
          <article><span>Cloud connection</span><strong>${escapeHtml(humanize(cloudState))}</strong><small>Signed Microgifter bridge</small></article>
          <article><span>Sync queue</span><strong>${Number(cloudSnapshot?.pending_sync || snapshot?.pending_sync || 0)}</strong><small>Waiting operations</small></article>
          <article><span>Last backup</span><strong class="metric-date">${escapeHtml(formatDate(snapshot?.last_backup))}</strong><small>${intervalHours}-hour automatic schedule</small></article>
          <article><span>Update state</span><strong>${escapeHtml(humanize(snapshot?.update || "idle"))}</strong><small>${escapeHtml(snapshot?.update_version || "Stable channel")}</small></article>
        </section>

        ${cloudPanel(apiAvailable)}
        ${updatePanel(apiAvailable)}

        <section class="panel backup-panel" id="backups">
          <div class="panel-heading">
            <div><p class="eyebrow">Encrypted local protection</p><h2>Backups & Recovery</h2></div>
            <span>${backupCatalog?.backups?.length ?? 0} catalog records</span>
          </div>

          <div class="backup-tools">
            <article class="backup-tool-card">
              <h3>Protected local backup</h3>
              <p>Creates an encrypted SQLite snapshot using a Windows DPAPI-protected device key stored with HomeServer data.</p>
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
            <article class="backup-tool-card import-card">
              <div>
                <h3>Recover another installation</h3>
                <p>Choose an exported <code>.mghbackup</code> package. HomeServer decrypts, validates, and registers it before restore can be staged.</p>
              </div>
              <form id="import-recovery-form" class="import-recovery-form">
                <input id="import-recovery-passphrase" type="password" minlength="12" maxlength="256" autocomplete="current-password" placeholder="Package passphrase" required>
                <button class="primary-button" type="submit" ${busy || !apiAvailable ? "disabled" : ""}>Choose package to import</button>
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
                const cloudRunning = ["connected", "degraded"].includes(cloudState);
                const running = index < 3 || (index === 3 && cloudRunning) || (index === 4 && apiAvailable) || (index === 5 && apiAvailable);
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
  document.querySelector("#cloud-pair-form")?.addEventListener("submit", pairCloud);
  document.querySelector("#cloud-sync-now")?.addEventListener("click", syncCloud);
  document.querySelector("#cloud-vault-test")?.addEventListener("click", testCloudVault);
  document.querySelector("#cloud-disconnect")?.addEventListener("click", disconnectCloud);
  document.querySelectorAll("[data-cloud-operation]").forEach((button) => {
    button.addEventListener("click", enqueueCloudOperation);
  });
  document.querySelector("#create-manual-backup")?.addEventListener("click", createManualBackup);
  document.querySelector("#recovery-package-form")?.addEventListener("submit", createRecoveryPackage);
  document.querySelector("#import-recovery-form")?.addEventListener("submit", importRecoveryPackage);
  document.querySelector("#check-updates")?.addEventListener("click", checkUpdates);
  document.querySelector("#download-update")?.addEventListener("click", downloadUpdate);
  document.querySelector("#apply-update")?.addEventListener("click", applyUpdate);
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

async function pairCloud(event) {
  event.preventDefault();
  const cloudBaseUrl = document.querySelector("#cloud-base-url")?.value?.trim() || "";
  const pairingCode = document.querySelector("#cloud-pairing-code")?.value?.trim() || "";
  await withBusy(async () => {
    await invoke("homeserver_pair_cloud", {
      request: { cloud_base_url: cloudBaseUrl, pairing_code: pairingCode },
    });
    return { kind: "success", message: "HomeServer paired and its first signed cloud request was verified." };
  });
}

async function syncCloud() {
  await withBusy(async () => {
    const result = await invoke("homeserver_sync_cloud");
    return {
      kind: "success",
      message: `Cloud synchronization completed: ${result.accepted || 0} accepted, ${result.rejected || 0} rejected, ${result.review || 0} queued for review.`,
    };
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
  const payload =
    operationType === "local.settings.snapshot"
      ? { source: "control_center", captured_at_utc: new Date().toISOString() }
      : { source: "control_center", requested_at_utc: new Date().toISOString() };
  await withBusy(async () => {
    const result = await invoke("homeserver_enqueue_cloud_sync", {
      request: { operation_type: operationType, payload, idempotency_key: null },
    });
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
    return {
      kind: "success",
      message: `${result.message} Use Export beside the package to save a disaster-recovery copy.`,
    };
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
      const destination = await invoke("homeserver_export_recovery_package", {
        backupId,
        suggestedFileName: button.dataset.backupFileName || "Microgifter-HomeServer-Recovery.mghbackup",
      });
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
  const results = await Promise.allSettled([
    invoke("homeserver_status"),
    invoke("homeserver_cloud_status"),
    invoke("homeserver_backups"),
    invoke("homeserver_updates"),
  ]);

  if (results[0].status === "rejected") {
    statusSnapshot = null;
    cloudSnapshot = null;
    backupCatalog = null;
    updateStatus = null;
    notice = { kind: "warning", message: `HomeServer service unavailable: ${String(results[0].reason)}` };
    render();
    return;
  }

  statusSnapshot = results[0].value;
  cloudSnapshot = results[1].status === "fulfilled" ? results[1].value : { state: "degraded", scopes: [], pending_sync: 0, last_error: "cloud_status_unavailable" };
  backupCatalog = results[2].status === "fulfilled" ? results[2].value : null;
  updateStatus = results[3].status === "fulfilled" ? results[3].value : null;
  if (!notice && results[1].status === "rejected") {
    notice = { kind: "warning", message: `Cloud connector unavailable: ${String(results[1].reason)}` };
  }
  render();
}

render();
loadAll();
