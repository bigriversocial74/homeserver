import { invoke } from "@tauri-apps/api/core";
import "./cloud-connections.css";

let snapshot = null;
let podSnapshot = null;
let loading = false;
let actionBusy = false;
let notice = null;
let selectedProvider = "microgifter";

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

function argumentText(values) {
  return Array.isArray(values) ? values.join("\n") : "";
}

function podConnectionCard(connection) {
  const runtime = connection.runtime || {};
  const capabilities = Array.isArray(connection.granted_capabilities) ? connection.granted_capabilities : [];
  return `<article class="cloud-connection-card pod-provider-card" data-pod-connection-id="${escapeHtml(connection.connection_id)}">
    <div class="cloud-connection-card-head">
      <div><span class="cloud-provider-label">POD Provider</span><h3>${escapeHtml(connection.display_name)}</h3><p>${escapeHtml(connection.provider_display_name || connection.provider_identity_id)}</p></div>
      <div class="cloud-connection-status">${badge(connection.state)}${badge(runtime.runtime_state)}</div>
    </div>
    <dl class="cloud-connection-details pod-connection-details">
      <div><dt>POD URL</dt><dd>${escapeHtml(connection.pod_base_url)}</dd></div>
      <div><dt>Device</dt><dd class="mono">${escapeHtml(connection.device_id)}</dd></div>
      <div><dt>Last heartbeat</dt><dd>${escapeHtml(formatDate(connection.last_heartbeat_at_utc))}</dd></div>
      <div><dt>Last poll</dt><dd>${escapeHtml(formatDate(connection.last_poll_at_utc))}</dd></div>
      <div><dt>Active jobs</dt><dd>${Number(connection.active_jobs || 0)}</dd></div>
      <div><dt>Failed jobs</dt><dd>${Number(connection.failed_jobs || 0)}</dd></div>
    </dl>
    ${connection.last_error ? `<div class="cloud-connection-error">${escapeHtml(connection.last_error)}</div>` : ""}
    ${runtime.runtime_health_message ? `<div class="pod-runtime-health">${escapeHtml(runtime.runtime_health_message)}</div>` : ""}
    <div class="cloud-connection-scopes">${capabilities.map((scope) => `<span>${escapeHtml(scope)}</span>`).join("")}</div>
    <form class="pod-runtime-form" data-pod-runtime-form data-connection-id="${escapeHtml(connection.connection_id)}">
      <div class="pod-runtime-heading"><div><strong>Local voice runtime</strong><span>Absolute executables only. Commands run without a shell.</span></div></div>
      <div class="pod-runtime-grid">
        <label class="pod-runtime-toggle"><input name="transcription_enabled" type="checkbox" ${runtime.transcription_enabled ? "checked" : ""}><span>Enable speech-to-text</span></label>
        <label><span>Transcription executable</span><input name="transcription_executable" value="${escapeHtml(runtime.transcription_executable || "")}" placeholder="C:\\Program Files\\VoiceRuntime\\transcribe.exe"></label>
        <label><span>Transcription model</span><input name="transcription_model" value="${escapeHtml(runtime.transcription_model || "")}" placeholder="Local model name"></label>
        <label><span>Transcription arguments <small>one per line</small></span><textarea name="transcription_arguments">${escapeHtml(argumentText(runtime.transcription_arguments))}</textarea></label>
        <label class="pod-runtime-toggle"><input name="synthesis_enabled" type="checkbox" ${runtime.synthesis_enabled ? "checked" : ""}><span>Enable text-to-speech</span></label>
        <label><span>Synthesis executable</span><input name="synthesis_executable" value="${escapeHtml(runtime.synthesis_executable || "")}" placeholder="C:\\Program Files\\VoiceRuntime\\synthesize.exe"></label>
        <label><span>Synthesis model</span><input name="synthesis_model" value="${escapeHtml(runtime.synthesis_model || "")}" placeholder="Local model name"></label>
        <label><span>Synthesis voice</span><input name="synthesis_voice" value="${escapeHtml(runtime.synthesis_voice || "")}" placeholder="Optional voice"></label>
        <label><span>Synthesis arguments <small>one per line</small></span><textarea name="synthesis_arguments">${escapeHtml(argumentText(runtime.synthesis_arguments))}</textarea></label>
        <label><span>Timeout seconds</span><input name="execution_timeout_seconds" type="number" min="5" max="1800" value="${Number(runtime.execution_timeout_seconds || 120)}"></label>
        <label><span>Maximum input bytes</span><input name="maximum_input_bytes" type="number" min="262144" max="16777216" value="${Number(runtime.maximum_input_bytes || 8388608)}"></label>
        <label><span>Maximum output bytes</span><input name="maximum_output_bytes" type="number" min="262144" max="16777216" value="${Number(runtime.maximum_output_bytes || 8388608)}"></label>
      </div>
      <div class="cloud-connection-actions pod-runtime-actions">
        <button class="button primary" type="submit" ${actionBusy ? "disabled" : ""}>Save Runtime</button>
        <button class="button secondary" type="button" data-pod-action="poll" data-connection-id="${escapeHtml(connection.connection_id)}" ${actionBusy || connection.state === "disconnected" ? "disabled" : ""}>Poll Now</button>
        <button class="button ghost danger" type="button" data-pod-action="disconnect" data-connection-id="${escapeHtml(connection.connection_id)}" ${actionBusy || connection.state === "disconnected" ? "disabled" : ""}>Disconnect POD</button>
      </div>
    </form>
  </article>`;
}

function providerFields() {
  const pod = selectedProvider === "pod";
  return `<label><span>${pod ? "POD URL" : "Cloud URL"}</span><input id="cloud-connection-url" type="url" maxlength="300" value="${pod ? "" : "https://microgifter.com"}" placeholder="${pod ? "https://pod.example.com" : "https://microgifter.com"}" required></label>
    <label><span>${pod ? "POD Sync Code" : "Pairing token"}</span><input id="cloud-connection-token" type="password" minlength="20" maxlength="120" autocomplete="one-time-code" placeholder="${pod ? "POD-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX" : "Paste the one-time pairing token"}" required></label>
    ${pod ? "" : '<div class="cloud-connection-form-row"><label><span>Tenant ID <small>optional</small></span><input id="cloud-connection-tenant" maxlength="120" placeholder="Company or account"></label><label><span>Site ID <small>optional</small></span><input id="cloud-connection-site" maxlength="120" placeholder="Location or workspace"></label></div>'}`;
}

function renderPanel() {
  const connections = Array.isArray(snapshot?.connections) ? snapshot.connections : [];
  const podConnections = Array.isArray(podSnapshot?.connections) ? podSnapshot.connections : [];
  const noticeMarkup = notice ? `<div class="cloud-connection-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : "";
  return `<section class="panel cloud-connections-panel" id="cloud-connections-registry">
    <div class="panel-title cloud-connections-title">
      <div><h2>Cloud & POD Connection Registry</h2><p>Pair this HomeServer with Microgifter sites and independently owned POD wrappers. Every connection keeps separate credentials, capabilities, jobs, receipts, and revocation state.</p></div>
      <div class="cloud-connection-title-actions"><span class="planned-label">${Number(snapshot?.active_connections || 0)} cloud active · ${podConnections.length} POD</span><button class="button secondary" id="cloud-connections-refresh" type="button" ${loading || actionBusy ? "disabled" : ""}>Refresh</button><button class="button primary" id="cloud-connections-sync-all" type="button" ${loading || actionBusy || !snapshot?.active_connections ? "disabled" : ""}>Sync All Cloud</button></div>
    </div>
    ${noticeMarkup}
    <div class="cloud-connections-boundary"><strong>Connection-scoped authority</strong><span>Microgifter synchronization, POD voice jobs, updater authorization, local Knowledge Vault, and other wrappers remain separate. HomeServer continues operating locally with zero external connections.</span></div>
    <div class="cloud-connections-layout">
      <form id="cloud-connection-pair-form" class="cloud-connection-form">
        <div><h3>Pair a Connection</h3><p>${selectedProvider === "pod" ? "Exchange one short-lived POD Sync Code. The returned bearer and signing seed stay in the Windows credential vault." : "The pairing token is exchanged once and is never retained."}</p></div>
        <label><span>Provider</span><select id="cloud-connection-provider" required><option value="microgifter" ${selectedProvider === "microgifter" ? "selected" : ""}>Microgifter Cloud</option><option value="pod" ${selectedProvider === "pod" ? "selected" : ""}>POD Wrapper</option></select></label>
        <label><span>Connection name</span><input id="cloud-connection-name" minlength="1" maxlength="120" placeholder="${selectedProvider === "pod" ? "Office POD" : "Restaurant A"}" required></label>
        ${providerFields()}
        <label class="cloud-default-check"><input id="cloud-connection-default" type="checkbox"><span>Make this the default connection</span></label>
        <button class="button primary" type="submit" ${actionBusy ? "disabled" : ""}>Pair ${selectedProvider === "pod" ? "POD" : "Connection"}</button>
      </form>
      <div class="cloud-connections-list">
        ${loading && !snapshot ? '<div class="cloud-connections-empty"><strong>Loading connections…</strong></div>' : connections.length ? connections.map(connectionCard).join("") : '<div class="cloud-connections-empty"><strong>Local-only cloud mode</strong><p>No Microgifter cloud connection is required.</p></div>'}
      </div>
    </div>
    <div class="pod-provider-section">
      <div class="pod-provider-title"><div><span class="cloud-provider-label">Local voice processing</span><h3>POD Provider Connections</h3><p>Paired PODs can queue capability, speech-to-text, and text-to-speech jobs. Browser voice remains the fallback when no local runtime is configured.</p></div><span class="planned-label">${Number(podSnapshot?.recent_jobs?.length || 0)} recent jobs</span></div>
      <div class="pod-provider-list">${podConnections.length ? podConnections.map(podConnectionCard).join("") : '<div class="cloud-connections-empty"><strong>No POD is paired</strong><p>Issue a Sync Code from the POD owner console, select POD Wrapper above, and pair it here.</p></div>'}</div>
    </div>
  </section>`;
}

function findMount() {
  const integrationGrid = document.querySelector(".integration-grid");
  if (!integrationGrid) return null;
  return integrationGrid.parentElement;
}

function mount(force = false) {
  const parent = findMount();
  if (!parent) return;
  let panel = document.querySelector("#cloud-connections-registry");
  let changed = false;
  if (!panel) {
    const holder = document.createElement("div");
    holder.innerHTML = renderPanel();
    panel = holder.firstElementChild;
    const mcpPanel = parent.querySelector("#mcp-runtime");
    if (mcpPanel) parent.insertBefore(panel, mcpPanel);
    else parent.append(panel);
    changed = true;
  } else if (force) {
    const holder = document.createElement("div");
    holder.innerHTML = renderPanel();
    const replacement = holder.firstElementChild;
    panel.replaceWith(replacement);
    panel = replacement;
    changed = true;
  }
  if (changed) bindEvents(panel);
  if (!snapshot && !loading) void refresh();
}

function bindEvents(panel) {
  panel.querySelector("#cloud-connection-pair-form")?.addEventListener("submit", pairConnection);
  panel.querySelector("#cloud-connection-provider")?.addEventListener("change", (event) => {
    selectedProvider = event.currentTarget.value === "pod" ? "pod" : "microgifter";
    mount(true);
  });
  panel.querySelector("#cloud-connections-refresh")?.addEventListener("click", () => refresh(true));
  panel.querySelector("#cloud-connections-sync-all")?.addEventListener("click", syncAll);
  panel.querySelectorAll("[data-cloud-action]").forEach((button) => button.addEventListener("click", handleConnectionAction));
  panel.querySelectorAll("[data-pod-action]").forEach((button) => button.addEventListener("click", handlePodAction));
  panel.querySelectorAll("[data-pod-runtime-form]").forEach((form) => form.addEventListener("submit", savePodRuntime));
}

async function refresh(showNotice = false) {
  loading = true;
  mount(true);
  try {
    const [cloudResult, podResult] = await Promise.allSettled([
      invoke("homeserver_cloud_connections"),
      invoke("homeserver_pod_status"),
    ]);
    if (cloudResult.status === "fulfilled") snapshot = cloudResult.value;
    else notice = { kind: "warning", message: `Cloud registry unavailable: ${String(cloudResult.reason)}` };
    if (podResult.status === "fulfilled") podSnapshot = podResult.value;
    else notice = { kind: "warning", message: `POD provider unavailable: ${String(podResult.reason)}` };
    if (showNotice && cloudResult.status === "fulfilled" && podResult.status === "fulfilled") notice = { kind: "info", message: "Cloud and POD connection registries refreshed." };
  } finally {
    loading = false;
    mount(true);
  }
}

async function pairConnection(event) {
  event.preventDefault();
  const form = event.currentTarget;
  actionBusy = true;
  notice = null;
  mount(true);
  try {
    if (selectedProvider === "pod") {
      const request = {
        display_name: form.querySelector("#cloud-connection-name")?.value?.trim() || "",
        pod_base_url: form.querySelector("#cloud-connection-url")?.value?.trim() || "",
        sync_code: form.querySelector("#cloud-connection-token")?.value?.trim() || "",
        make_default: Boolean(form.querySelector("#cloud-connection-default")?.checked),
      };
      const connection = await invoke("homeserver_connect_pod", { request });
      notice = { kind: "success", message: `${connection.display_name} was paired as a POD provider.` };
    } else {
      const request = {
        provider_key: "microgifter",
        display_name: form.querySelector("#cloud-connection-name")?.value?.trim() || "",
        cloud_base_url: form.querySelector("#cloud-connection-url")?.value?.trim() || "",
        pairing_code: form.querySelector("#cloud-connection-token")?.value?.trim() || "",
        tenant_id: form.querySelector("#cloud-connection-tenant")?.value?.trim() || null,
        site_id: form.querySelector("#cloud-connection-site")?.value?.trim() || null,
        make_default: Boolean(form.querySelector("#cloud-connection-default")?.checked),
      };
      const connection = await invoke("homeserver_pair_cloud_connection", { request });
      notice = { kind: "success", message: `${connection.display_name} was paired and verified.` };
    }
    await refresh(false);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount(true);
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
  mount(true);
  try {
    if (action === "sync") {
      const result = await invoke("homeserver_sync_cloud_connection", { connectionId });
      notice = { kind: "success", message: `Connection sync completed: ${Number(result.accepted || 0)} accepted, ${Number(result.rejected || 0)} rejected, ${Number(result.review || 0)} for review.` };
    } else if (action === "disconnect") {
      await invoke("homeserver_disconnect_cloud_connection", { connectionId });
      notice = { kind: "success", message: "Cloud connection disconnected and its credential was removed." };
    }
    await refresh(false);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount(true);
  }
}

function lines(value) {
  return String(value || "").split(/\r?\n/).map((item) => item.trim()).filter(Boolean);
}

async function savePodRuntime(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const connectionId = form.dataset.connectionId || "";
  if (!connectionId) return;
  const data = new FormData(form);
  const request = {
    connection_id: connectionId,
    transcription_enabled: data.get("transcription_enabled") === "on",
    transcription_executable: String(data.get("transcription_executable") || "").trim() || null,
    transcription_arguments: lines(data.get("transcription_arguments")),
    transcription_model: String(data.get("transcription_model") || "").trim() || null,
    synthesis_enabled: data.get("synthesis_enabled") === "on",
    synthesis_executable: String(data.get("synthesis_executable") || "").trim() || null,
    synthesis_arguments: lines(data.get("synthesis_arguments")),
    synthesis_model: String(data.get("synthesis_model") || "").trim() || null,
    synthesis_voice: String(data.get("synthesis_voice") || "").trim() || null,
    execution_timeout_seconds: Number(data.get("execution_timeout_seconds") || 120),
    maximum_input_bytes: Number(data.get("maximum_input_bytes") || 8388608),
    maximum_output_bytes: Number(data.get("maximum_output_bytes") || 8388608),
  };
  actionBusy = true;
  notice = null;
  mount(true);
  try {
    await invoke("homeserver_update_pod_runtime", { request });
    notice = { kind: "success", message: "The local POD voice runtime settings were saved." };
    await refresh(false);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount(true);
  }
}

async function handlePodAction(event) {
  const button = event.currentTarget;
  const connectionId = button.dataset.connectionId || "";
  const action = button.dataset.podAction;
  if (!connectionId) return;
  if (action === "disconnect" && !window.confirm("Disconnect this POD, remove its credential-vault entry, and cancel active POD jobs? Other wrappers remain active.")) return;
  actionBusy = true;
  notice = null;
  mount(true);
  try {
    if (action === "poll") {
      const result = await invoke("homeserver_poll_pod", { connectionId });
      notice = { kind: "success", message: `POD polling completed. ${Number(result.processed_jobs || 0)} job processed.` };
    } else if (action === "disconnect") {
      await invoke("homeserver_disconnect_pod", { connectionId });
      notice = { kind: "success", message: "POD connection disconnected. Local HomeServer operation remains available." };
    }
    await refresh(false);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount(true);
  }
}

async function syncAll() {
  actionBusy = true;
  notice = null;
  mount(true);
  try {
    const result = await invoke("homeserver_sync_all_cloud_connections");
    notice = { kind: "success", message: `All cloud connections synchronized: ${Number(result.processed || 0)} processed, ${Number(result.pending || 0)} still pending.` };
    await refresh(false);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    mount(true);
  }
}

const app = document.querySelector("#app");
if (app) {
  const observer = new MutationObserver(() => mount(false));
  observer.observe(app, { childList: true, subtree: true });
}
window.addEventListener("hashchange", () => window.setTimeout(() => mount(false), 0));
window.addEventListener("DOMContentLoaded", () => window.setTimeout(() => mount(false), 0));
