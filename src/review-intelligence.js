import { invoke } from "@tauri-apps/api/core";
import "./review-intelligence.css";

const PAGE_KEY = "intelligence";
const REVIEW_DATASETS = [
  ["reviews.customer_reviews", "Customer reviews", "Ratings, full review text, order/product context, and response state."],
  ["reviews.resolution_history", "Review resolutions", "Merchant responses, Make-Good history, and resolution evidence."],
  ["conversations.messages", "Messages", "Authorized message bodies for intent, sentiment, commitments, and continuity."],
  ["conversations.threads", "Conversation threads", "Conversation status, ownership, summaries, and closure state."],
  ["conversations.follow_ups", "Conversation follow-ups", "Commitments, next steps, due dates, and closure evidence."],
  ["crm.contacts", "CRM contacts", "Lifecycle, relationship, preferences, contact details, and consent when granted."],
  ["crm.activities", "CRM activities", "Customer timeline, campaign, service, and relationship events."],
  ["crm.notes", "CRM notes", "Authorized merchant notes and relationship context."],
  ["crm.consent", "CRM consent", "Provider-authoritative communication consent and preferences."],
  ["commerce.orders", "Purchase history", "Orders, value, campaign attribution, and service-recovery context without payment credentials."],
  ["commerce.order_items", "Purchased products", "Product, quantity, value, and affinity evidence."],
  ["commerce.refunds", "Refund history", "Refund and recovery evidence without payment credentials."],
  ["gifts.ownership", "Gift and Wallet ownership", "Provider-authoritative PPPM lifecycle copies for analysis."],
  ["gifts.claims", "Gift claims", "Claim history and status evidence."],
  ["gifts.redemptions", "Gift redemptions", "Redemption history and location evidence."],
  ["campaigns.definition", "Campaign definitions", "Campaign type, rules, audience, reward, and lifecycle state."],
  ["campaigns.performance", "Campaign performance", "Delivery, claims, redemptions, conversion, CRM, and follow-up outcomes."],
  ["campaigns.authorizations", "Campaign authorizations", "Merchant-owned provider policies for agent campaign actions."],
];

let snapshot = null;
let connectionsSnapshot = null;
let operationalSnapshot = null;
let modelSnapshot = null;
let loading = false;
let busy = false;
let notice = null;
let selectedConnectionId = "";
let selectedRecommendationId = null;
let lastAnalysis = null;
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

function formatNumber(value, digits = 0) {
  const number = Number(value || 0);
  return Number.isFinite(number) ? number.toLocaleString(undefined, { maximumFractionDigits: digits }) : "0";
}

function currentHash() {
  return window.location.hash.replace("#", "");
}

function isPage() {
  return currentHash() === PAGE_KEY;
}

function connections() {
  return Array.isArray(connectionsSnapshot?.connections) ? connectionsSnapshot.connections : [];
}

function connectedConnections() {
  return connections().filter((connection) => ["connected", "degraded"].includes(connection.state));
}

function selectedConnection() {
  return connectedConnections().find((connection) => connection.connection_id === selectedConnectionId) || connectedConnections()[0] || null;
}

function recommendations() {
  return Array.isArray(snapshot?.recommendations) ? snapshot.recommendations : [];
}

function clusters() {
  return Array.isArray(snapshot?.recent_clusters) ? snapshot.recent_clusters : [];
}

function selectedRecommendation() {
  return recommendations().find((item) => item.recommendation_id === selectedRecommendationId) || null;
}

function enabledOperationalDatasets() {
  const connectionId = selectedConnection()?.connection_id;
  return (operationalSnapshot?.datasets || []).filter((dataset) => dataset.connection_id === connectionId && dataset.grant_state === "enabled");
}

function enabledDatasetKeys() {
  return new Set(enabledOperationalDatasets().map((dataset) => dataset.dataset_key));
}

function injectNavigation() {
  const nav = document.querySelector(".primary-nav");
  if (!nav || nav.querySelector('[data-review-intelligence-nav="true"]')) return;
  const button = document.createElement("button");
  button.type = "button";
  button.className = `nav-item review-intelligence-navigation-item ${isPage() ? "active" : ""}`;
  button.dataset.reviewIntelligenceNav = "true";
  button.innerHTML = '<span aria-hidden="true">✦</span><span>Review Intelligence</span>';
  const operational = nav.querySelector('[data-operational-nav="true"]');
  if (operational?.nextSibling) nav.insertBefore(button, operational.nextSibling);
  else nav.prepend(button);
  button.addEventListener("click", () => {
    window.location.hash = `#${PAGE_KEY}`;
    window.setTimeout(() => mount(true), 0);
  });
}

function metric(label, value, detail) {
  return `<article class="review-metric"><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong><small>${escapeHtml(detail)}</small></article>`;
}

function connectionOptions() {
  return connectedConnections().map((connection) => `<option value="${escapeHtml(connection.connection_id)}" ${selectedConnectionId === connection.connection_id ? "selected" : ""}>${escapeHtml(connection.display_name)} · ${escapeHtml(connection.provider_key)}</option>`).join("");
}

function providerModelHelp() {
  const provider = snapshot?.settings?.provider || "disabled";
  if (provider === "openai") return "Only explicitly selected review/message evidence is sent. Responses API requests use store: false. The API key remains in the Windows credential vault.";
  if (provider === "ollama") return "Analysis stays local through the fixed loopback Ollama runtime. Deterministic tracking remains available even when the model is offline.";
  return "The deterministic system still tracks ratings, exact counts, recurring categories, evidence, outcomes, and thresholds without an LLM.";
}

function renderSettings() {
  const settings = snapshot?.settings || {};
  const provider = settings.provider || "disabled";
  const localModels = (modelSnapshot?.models || []).filter((model) => model.installed).map((model) => model.model);
  const modelOptions = localModels.map((model) => `<option value="${escapeHtml(model)}" ${settings.model_name === model ? "selected" : ""}>${escapeHtml(model)}</option>`).join("");
  return `<form id="review-settings-form" class="review-settings-form">
    <div class="review-form-grid">
      <label>Analysis provider<select id="review-provider"><option value="disabled" ${provider === "disabled" ? "selected" : ""}>Deterministic only</option><option value="ollama" ${provider === "ollama" ? "selected" : ""}>Local Ollama</option><option value="openai" ${provider === "openai" ? "selected" : ""}>OpenAI</option></select></label>
      <label>Model<input id="review-model-name" list="review-model-options" maxlength="120" value="${escapeHtml(settings.model_name || "")}" placeholder="${provider === "openai" ? "gpt-5-mini" : "qwen2.5:7b"}"><datalist id="review-model-options">${modelOptions}</datalist></label>
      <label>Minimum matching reviews/messages<input id="review-min-cluster" type="number" min="2" max="100" value="${Number(settings.minimum_cluster_size || 3)}"></label>
      <label>Negative sentiment threshold<input id="review-negative-threshold" type="number" min="-1" max="1" step="0.05" value="${Number(settings.negative_sentiment_threshold ?? -0.25)}"></label>
    </div>
    <div class="review-policy-checks">
      <label><input id="review-auto-processing" type="checkbox" ${settings.automatic_processing ? "checked" : ""}>Process new evidence automatically after synchronization</label>
      <label><input id="review-campaign-drafting" type="checkbox" ${settings.campaign_drafting_enabled !== false ? "checked" : ""}>Allow evidence-backed campaign draft recommendations</label>
      <label><input id="review-campaign-execution" type="checkbox" ${settings.campaign_execution_enabled ? "checked" : ""}>Allow locally approved plans to request provider campaign actions</label>
      <label class="review-remote-context ${provider === "openai" ? "visible" : ""}"><input id="review-remote-context" type="checkbox" ${settings.remote_context_allowed ? "checked" : ""}>I authorize selected review/message context to be sent to the configured OpenAI model</label>
    </div>
    <div class="review-openai-key ${provider === "openai" ? "visible" : ""}"><label>OpenAI API key <small>${settings.openai_key_configured ? "Configured · leave blank to keep current key" : "Required for OpenAI analysis"}</small><input id="review-openai-key" type="password" maxlength="300" autocomplete="new-password" placeholder="${settings.openai_key_configured ? "••••••••••••••••" : "Enter API key"}"></label><label class="review-clear-key"><input id="review-clear-openai-key" type="checkbox">Remove the stored OpenAI API key</label></div>
    <p class="review-model-help">${escapeHtml(providerModelHelp())}</p>
    <div class="review-inline-actions"><button type="submit" class="button primary" ${busy ? "disabled" : ""}>Save Intelligence Settings</button><span>${settings.updated_at_utc ? `Updated ${escapeHtml(formatDate(settings.updated_at_utc))}` : "Local settings not loaded"}</span></div>
  </form>`;
}

function datasetRow([key, label, description]) {
  const enabled = enabledDatasetKeys().has(key);
  return `<article class="review-dataset-row"><div><span class="review-state ${enabled ? "enabled" : "not-authorized"}">${enabled ? "Authorized locally" : "Authorize in Operational Data"}</span><h3>${escapeHtml(label)}</h3><p>${escapeHtml(description)}</p></div><div class="review-dataset-actions"><code>${escapeHtml(key)}</code><button type="button" class="button ${enabled ? "secondary" : "ghost"}" data-review-sync="${escapeHtml(key)}" ${enabled && selectedConnection() && !busy ? "" : "disabled"}>Sync Now</button></div></article>`;
}

function renderClusters() {
  if (!clusters().length) return '<div class="review-empty">No recurring themes have been recorded. Synchronize reviews or messages, then run deterministic or model-assisted analysis.</div>';
  return clusters().map((cluster) => `<article class="review-cluster-card">
    <div class="review-card-head"><span class="review-state ${escapeHtml(statusClass(cluster.source_kind))}">${escapeHtml(humanize(cluster.source_kind))}</span><span>${formatNumber(cluster.observation_count)} observations</span></div>
    <h3>${escapeHtml(cluster.label)}</h3><p>${escapeHtml(cluster.summary)}</p>
    <div class="review-score-row"><span>Average sentiment <strong>${Number(cluster.average_sentiment || 0).toFixed(2)}</strong></span><span>Average rating <strong>${cluster.average_rating == null ? "Not available" : Number(cluster.average_rating).toFixed(1)}</strong></span><span>Confidence <strong>${Math.round(Number(cluster.confidence || 0) * 100)}%</strong></span></div>
    <div class="review-two-column"><div><strong>Likely causes</strong><ul>${(cluster.likely_causes || []).map((item) => `<li>${escapeHtml(item)}</li>`).join("") || "<li>More evidence is needed.</li>"}</ul></div><div><strong>Suggested fixes</strong><ul>${(cluster.suggested_fixes || []).map((item) => `<li>${escapeHtml(item)}</li>`).join("") || "<li>Review supporting evidence.</li>"}</ul></div></div>
    <small>${escapeHtml(formatDate(cluster.created_at_utc))} · ${escapeHtml(humanize(cluster.trend_direction))}</small>
  </article>`).join("");
}

function recommendationActions(item) {
  const campaign = item.campaign_draft;
  return `<div class="review-recommendation-actions">
    <button type="button" class="button ghost" data-review-outcome="accepted" data-review-id="${escapeHtml(item.recommendation_id)}" ${busy ? "disabled" : ""}>Accept</button>
    <button type="button" class="button ghost" data-review-outcome="dismissed" data-review-id="${escapeHtml(item.recommendation_id)}" ${busy ? "disabled" : ""}>Dismiss</button>
    ${campaign ? `<button type="button" class="button primary" data-review-plan="${escapeHtml(item.recommendation_id)}" ${busy ? "disabled" : ""}>Prepare Supervised Campaign Plan</button>` : ""}
  </div>`;
}

function renderRecommendations() {
  if (!recommendations().length) return '<div class="review-empty">No evidence-backed recommendations are available yet.</div>';
  return recommendations().map((item) => `<article class="review-recommendation-card severity-${escapeHtml(item.severity)}">
    <div class="review-card-head"><span class="review-state ${escapeHtml(statusClass(item.state))}">${escapeHtml(humanize(item.state))}</span><span>${escapeHtml(humanize(item.severity))} · ${Math.round(Number(item.confidence || 0) * 100)}% confidence</span></div>
    <h3>${escapeHtml(item.title)}</h3><p>${escapeHtml(item.rationale)}</p>
    <div class="review-recommendation-meta"><span>${escapeHtml(humanize(item.recommendation_type))}</span>${item.campaign_draft ? `<span>Campaign draft · ${escapeHtml(humanize(item.campaign_draft.campaign_type || "customer_refund"))}</span>` : ""}</div>
    ${recommendationActions(item)}
    <small>${escapeHtml(formatDate(item.updated_at_utc))} · evidence remains provider-authoritative</small>
  </article>`).join("");
}

function renderPlanModal() {
  const item = selectedRecommendation();
  if (!item) return "";
  const draft = item.campaign_draft || {};
  return `<div class="review-modal-backdrop" data-review-close-modal><div class="review-modal" role="dialog" aria-modal="true" aria-label="Create supervised campaign plan">
    <div class="review-modal-head"><div><h2>Prepare Supervised Campaign Plan</h2><p>${escapeHtml(item.title)} · this creates a local plan only. It does not publish or send.</p></div><button type="button" class="icon-button" data-review-close-modal>×</button></div>
    <form id="review-plan-form" class="review-plan-form">
      <label>Action type<select id="review-plan-action"><option value="campaign.send_make_good" ${draft.action_type === "campaign.send_make_good" ? "selected" : ""}>Send Make-Good campaign</option><option value="campaign.send_authorized">Send another authorized campaign</option><option value="campaign.draft">Create provider campaign draft</option><option value="campaign.publish">Publish authorized campaign</option><option value="campaign.pause">Pause campaign</option><option value="campaign.resume">Resume campaign</option></select></label>
      <label>Campaign type<input id="review-plan-campaign-type" maxlength="80" value="${escapeHtml(draft.campaign_type || "customer_refund")}" required></label>
      <label>Microgifter campaign ID<input id="review-plan-campaign-id" maxlength="36" placeholder="Campaign UUID" required></label>
      <label>CRM contact ID <small>required for sends</small><input id="review-plan-contact-id" maxlength="36" placeholder="Contact UUID"></label>
      <label>Channel<select id="review-plan-channel"><option value="microgifter_inbox">Microgifter Inbox</option><option value="email">Email</option></select></label>
      <label class="review-plan-message">Message intent<textarea id="review-plan-message" maxlength="1000">${escapeHtml(draft.message_intent || "Acknowledge the issue, apologize, and provide the merchant-authorized recovery offer.")}</textarea></label>
      <div class="review-plan-boundary"><strong>Two authority gates remain</strong><span>The HomeServer plan requires a one-use local approval. Microgifter then enforces its separate merchant campaign authorization, consent, duplicate window, budgets, channels, inventory, dates, and delivery rules.</span></div>
      <div class="review-inline-actions"><button type="button" class="button ghost" data-review-close-modal>Cancel</button><button type="submit" class="button primary" ${busy ? "disabled" : ""}>Create Plan for Approval</button></div>
    </form>
  </div></div>`;
}

function renderPage() {
  const settings = snapshot?.settings || {};
  const connection = selectedConnection();
  return `<div class="review-intelligence-page" data-review-intelligence-mounted="true">
    <header class="review-intelligence-header"><div><h1>Review Intelligence</h1><p>Track exact operational evidence deterministically, use an optional selected LLM to understand recurring context, and convert recommendations into supervised merchant-authorized actions.</p></div><div class="review-header-actions"><span class="review-state enabled">Deterministic core active</span><button type="button" class="button secondary" id="review-refresh" ${loading || busy ? "disabled" : ""}>Refresh</button></div></header>
    ${notice ? `<div class="review-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}
    <section class="review-boundary-grid"><article><strong>System tracks facts</strong><span>Exact records, ratings, counts, thresholds, permissions, budgets, outcomes, and receipts remain deterministic.</span></article><article><strong>LLM understands meaning</strong><span>Ollama or OpenAI can refine sentiment, semantic themes, likely causes, fixes, and drafts using bounded evidence.</span></article><article><strong>Provider controls action</strong><span>Microgifter remains authoritative and independently enforces campaign authorization, consent, inventory, delivery, and duplicate prevention.</span></article></section>
    <section class="review-metrics">${metric("Observations", formatNumber(snapshot?.observation_count), "Reviews and messages processed")}${metric("Recurring themes", formatNumber(clusters().length), "Deterministic and model-refined")}${metric("Recommendations", formatNumber(recommendations().length), "Evidence-backed next steps")}${metric("Completed runs", formatNumber(snapshot?.completed_runs), "Audited analysis receipts")}${metric("Provider sync receipts", formatNumber(snapshot?.provider_sync_receipts), "Signed operational imports")}${metric("Campaign receipts", formatNumber(snapshot?.campaign_action_receipts), "Provider action outcomes")}</section>
    <section class="review-panel"><div class="review-panel-head"><div><h2>Intelligence Model</h2><p>The selected model is optional. Disabling it leaves deterministic tracking and threshold analysis available.</p></div><span>${escapeHtml(humanize(settings.provider || "disabled"))}</span></div>${renderSettings()}</section>
    <section class="review-layout"><main class="review-panel"><div class="review-panel-head"><div><h2>Operational Evidence</h2><p>Synchronize only datasets already authorized both in Microgifter and in the local Operational Data page.</p></div><label class="review-connection-select">Connection<select id="review-connection">${connectionOptions()}</select></label></div>${connection ? `<div class="review-dataset-list">${REVIEW_DATASETS.map(datasetRow).join("")}</div>` : '<div class="review-empty">Pair an active Microgifter connection before synchronizing evidence.</div>'}<div class="review-analysis-actions"><button type="button" class="button secondary" id="review-run-deterministic" ${connection && !busy ? "" : "disabled"}>Run Deterministic Analysis</button><button type="button" class="button primary" id="review-run-model" ${connection && settings.provider !== "disabled" && !busy ? "" : "disabled"}>Run ${settings.provider === "openai" ? "OpenAI" : "Local Model"} Analysis</button><span>Latest run: ${lastAnalysis ? `${escapeHtml(humanize(lastAnalysis.provider))} · ${formatNumber(lastAnalysis.records_considered)} records` : "Not run in this session"}</span></div></main>
    <aside class="review-panel review-process-panel"><div class="review-panel-head"><div><h2>Processing Contract</h2><p>Every stage keeps evidence and authority visible.</p></div></div><ol><li><strong>Import</strong><span>Verify signed provider scope, cursor, revisions, hashes, and provenance.</span></li><li><strong>Observe</strong><span>Extract ratings, text, deterministic sentiment, categories, entities, and commitments.</span></li><li><strong>Cluster</strong><span>Group repeated operational context; the selected LLM may refine semantic themes.</span></li><li><strong>Recommend</strong><span>Separate observed facts, repeated patterns, likely causes, fixes, confidence, and evidence.</span></li><li><strong>Act</strong><span>Create a supervised plan; HomeServer and Microgifter enforce separate policy gates.</span></li><li><strong>Measure</strong><span>Record implementation and compare later reviews and operational results.</span></li></ol></aside></section>
    <section class="review-results-layout"><main class="review-panel"><div class="review-panel-head"><div><h2>Recurring Themes</h2><p>Similar review and message context is grouped even when customers use different wording.</p></div><span>${clusters().length} clusters</span></div><div class="review-cluster-list">${renderClusters()}</div></main><aside class="review-panel"><div class="review-panel-head"><div><h2>Recommendations</h2><p>Suggested fixes and campaign drafts remain evidence-backed proposals until authorized.</p></div><span>${recommendations().length} items</span></div><div class="review-recommendation-list">${renderRecommendations()}</div></aside></section>
    ${renderPlanModal()}
  </div>`;
}

function mount(force = false) {
  injectNavigation();
  document.querySelector('[data-review-intelligence-nav="true"]')?.classList.toggle("active", isPage());
  if (!isPage()) return;
  const canvas = document.querySelector(".page-canvas");
  if (!canvas) return;
  if (!force && canvas.querySelector('[data-review-intelligence-mounted="true"]')) return;
  canvas.innerHTML = loading && !snapshot ? '<div class="review-intelligence-page"><div class="review-empty">Loading review intelligence, operational evidence, models, and connections…</div></div>' : renderPage();
  bindEvents();
  if (!snapshot && !loading) void refresh();
}

function bindEvents() {
  document.querySelector("#review-refresh")?.addEventListener("click", refresh);
  document.querySelector("#review-settings-form")?.addEventListener("submit", saveSettings);
  document.querySelector("#review-provider")?.addEventListener("change", () => mount(true));
  document.querySelector("#review-connection")?.addEventListener("change", (event) => {
    selectedConnectionId = event.target.value;
    mount(true);
  });
  document.querySelectorAll("[data-review-sync]").forEach((button) => button.addEventListener("click", () => syncDataset(button.dataset.reviewSync)));
  document.querySelector("#review-run-deterministic")?.addEventListener("click", () => runAnalysis(false));
  document.querySelector("#review-run-model")?.addEventListener("click", () => runAnalysis(true));
  document.querySelectorAll("[data-review-outcome]").forEach((button) => button.addEventListener("click", () => recordOutcome(button.dataset.reviewId, button.dataset.reviewOutcome)));
  document.querySelectorAll("[data-review-plan]").forEach((button) => button.addEventListener("click", () => {
    selectedRecommendationId = button.dataset.reviewPlan;
    mount(true);
  }));
  document.querySelector("#review-plan-form")?.addEventListener("submit", createCampaignPlan);
  document.querySelectorAll("[data-review-close-modal]").forEach((element) => element.addEventListener("click", (event) => {
    if (element.classList.contains("review-modal-backdrop") && event.target !== element) return;
    selectedRecommendationId = null;
    mount(true);
  }));
}

async function refresh() {
  loading = true;
  mount(true);
  try {
    [snapshot, connectionsSnapshot, operationalSnapshot, modelSnapshot] = await Promise.all([
      invoke("homeserver_review_intelligence"),
      invoke("homeserver_cloud_connections"),
      invoke("homeserver_operational_data"),
      invoke("homeserver_models"),
    ]);
    if (!selectedConnectionId || !connectedConnections().some((connection) => connection.connection_id === selectedConnectionId)) {
      selectedConnectionId = connectedConnections()[0]?.connection_id || "";
    }
    notice = { kind: "success", message: "Review intelligence, models, operational grants, and provider connections refreshed." };
  } catch (error) {
    notice = { kind: "warning", message: `Review Intelligence unavailable: ${String(error)}` };
  } finally {
    loading = false;
    mount(true);
  }
}

async function saveSettings(event) {
  event.preventDefault();
  const provider = document.querySelector("#review-provider")?.value || "disabled";
  const remoteAllowed = Boolean(document.querySelector("#review-remote-context")?.checked);
  if (provider === "openai" && !remoteAllowed) {
    notice = { kind: "warning", message: "OpenAI analysis requires explicit permission to send the selected review/message context." };
    mount(true);
    return;
  }
  busy = true;
  mount(true);
  try {
    snapshot.settings = await invoke("homeserver_update_review_intelligence_settings", { request: {
      provider,
      model_name: document.querySelector("#review-model-name")?.value.trim() || null,
      remote_context_allowed: remoteAllowed,
      automatic_processing: Boolean(document.querySelector("#review-auto-processing")?.checked),
      minimum_cluster_size: Number(document.querySelector("#review-min-cluster")?.value || 3),
      negative_sentiment_threshold: Number(document.querySelector("#review-negative-threshold")?.value || -0.25),
      campaign_drafting_enabled: Boolean(document.querySelector("#review-campaign-drafting")?.checked),
      campaign_execution_enabled: Boolean(document.querySelector("#review-campaign-execution")?.checked),
      openai_api_key: document.querySelector("#review-openai-key")?.value.trim() || null,
      clear_openai_api_key: Boolean(document.querySelector("#review-clear-openai-key")?.checked),
    }});
    notice = { kind: "success", message: "Review intelligence settings and credential policy saved locally." };
  } catch (error) {
    notice = { kind: "warning", message: `Settings were not saved: ${String(error)}` };
  } finally {
    busy = false;
    mount(true);
  }
}

async function syncDataset(datasetKey) {
  const connection = selectedConnection();
  if (!connection) return;
  busy = true;
  notice = { kind: "neutral", message: `Synchronizing ${datasetKey} through the signed Microgifter provider adapter…` };
  mount(true);
  try {
    const result = await invoke("homeserver_sync_review_dataset", { request: {
      connection_id: connection.connection_id,
      dataset_key: datasetKey,
      import_mode: "incremental",
      limit: 100,
    }});
    notice = { kind: "success", message: `${datasetKey}: ${result.state}, ${Number(result.records_received || 0)} records and ${Number(result.events_received || 0)} events received.` };
    operationalSnapshot = await invoke("homeserver_operational_data");
  } catch (error) {
    notice = { kind: "warning", message: `${datasetKey} synchronization failed: ${String(error)}` };
  } finally {
    busy = false;
    mount(true);
  }
}

async function runAnalysis(useLlm) {
  const connection = selectedConnection();
  if (!connection) return;
  busy = true;
  notice = { kind: "neutral", message: useLlm ? "Running deterministic processing and selected-model semantic refinement…" : "Running deterministic review and message processing…" };
  mount(true);
  try {
    lastAnalysis = await invoke("homeserver_run_review_analysis", { request: {
      connection_id: connection.connection_id,
      dataset_keys: REVIEW_DATASETS.map(([key]) => key).filter((key) => enabledDatasetKeys().has(key)),
      use_llm: useLlm,
      maximum_records: 200,
    }});
    snapshot = await invoke("homeserver_review_intelligence");
    notice = { kind: "success", message: `${humanize(lastAnalysis.provider)} analysis processed ${Number(lastAnalysis.records_considered || 0)} evidence records and produced ${Number(lastAnalysis.clusters_created || 0)} clusters with ${Number(lastAnalysis.recommendations_created || 0)} recommendations.` };
  } catch (error) {
    notice = { kind: "warning", message: `Analysis failed: ${String(error)}` };
  } finally {
    busy = false;
    mount(true);
  }
}

async function recordOutcome(recommendationId, state) {
  busy = true;
  mount(true);
  try {
    await invoke("homeserver_record_review_recommendation_outcome", { request: {
      recommendation_id: recommendationId,
      state,
      note: null,
      evidence: { source: "local_control_center" },
    }});
    snapshot = await invoke("homeserver_review_intelligence");
    notice = { kind: "success", message: `Recommendation marked ${humanize(state)}.` };
  } catch (error) {
    notice = { kind: "warning", message: `Recommendation outcome failed: ${String(error)}` };
  } finally {
    busy = false;
    mount(true);
  }
}

async function createCampaignPlan(event) {
  event.preventDefault();
  const item = selectedRecommendation();
  const connection = selectedConnection();
  if (!item || !connection) return;
  const actionType = document.querySelector("#review-plan-action")?.value || "campaign.send_make_good";
  const campaignId = document.querySelector("#review-plan-campaign-id")?.value.trim() || "";
  const contactId = document.querySelector("#review-plan-contact-id")?.value.trim() || "";
  if (actionType.includes("send") && !contactId) {
    notice = { kind: "warning", message: "A Microgifter CRM contact ID is required for campaign sends." };
    mount(true);
    return;
  }
  busy = true;
  mount(true);
  try {
    const plan = await invoke("homeserver_create_agent_plan", { request: {
      thread_id: null,
      title: `${item.title} · ${humanize(actionType)}`,
      rationale: `${item.rationale} This action is based on recommendation ${item.recommendation_id} and remains subject to one-use local approval plus Microgifter merchant policy.`,
      action_type: actionType,
      arguments: {
        recommendation_id: item.recommendation_id,
        campaign_type: document.querySelector("#review-plan-campaign-type")?.value.trim() || "customer_refund",
        campaign_id: campaignId,
        contact_id: contactId || null,
        channel: document.querySelector("#review-plan-channel")?.value || "microgifter_inbox",
        message: document.querySelector("#review-plan-message")?.value.trim() || "",
        evidence: item.evidence || {},
      },
      connection_id: connection.connection_id,
      goal_id: null,
      dataset_keys: REVIEW_DATASETS.map(([key]) => key).filter((key) => enabledDatasetKeys().has(key)),
      expires_minutes: 60,
    }});
    selectedRecommendationId = null;
    notice = { kind: "success", message: `Supervised plan ${plan.plan_id} created. Review and approve it in Agent Workspace; no campaign was sent.` };
  } catch (error) {
    notice = { kind: "warning", message: `Campaign plan was not created: ${String(error)}` };
  } finally {
    busy = false;
    mount(true);
  }
}

window.addEventListener("hashchange", () => mount(true));
window.addEventListener("homeserver:rendered", () => mount());

if (!initialized) {
  initialized = true;
  const observer = new MutationObserver(() => mount());
  observer.observe(document.documentElement, { childList: true, subtree: true });
  window.setTimeout(() => mount(), 0);
}
