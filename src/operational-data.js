import { invoke } from "@tauri-apps/api/core";
import "./operational-data.css";

const PAGE_KEY = "operational";
let snapshot = null;
let queryResult = null;
let loading = false;
let busy = false;
let notice = null;
let grantDataset = null;
let initialized = false;

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

function statusClass(value) {
  return String(value || "unknown").toLowerCase().replaceAll("_", "-");
}

function formatDate(value) {
  if (!value) return "Not yet";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function compact(value, limit = 28) {
  const text = String(value || "");
  if (text.length <= limit) return text || "Not assigned";
  return `${text.slice(0, Math.floor(limit / 2))}…${text.slice(-Math.floor(limit / 2))}`;
}

function currentHash() {
  return window.location.hash.replace("#", "");
}

function isOperationalPage() {
  return currentHash() === PAGE_KEY;
}

function datasets() {
  return Array.isArray(snapshot?.datasets) ? snapshot.datasets : [];
}

function enabledDatasets() {
  return datasets().filter((dataset) => dataset.grant_state === "enabled");
}

function injectNavigation() {
  const nav = document.querySelector(".primary-nav");
  if (!nav || nav.querySelector('[data-operational-nav="true"]')) return;
  const button = document.createElement("button");
  button.type = "button";
  button.className = `nav-item operational-navigation-item ${isOperationalPage() ? "active" : ""}`;
  button.dataset.operationalNav = "true";
  button.innerHTML = '<span aria-hidden="true">▦</span><span>Operational Data</span>';
  const agentButton = nav.querySelector('[data-agent-workspace-nav="true"]');
  if (agentButton?.nextSibling) nav.insertBefore(button, agentButton.nextSibling);
  else nav.prepend(button);
  button.addEventListener("click", () => {
    window.location.hash = `#${PAGE_KEY}`;
    window.setTimeout(() => mount(true), 0);
  });
}

function datasetIdentity(dataset) {
  return `${dataset.connection_id}|${dataset.dataset_key}`;
}

function selectedDataset() {
  return datasets().find((dataset) => datasetIdentity(dataset) === grantDataset) || null;
}

function metric(label, value, detail) {
  return `<article class="operational-metric"><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong><small>${escapeHtml(detail)}</small></article>`;
}

function datasetCard(dataset) {
  const enabled = dataset.grant_state === "enabled";
  const freshness = dataset.last_successful_sync_utc ? formatDate(dataset.last_successful_sync_utc) : "No import yet";
  return `<article class="operational-dataset-card">
    <div><span class="operational-state ${escapeHtml(statusClass(dataset.grant_state))}">${escapeHtml(humanize(dataset.grant_state))}</span><h3>${escapeHtml(dataset.connection_name)} · ${escapeHtml(dataset.label)}</h3><p>${escapeHtml(dataset.description)}</p></div>
    <div class="operational-dataset-meta"><span>Authority</span><strong>${escapeHtml(dataset.authority)}</strong><span>${escapeHtml(dataset.sensitivity)} sensitivity</span></div>
    <div class="operational-dataset-meta"><span>Local evidence</span><strong>${Number(dataset.record_count || 0)} records · ${Number(dataset.event_count || 0)} events</strong><span>${escapeHtml(freshness)}</span></div>
    <div class="operational-card-actions"><button type="button" class="button ${enabled ? "secondary" : "primary"}" data-operational-grant="${escapeHtml(datasetIdentity(dataset))}">${enabled ? "Manage" : "Authorize"}</button>${enabled ? `<button type="button" class="button ghost" data-operational-pause="${escapeHtml(datasetIdentity(dataset))}">Pause</button>` : ""}</div>
  </article>`;
}

function renderDatasets() {
  if (!datasets().length) return '<div class="operational-empty">Pair a supported provider connection to review its declared datasets.</div>';
  return datasets().map(datasetCard).join("");
}

function renderRuns() {
  const runs = snapshot?.recent_runs || [];
  if (!runs.length) return '<div class="operational-empty">No operational import runs have been recorded.</div>';
  return runs.slice(0, 30).map((run) => `<article class="operational-run"><span class="operational-state ${escapeHtml(statusClass(run.state))}">${escapeHtml(humanize(run.state))}</span><h3>${escapeHtml(run.dataset_key)} · ${escapeHtml(humanize(run.import_mode))}</h3><p>${Number(run.records_imported || 0)} imported · ${Number(run.records_rejected || 0)} rejected · ${Number(run.events_received || 0)} events</p><small>${escapeHtml(formatDate(run.completed_at_utc || run.started_at_utc))} · cursor ${escapeHtml(compact(run.cursor_after || run.cursor_before || "not assigned"))}</small></article>`).join("");
}

function renderEvidence() {
  const records = queryResult?.records || [];
  if (!queryResult) return '<div class="operational-empty">Select an authorized dataset to inspect locally imported evidence.</div>';
  if (!records.length) return `<div class="operational-empty">No imported records match this query. ${Number(queryResult.available_records || 0)} records are available in the selected scope.</div>`;
  return records.map((record) => `<article class="operational-evidence"><h3>${escapeHtml(record.source_object_type)} · ${escapeHtml(record.source_object_id)}</h3><p>Revision ${escapeHtml(record.source_revision)} · received ${escapeHtml(formatDate(record.received_at_utc))}</p><code>${escapeHtml(JSON.stringify(record.payload, null, 2).slice(0, 1800))}</code><small>${escapeHtml(record.citation)} · hash ${escapeHtml(compact(record.payload_hash, 34))}</small></article>`).join("");
}

function datasetOptions() {
  return enabledDatasets().map((dataset) => `<option value="${escapeHtml(datasetIdentity(dataset))}">${escapeHtml(dataset.connection_name)} · ${escapeHtml(dataset.label)}</option>`).join("");
}

function renderGrantModal() {
  const dataset = selectedDataset();
  if (!dataset) return "";
  const uses = new Set(dataset.permitted_agent_uses || ["read", "analyze", "goal_match", "report"]);
  return `<div class="operational-modal-backdrop" data-operational-close-modal><div class="operational-modal" role="dialog" aria-modal="true" aria-label="Authorize operational dataset"><div class="operational-modal-head"><div><h2>${dataset.grant_state === "enabled" ? "Manage" : "Authorize"} ${escapeHtml(dataset.label)}</h2><p>${escapeHtml(dataset.connection_name)} · provider-authoritative data copied locally as untrusted evidence.</p></div><button class="icon-button" type="button" data-operational-close-modal>×</button></div><form class="operational-form" id="operational-grant-form"><input type="hidden" id="operational-grant-connection" value="${escapeHtml(dataset.connection_id)}"><input type="hidden" id="operational-grant-dataset" value="${escapeHtml(dataset.dataset_key)}"><label>Classification<select id="operational-grant-classification"><option value="business" ${dataset.classification === "business" ? "selected" : ""}>Business</option><option value="restricted" ${dataset.classification === "restricted" ? "selected" : ""}>Restricted</option><option value="sensitive" ${dataset.classification === "sensitive" ? "selected" : ""}>Sensitive</option></select></label><label>Retention days<input id="operational-grant-retention" type="number" min="1" max="3650" value="${Number(dataset.retention_days || 365)}" required></label><div><strong>Permitted Agent Workspace uses</strong><div class="operational-checks"><label><input type="checkbox" name="operational-agent-use" value="read" ${uses.has("read") ? "checked" : ""}>Read evidence</label><label><input type="checkbox" name="operational-agent-use" value="analyze" ${uses.has("analyze") ? "checked" : ""}>Analyze</label><label><input type="checkbox" name="operational-agent-use" value="goal_match" ${uses.has("goal_match") ? "checked" : ""}>Match goals</label><label><input type="checkbox" name="operational-agent-use" value="report" ${uses.has("report") ? "checked" : ""}>Create reports</label></div></div><div class="operational-inline-actions"><button type="button" class="button ghost" data-operational-close-modal>Cancel</button><button type="submit" class="button primary" ${busy ? "disabled" : ""}>Save Authorization</button></div></form></div></div>`;
}

function renderPage() {
  const enabled = Number(snapshot?.enabled_grants || 0);
  return `<div class="operational-page" data-operational-mounted="true"><header class="operational-header"><div><h1>Operational Data</h1><p>Authorize provider-declared datasets, inspect import freshness and provenance, and make structured evidence available to the HomeServer Agent Workspace.</p></div><div class="operational-actions"><span class="operational-state enabled">Local evidence store</span><button type="button" class="button secondary" id="operational-refresh" ${loading || busy ? "disabled" : ""}>Refresh</button></div></header>${notice ? `<div class="operational-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}<section class="operational-boundary"><article><strong>Provider authority preserved</strong><span>Microgifter remains authoritative for merchants, products, campaigns, CRM, claims, rewards, and redemptions.</span></article><article><strong>Explicit dataset grants</strong><span>${enabled} dataset grant${enabled === 1 ? "" : "s"} currently allow provider-to-HomeServer imports and Agent Workspace use.</span></article><article><strong>Untrusted evidence boundary</strong><span>Imported strings and records are evidence, never executable instructions and never a policy override.</span></article></section><section class="operational-metrics">${metric("Provider manifests", String(snapshot?.provider_manifests || 0), "Installed and allowlisted")}${metric("Authorized datasets", String(enabled), "Connection-specific grants")}${metric("Imported records", String(snapshot?.imported_records || 0), "Current normalized entities")}${metric("Imported events", String(snapshot?.imported_events || 0), "Provider event timeline")}${metric("Quarantined errors", String(snapshot?.quarantined_errors || 0), "Rejected evidence retained for review")}</section><section class="operational-layout"><main class="operational-panel"><div class="operational-panel-head"><div><h2>Provider Dataset Catalog</h2><p>Each paired connection exposes only datasets declared by its audited provider adapter.</p></div><span>${datasets().length} available</span></div><div class="operational-dataset-list">${renderDatasets()}</div><div class="operational-footnote">Detailed payment data, private messages, gift ownership, and full customer contact records are not present in the Phase 5C-A manifest.</div></main><aside class="operational-side"><section class="operational-panel"><div class="operational-panel-head"><div><h2>Evidence Query</h2><p>Read authorized records with source revisions, hashes, and citations.</p></div></div><form id="operational-query-form" class="operational-query-form"><label>Dataset<select id="operational-query-dataset" ${enabledDatasets().length ? "" : "disabled"}>${datasetOptions()}</select></label><label>Object type <small>optional</small><input id="operational-query-type" maxlength="160" placeholder="product"></label><label>Maximum results<input id="operational-query-limit" type="number" min="1" max="100" value="25"></label><button class="button primary" type="submit" ${enabledDatasets().length && !busy ? "" : "disabled"}>Query Local Evidence</button></form><div class="operational-evidence-list">${renderEvidence()}</div></section><section class="operational-panel"><div class="operational-panel-head"><div><h2>Recent Imports</h2><p>Snapshot, incremental, and event ingestion receipts.</p></div></div><div class="operational-run-list">${renderRuns()}</div></section></aside></section>${renderGrantModal()}</div>`;
}

function mount(force = false) {
  injectNavigation();
  document.querySelector('[data-operational-nav="true"]')?.classList.toggle("active", isOperationalPage());
  if (!isOperationalPage()) return;
  const canvas = document.querySelector(".page-canvas");
  if (!canvas) return;
  if (!force && canvas.querySelector('[data-operational-mounted="true"]')) return;
  canvas.innerHTML = loading && !snapshot ? '<div class="operational-page"><div class="operational-empty">Loading operational manifests, grants, imports, and provenance…</div></div>' : renderPage();
  bindEvents();
  if (!snapshot && !loading) void refresh();
}

function bindEvents() {
  document.querySelector("#operational-refresh")?.addEventListener("click", refresh);
  document.querySelector("#operational-query-form")?.addEventListener("submit", queryEvidence);
  document.querySelector("#operational-grant-form")?.addEventListener("submit", saveGrant);
  document.querySelectorAll("[data-operational-grant]").forEach((button) => button.addEventListener("click", () => {
    grantDataset = button.dataset.operationalGrant;
    mount(true);
  }));
  document.querySelectorAll("[data-operational-pause]").forEach((button) => button.addEventListener("click", pauseGrant));
  document.querySelectorAll("[data-operational-close-modal]").forEach((element) => element.addEventListener("click", (event) => {
    if (element.classList.contains("operational-modal-backdrop") && event.target !== element) return;
    grantDataset = null;
    mount(true);
  }));
}

async function refresh() {
  loading = true;
  mount(true);
  try {
    snapshot = await invoke("homeserver_operational_data");
    notice = { kind: "success", message: "Operational dataset catalog and local evidence state refreshed." };
  } catch (error) {
    notice = { kind: "warning", message: `Operational data unavailable: ${String(error)}` };
  } finally {
    loading = false;
    mount(true);
  }
}

function checkedUses() {
  return [...document.querySelectorAll('input[name="operational-agent-use"]:checked')].map((input) => input.value);
}

async function saveGrant(event) {
  event.preventDefault();
  busy = true;
  mount(true);
  try {
    snapshot = await invoke("homeserver_update_operational_dataset_grant", { request: {
      connection_id: document.querySelector("#operational-grant-connection")?.value || "",
      dataset_key: document.querySelector("#operational-grant-dataset")?.value || "",
      enabled: true,
      retention_days: Number(document.querySelector("#operational-grant-retention")?.value || 365),
      classification: document.querySelector("#operational-grant-classification")?.value || "business",
      permitted_agent_uses: checkedUses(),
    } });
    grantDataset = null;
    notice = { kind: "success", message: "Dataset authorization saved locally. The provider may now import this dataset under the signed connection scope." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    mount(true);
  }
}

async function pauseGrant(event) {
  const identity = event.currentTarget.dataset.operationalPause;
  const dataset = datasets().find((item) => datasetIdentity(item) === identity);
  if (!dataset || !window.confirm(`Pause imports and Agent Workspace access for ${dataset.label}? Existing local history will be retained.`)) return;
  busy = true;
  mount(true);
  try {
    snapshot = await invoke("homeserver_update_operational_dataset_grant", { request: {
      connection_id: dataset.connection_id,
      dataset_key: dataset.dataset_key,
      enabled: false,
      retention_days: dataset.retention_days,
      classification: dataset.classification,
      permitted_agent_uses: dataset.permitted_agent_uses || [],
    } });
    queryResult = null;
    notice = { kind: "success", message: "Dataset grant paused. Future imports and agent queries are blocked; retained evidence was not deleted." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    mount(true);
  }
}

async function queryEvidence(event) {
  event.preventDefault();
  const [connectionId, datasetKey] = String(document.querySelector("#operational-query-dataset")?.value || "").split("|");
  if (!connectionId || !datasetKey) return;
  busy = true;
  mount(true);
  try {
    queryResult = await invoke("homeserver_query_operational_data", { request: {
      connection_id: connectionId,
      dataset_key: datasetKey,
      source_object_type: document.querySelector("#operational-query-type")?.value?.trim() || null,
      limit: Number(document.querySelector("#operational-query-limit")?.value || 25),
    } });
    notice = { kind: "success", message: `Loaded ${queryResult.records?.length || 0} evidence records from local operational storage.` };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    busy = false;
    mount(true);
  }
}

const app = document.querySelector("#app");
if (app) {
  const observer = new MutationObserver(() => {
    injectNavigation();
    if (isOperationalPage()) mount(false);
  });
  observer.observe(app, { childList: true, subtree: true });
}

window.addEventListener("hashchange", () => window.setTimeout(() => mount(true), 0));
window.addEventListener("DOMContentLoaded", () => {
  if (initialized) return;
  initialized = true;
  window.setTimeout(() => mount(true), 0);
});
