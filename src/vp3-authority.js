import { invoke } from "@tauri-apps/api/core";
import "./vp3-authority.css";

let snapshot = null;
let identity = null;
let loading = false;
let notice = null;
let installed = false;
let shellObserver = null;

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

function compact(value) {
  const text = String(value || "");
  if (!text) return "Not assigned";
  if (text.length <= 22) return text;
  return `${text.slice(0, 9)}…${text.slice(-9)}`;
}

function authorityState() {
  if (!snapshot) return { label: "Loading", tone: "attention" };
  if (snapshot.configured && snapshot.activation_state === "active") return { label: "VP3 active", tone: "active" };
  if (snapshot.activation_state === "disconnected") return { label: "Legacy fallback", tone: "attention" };
  return { label: humanize(snapshot.activation_state || "not activated"), tone: "attention" };
}

function render() {
  if (window.location.hash !== "#settings") return;
  const target = document.querySelector(".settings-layout");
  if (!target) return;
  let root = document.querySelector("#vp3-authority-section");
  if (!root) {
    root = document.createElement("section");
    root.id = "vp3-authority-section";
    root.className = "vp3-authority-section";
    target.insertAdjacentElement("afterend", root);
  }

  const state = authorityState();
  const active = Boolean(snapshot?.configured && snapshot?.activation_state === "active");
  const fingerprint = identity?.device_fingerprint || "Unavailable";
  root.innerHTML = `
    <article class="panel vp3-authority-card">
      <header class="vp3-authority-heading">
        <div><span class="vp3-kicker">Software licensing and update authority</span><h2>VP3 HomeServer Activation</h2><p>Register this HomeServer with your VP3 account, verify its signed entitlement lease, and manage the update-authority connection without exposing local data.</p></div>
        <span class="vp3-state ${escapeHtml(state.tone)}">${escapeHtml(state.label)}</span>
      </header>
      ${notice ? `<div class="vp3-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}
      <div class="vp3-authority-grid">
        <div class="vp3-authority-main">
          <div class="vp3-identity-box"><strong>Local device identity</strong><div class="vp3-identity-row"><input id="vp3-device-fingerprint" type="text" readonly value="${escapeHtml(fingerprint)}"><button id="vp3-copy-fingerprint" class="button ghost" type="button" ${identity?.device_fingerprint ? "" : "disabled"}>Copy</button></div><small>This SHA-256 value is derived locally from the persistent HomeServer installation identity. It is not a hardware serial number and cannot be replaced by the activation caller.</small></div>
          <div class="vp3-metrics">
            <div class="vp3-metric"><span>Authority</span><strong>${escapeHtml(snapshot?.authority?.authority || "microgifter_legacy")}</strong></div>
            <div class="vp3-metric"><span>Lease</span><strong>${escapeHtml(snapshot?.lease_public_id ? humanize(snapshot?.authority?.lease_state || "active") : "Not issued")}</strong></div>
            <div class="vp3-metric"><span>Credential vault</span><strong>${snapshot?.credential_in_os_vault ? "Configured" : "Empty"}</strong></div>
          </div>
          ${active ? renderActiveActions() : renderActivationForm()}
        </div>
        <aside class="vp3-authority-side">
          <div class="vp3-detail-card"><strong>Authority details</strong><dl class="vp3-detail-list">
            <div><dt>VP3 account</dt><dd>${escapeHtml(snapshot?.account_id ?? "Not assigned")}</dd></div>
            <div><dt>Device ID</dt><dd title="${escapeHtml(snapshot?.device_public_id)}">${escapeHtml(compact(snapshot?.device_public_id))}</dd></div>
            <div><dt>License</dt><dd title="${escapeHtml(snapshot?.license_public_id)}">${escapeHtml(compact(snapshot?.license_public_id))}</dd></div>
            <div><dt>Lease expires</dt><dd>${escapeHtml(formatDate(snapshot?.lease_expires_at_utc))}</dd></div>
            <div><dt>Last heartbeat</dt><dd>${escapeHtml(formatDate(snapshot?.last_heartbeat_at_utc))}</dd></div>
            <div><dt>Last release check</dt><dd>${escapeHtml(formatDate(snapshot?.last_manifest_checked_at_utc))}</dd></div>
            <div><dt>Last error</dt><dd>${escapeHtml(humanize(snapshot?.last_error_code || "none"))}</dd></div>
          </dl></div>
          <div class="vp3-privacy-card"><strong>Privacy boundary</strong><p>The one-time credential and enrollment code move directly into the Windows credential vault and are cleared from the form. SQLite stores identifiers, signed hashes, lifecycle state, and receipts only. VP3 does not receive Knowledge Vault content, prompts, conversations, models, or unrelated provider credentials.</p></div>
        </aside>
      </div>
    </article>`;
  bind(root);
}

function renderActivationForm() {
  return `<form id="vp3-activation-form" class="vp3-activation-form">
    <div class="vp3-secret-note">First open the VP3 HomeServer fleet page, register this fingerprint to an eligible license, then paste the one-time registration bundle below.</div>
    <label><span>VP3 account ID</span><input id="vp3-account-id" type="number" min="1" step="1" autocomplete="off" required></label>
    <label><span>Device public ID</span><input id="vp3-device-public-id" type="text" maxlength="48" autocomplete="off" required></label>
    <label><span>License public ID (optional)</span><input id="vp3-license-public-id" type="text" maxlength="48" autocomplete="off"></label>
    <label><span>One-time device credential</span><input id="vp3-device-credential" type="password" maxlength="256" autocomplete="new-password" required></label>
    <label><span>One-time enrollment code</span><input id="vp3-enrollment-code" type="password" maxlength="256" autocomplete="new-password" required></label>
    <div class="vp3-action-row"><button class="button primary" type="submit" ${loading ? "disabled" : ""}>Activate VP3 Authority</button><button id="vp3-refresh" class="button ghost" type="button" ${loading ? "disabled" : ""}>Refresh Status</button></div>
  </form>`;
}

function renderActiveActions() {
  return `<div class="vp3-detail-card"><strong>Authority operations</strong><p>Use these actions to verify the deployed VP3 service and signed-update contract from this HomeServer.</p><div class="vp3-action-row"><button class="button secondary" id="vp3-heartbeat" type="button" ${loading ? "disabled" : ""}>Send Heartbeat</button><button class="button secondary" id="vp3-refresh-lease" type="button" ${loading ? "disabled" : ""}>Refresh Lease</button><button class="button secondary" id="vp3-check-update" type="button" ${loading ? "disabled" : ""}>Check Release</button><button class="button secondary" id="vp3-submit-receipts" type="button" ${loading ? "disabled" : ""}>Submit Receipts</button><button class="button danger" id="vp3-disconnect" type="button" ${loading ? "disabled" : ""}>Disconnect VP3</button></div></div>`;
}

function bind(root) {
  root.querySelector("#vp3-copy-fingerprint")?.addEventListener("click", copyFingerprint);
  root.querySelector("#vp3-activation-form")?.addEventListener("submit", activate);
  root.querySelector("#vp3-refresh")?.addEventListener("click", load);
  root.querySelector("#vp3-heartbeat")?.addEventListener("click", () => runAction("homeserver_vp3_heartbeat", {}, "VP3 heartbeat completed."));
  root.querySelector("#vp3-refresh-lease")?.addEventListener("click", () => runAction("homeserver_vp3_refresh_lease", {}, "Signed entitlement lease refreshed."));
  root.querySelector("#vp3-check-update")?.addEventListener("click", () => runAction("homeserver_vp3_check_update", {}, "VP3 release check completed."));
  root.querySelector("#vp3-submit-receipts")?.addEventListener("click", () => runAction("homeserver_vp3_submit_receipts", {}, "Pending VP3 update receipts submitted."));
  root.querySelector("#vp3-disconnect")?.addEventListener("click", disconnect);
}

async function copyFingerprint() {
  if (!identity?.device_fingerprint) return;
  try {
    await navigator.clipboard.writeText(identity.device_fingerprint);
    notice = { kind: "success", message: "Local HomeServer device fingerprint copied." };
  } catch {
    notice = { kind: "warning", message: identity.device_fingerprint };
  }
  render();
}

async function activate(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const confirmation = window.prompt("This moves VP3 authority credentials into Windows Credential Manager. Type ACTIVATE VP3 to continue:");
  if (confirmation !== "ACTIVATE VP3") return;
  loading = true;
  notice = null;
  render();
  try {
    snapshot = await invoke("homeserver_activate_vp3_authority", {
      accountId: Number(form.querySelector("#vp3-account-id")?.value || 0),
      devicePublicId: form.querySelector("#vp3-device-public-id")?.value?.trim() || "",
      licensePublicId: form.querySelector("#vp3-license-public-id")?.value?.trim() || null,
      credential: form.querySelector("#vp3-device-credential")?.value || "",
      enrollmentCode: form.querySelector("#vp3-enrollment-code")?.value || "",
      confirmation,
    });
    form.reset();
    notice = { kind: "success", message: "VP3 software authority activated and its signed lease verified." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    loading = false;
    await load(false);
  }
}

async function runAction(command, args, successMessage) {
  loading = true;
  notice = null;
  render();
  try {
    snapshot = await invoke(command, args);
    notice = { kind: "success", message: successMessage };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    loading = false;
    await load(false);
  }
}

async function disconnect() {
  const confirmation = window.prompt("Disconnecting returns software authority to the explicit legacy fallback. Type DISCONNECT VP3 to continue:");
  if (confirmation !== "DISCONNECT VP3") return;
  await runAction("homeserver_disconnect_vp3_authority", { confirmation }, "VP3 software authority disconnected locally.");
}

async function load(clearNotice = true) {
  if (window.location.hash !== "#settings") return;
  loading = true;
  if (clearNotice) notice = null;
  render();
  try {
    [snapshot, identity] = await Promise.all([
      invoke("homeserver_vp3_authority_status"),
      invoke("homeserver_vp3_device_identity"),
    ]);
  } catch (error) {
    notice = { kind: "warning", message: `VP3 authority controls are unavailable: ${String(error)}` };
  } finally {
    loading = false;
    render();
  }
}

function mount() {
  if (window.location.hash !== "#settings") return;
  render();
  if (!snapshot && !loading) load();
}

function install() {
  if (installed) return;
  installed = true;
  window.addEventListener("homeserver:rendered", mount);
  window.addEventListener("hashchange", mount);
  window.addEventListener("DOMContentLoaded", mount, { once: true });
  const app = document.querySelector("#app");
  if (app) {
    shellObserver = new MutationObserver(() => {
      if (window.location.hash === "#settings" && !document.querySelector("#vp3-authority-section")) {
        queueMicrotask(mount);
      }
    });
    shellObserver.observe(app, { childList: true, subtree: true });
  }
  mount();
}

install();
