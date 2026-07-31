import { invoke } from "@tauri-apps/api/core";
import "./homeserver-agent-chat.css";

window.__HOMESERVER_AGENT_CHAT_V1__ = true;

const PAGE_KEY = "agent";
const DEFAULT_PROVIDER_URL = "https://microgifter.com";

let workspace = null;
let provider = null;
let activeThreadId = null;
let loading = false;
let sending = false;
let connectionBusy = false;
let connectionDrawerOpen = false;
let connectFormOpen = false;
let historyQuery = "";
let notice = null;
let initialized = false;
let refreshGeneration = 0;
let mountScheduled = false;
let shellHealth = { service: "online", models: "unknown" };

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

function compactId(value) {
  const text = String(value || "");
  if (!text) return "Not assigned";
  if (text.length <= 18) return text;
  return `${text.slice(0, 8)}…${text.slice(-7)}`;
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
  if (seconds < 604800) return `${Math.floor(seconds / 86400)}d ago`;
  return new Date(value).toLocaleDateString();
}

function currentHash() {
  return window.location.hash.replace("#", "");
}

function isAgentPage() {
  return currentHash() === PAGE_KEY;
}

function threads() {
  return Array.isArray(workspace?.threads) ? workspace.threads : [];
}

function filteredThreads() {
  const query = historyQuery.trim().toLowerCase();
  if (!query) return threads();
  return threads().filter((thread) => String(thread.title || "").toLowerCase().includes(query));
}

function activeThread() {
  return threads().find((thread) => thread.thread_id === activeThreadId) || null;
}

function messages() {
  if (!activeThreadId) return [];
  return (workspace?.messages || []).filter((message) => message.thread_id === activeThreadId);
}

function goals() {
  return Array.isArray(workspace?.goals) ? workspace.goals.filter((goal) => goal.state === "active") : [];
}

function providerConnections() {
  return Array.isArray(provider?.connections) ? provider.connections : [];
}

function ensureActiveThread() {
  if (activeThreadId && threads().some((thread) => thread.thread_id === activeThreadId)) return;
  activeThreadId = threads()[0]?.thread_id || null;
}

function providerTone(state) {
  const normalized = String(state || "unpaired");
  if (["active"].includes(normalized)) return "good";
  if (["offline", "grace", "pairing_pending", "replacing"].includes(normalized)) return "warn";
  if (["suspended", "revoked", "error"].includes(normalized)) return "bad";
  return "neutral";
}

function overallProviderState() {
  const connections = providerConnections();
  if (!connections.length) return "unpaired";
  const priority = ["error", "revoked", "suspended", "offline", "grace", "replacing", "pairing_pending", "active"];
  return priority.find((state) => connections.some((connection) => connection.lifecycle_state === state)) || "active";
}

function renderThreadList() {
  const items = filteredThreads();
  if (!items.length) {
    return `<div class="hs-chat-history-empty"><strong>${historyQuery ? "No matching chats" : "No chats yet"}</strong><span>${historyQuery ? "Try another search." : "Start a new conversation with your private HomeServer agent."}</span></div>`;
  }
  return items.map((thread) => {
    const active = thread.thread_id === activeThreadId;
    return `<button type="button" class="hs-chat-thread ${active ? "active" : ""}" data-chat-thread="${escapeHtml(thread.thread_id)}">
      <span class="hs-chat-thread-icon">✦</span>
      <span class="hs-chat-thread-copy"><strong>${escapeHtml(thread.title || "HomeServer chat")}</strong><small>${escapeHtml(relativeDate(thread.updated_at_utc))}</small></span>
    </button>`;
  }).join("");
}

function renderWelcome() {
  const local = provider?.local_operation_available !== false;
  return `<div class="hs-chat-welcome">
    <div class="hs-chat-welcome-mark">✦</div>
    <h1>What can your HomeServer help with?</h1>
    <p>Ask about local knowledge, connected sites, operational evidence, goals, approved actions, models, backups, or system health.</p>
    <div class="hs-chat-suggestions">
      <button type="button" data-chat-suggestion="Summarize my connected sites and their current status.">Summarize connected sites</button>
      <button type="button" data-chat-suggestion="Review current HomeServer health and identify anything that needs attention.">Review system health</button>
      <button type="button" data-chat-suggestion="Use my local Knowledge Vault and goals to suggest the next best action.">Suggest the next action</button>
      <button type="button" data-chat-suggestion="Show what Microgifter capabilities and datasets are currently authorized.">Review authorized capabilities</button>
    </div>
    <div class="hs-chat-local-boundary ${local ? "good" : "warn"}"><span></span>${local ? "Local operation is available even when cloud connections are offline." : "Local operation status needs attention."}</div>
  </div>`;
}

function renderMessages() {
  const items = messages();
  if (!items.length) return renderWelcome();
  return items.map((message) => {
    const role = message.role === "user" ? "user" : "assistant";
    return `<article class="hs-chat-message ${role}">
      <div class="hs-chat-avatar">${role === "user" ? "YOU" : "HS"}</div>
      <div class="hs-chat-message-wrap">
        <div class="hs-chat-message-meta"><strong>${role === "user" ? "You" : "HomeServer"}</strong><span>${escapeHtml(humanize(message.mode || "ask"))}</span><time>${escapeHtml(formatDate(message.created_at_utc))}</time></div>
        <div class="hs-chat-message-body">${escapeHtml(message.content)}</div>
      </div>
    </article>`;
  }).join("");
}

function goalOptions() {
  return `<option value="">No goal selected</option>${goals().map((goal) => `<option value="${escapeHtml(goal.goal_id)}">${escapeHtml(goal.title)}</option>`).join("")}`;
}

function modelOptions() {
  const model = workspace?.default_chat_model;
  return `<option value="">${model ? `Automatic · ${escapeHtml(model)}` : "Automatic model routing"}</option>`;
}

function integrationSnapshot() {
  return workspace?.integrations || null;
}

function renderAgentGuidance() {
  const guidance = integrationSnapshot()?.active_prompt;
  if (!guidance) return "";
  return `<section class="hs-agent-guidance" data-guidance-key="${escapeHtml(guidance.key)}">
    <div><span>HomeServer guidance</span><strong>${escapeHtml(guidance.title)}</strong><p>${escapeHtml(guidance.message)}</p></div>
    <div class="hs-agent-guidance-actions"><button type="button" data-guidance-action="${escapeHtml(guidance.action_target)}">${escapeHtml(guidance.action_label)}</button><button type="button" class="quiet" data-dismiss-guidance="${escapeHtml(guidance.key)}">Dismiss</button></div>
  </section>`;
}

function renderMcpIntegrationPanel() {
  const clouds = Array.isArray(workspace?.connections) ? workspace.connections : [];
  const integrations = integrationSnapshot()?.site_integrations || [];
  const rows = clouds.map((connection) => {
    const integration = integrations.find((item) => item.connection_id === connection.connection_id);
    if (!integration) {
      return `<form class="hs-mcp-config" data-mcp-config="${escapeHtml(connection.connection_id)}">
        <div><strong>${escapeHtml(connection.display_name || "Microgifter")}</strong><span>Paired for sync · authorize live MCP tools</span></div>
        <input name="client_id" required maxlength="240" placeholder="Pre-registered MCP OAuth client ID">
        <button type="submit">Configure MCP</button>
      </form>`;
    }
    return `<article class="hs-mcp-integration">
      <div><strong>${escapeHtml(connection.display_name || "Microgifter")}</strong><span>${escapeHtml(humanize(integration.state))} · ${integration.tools.length} tools</span></div>
      <div>${integration.state === "connected" ? `<button type="button" data-mcp-refresh="${escapeHtml(connection.connection_id)}">Refresh tools</button>` : `<button type="button" data-mcp-authorize="${escapeHtml(connection.connection_id)}">Authorize MCP</button>`}</div>
      ${integration.last_error ? `<small>${escapeHtml(integration.last_error)}</small>` : ""}
    </article>`;
  }).join("");
  return `<section class="hs-provider-mcp"><div class="hs-provider-section-head"><h3>Live site tools</h3><span>Read tools can run automatically. Drafts and actions remain governed.</span></div>${rows || '<p>Pair a cloud connection before configuring MCP.</p>'}</section>`;
}

function renderComposer() {
  return `<form class="hs-chat-composer" id="homeserver-chat-form">
    <div class="hs-chat-composer-tools">
      <select id="hs-chat-mode" aria-label="Agent mode"><option value="ask">Ask</option><option value="analyze">Analyze</option><option value="plan">Plan</option><option value="dispatch">Dispatch draft</option><option value="execute">Execution request</option></select>
      <select id="hs-chat-goal" aria-label="Goal">${goalOptions()}</select>
      <select id="hs-chat-model" aria-label="Model">${modelOptions()}</select>
      <button type="button" class="hs-chat-tool-button" id="hs-chat-connection-toggle">Connections <span>${providerConnections().length}</span></button>
    </div>
    <div class="hs-chat-input-shell">
      <textarea id="hs-chat-input" maxlength="4000" rows="1" required placeholder="Message your HomeServer agent…" aria-label="Message HomeServer"></textarea>
      <button class="hs-chat-send" type="submit" aria-label="Send message" ${sending ? "disabled" : ""}>${sending ? "…" : "↑"}</button>
    </div>
    <div class="hs-chat-composer-footer">
      <div class="hs-chat-context-chips">
        <label><input type="checkbox" name="hs-chat-context" value="system" checked>System</label>
        <label><input type="checkbox" name="hs-chat-context" value="connections" checked>Connections</label>
        <label><input type="checkbox" name="hs-chat-context" value="knowledge" checked>Knowledge</label>
        <label><input type="checkbox" name="hs-chat-context" value="goals" checked>Goals</label>
        <label><input type="checkbox" name="hs-chat-context" value="operational_data" checked>Operational data</label>
      </div>
      <small>External actions still require a separate local approval.</small>
    </div>
  </form>`;
}

function renderConnectionCard(connection) {
  const tone = providerTone(connection.lifecycle_state);
  const capabilities = Array.isArray(connection.granted_capabilities) ? connection.granted_capabilities.filter((item) => item.grant_state === "granted") : [];
  return `<article class="hs-provider-card">
    <div class="hs-provider-card-head">
      <div><span class="hs-provider-state ${tone}"><i></i>${escapeHtml(humanize(connection.lifecycle_state))}</span><h3>${escapeHtml(connection.device_display_name || "Microgifter HomeServer")}</h3><p>${escapeHtml(compactId(connection.device_id))}</p></div>
      <span class="hs-provider-health">${escapeHtml(humanize(connection.health_category || "unknown"))}</span>
    </div>
    <dl>
      <div><dt>Subscription</dt><dd>${escapeHtml(humanize(connection.subscription_state || "unknown"))}</dd></div>
      <div><dt>Lease expires</dt><dd>${escapeHtml(formatDate(connection.entitlement_expires_at_utc))}</dd></div>
      <div><dt>Merchants</dt><dd>${Number(connection.assigned_merchant_count || 0)}</dd></div>
      <div><dt>Sites</dt><dd>${Number(connection.assigned_site_count || 0)}</dd></div>
      <div><dt>Capabilities</dt><dd>${capabilities.length}</dd></div>
      <div><dt>Last heartbeat</dt><dd>${escapeHtml(relativeDate(connection.last_heartbeat_at_utc))}</dd></div>
    </dl>
    <div class="hs-provider-actions">
      <button type="button" data-provider-action="heartbeat" data-connection-id="${escapeHtml(connection.connection_id)}" ${connectionBusy ? "disabled" : ""}>Heartbeat</button>
      <button type="button" data-provider-action="refresh" data-connection-id="${escapeHtml(connection.connection_id)}" ${connectionBusy ? "disabled" : ""}>Refresh lease</button>
      <button type="button" data-provider-action="rotate" data-connection-id="${escapeHtml(connection.connection_id)}" ${connectionBusy ? "disabled" : ""}>Rotate credential</button>
    </div>
  </article>`;
}

function renderConnectForm() {
  if (!connectFormOpen) return "";
  return `<form class="hs-provider-connect" id="hs-provider-connect-form">
    <div class="hs-provider-connect-head"><div><strong>Connect Microgifter</strong><span>Enter the one-time Microgifter Sync Code. It is exchanged once and never retained.</span></div><button type="button" data-close-connect>×</button></div>
    <label><span>Microgifter Sync Code</span><input id="hs-sync-code" type="password" autocomplete="one-time-code" minlength="6" maxlength="160" required placeholder="Enter Sync Code"></label>
    <label><span>Device name</span><input id="hs-device-name" minlength="1" maxlength="120" required value="Office HomeServer"></label>
    <label><span>Provider URL</span><input id="hs-provider-url" type="url" maxlength="300" required value="${DEFAULT_PROVIDER_URL}"></label>
    <div class="hs-provider-form-grid"><label><span>Merchant ID <small>optional</small></span><input id="hs-merchant-id" maxlength="160"></label><label><span>Site ID <small>optional</small></span><input id="hs-site-id" maxlength="160"></label></div>
    <label class="hs-provider-check"><input id="hs-provider-default" type="checkbox" checked><span>Make this the default Microgifter connection</span></label>
    <button class="button primary" type="submit" ${connectionBusy ? "disabled" : ""}>Connect HomeServer</button>
  </form>`;
}

function renderConnectionDrawer() {
  if (!connectionDrawerOpen) return "";
  const connections = providerConnections();
  const state = overallProviderState();
  return `<div class="hs-provider-backdrop" data-close-provider><aside class="hs-provider-drawer" role="dialog" aria-modal="true" aria-label="HomeServer connections">
    <header><div><span class="hs-provider-state ${providerTone(state)}"><i></i>${escapeHtml(humanize(state))}</span><h2>HomeServer Connections</h2><p>Manage Microgifter connection state without changing the existing pairing node.</p></div><button type="button" data-close-provider>×</button></header>
    ${notice ? `<div class="hs-chat-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}
    <div class="hs-provider-boundary"><strong>Local-first boundary</strong><span>HomeServer local data, models, agents, Knowledge Vault, backups, and other wrappers remain available independently.</span></div>
    <div class="hs-provider-toolbar"><button type="button" class="button primary" id="hs-provider-open-connect">Connect Microgifter</button><button type="button" class="button secondary" id="hs-provider-refresh" ${connectionBusy ? "disabled" : ""}>Refresh</button></div>
    ${renderConnectForm()}
    <div class="hs-provider-list">${connections.length ? connections.map(renderConnectionCard).join("") : '<div class="hs-provider-empty"><strong>No Microgifter connection</strong><span>Generate a Sync Code from the Microgifter account panel, then connect it here.</span></div>'}</div>
    ${renderMcpIntegrationPanel()}
    <section class="hs-provider-receipts"><h3>Recent connection activity</h3>${(provider?.recent_receipts || []).slice(0, 8).map((receipt) => `<div><span class="hs-provider-result ${escapeHtml(receipt.result_category)}"></span><strong>${escapeHtml(humanize(receipt.event_type))}</strong><small>${escapeHtml(relativeDate(receipt.created_at_utc))}</small></div>`).join("") || '<p>No Phase 6A connection receipts yet.</p>'}</section>
  </aside></div>`;
}

function renderPage() {
  const thread = activeThread();
  const state = overallProviderState();
  return `<div class="hs-chat-page" data-homeserver-chat-mounted="true">
    <aside class="hs-chat-sidebar">
      <button class="hs-chat-sidebar-brand" id="hs-chat-logo-home" type="button" aria-label="Back to dashboard" title="Back to dashboard"><span>✦</span><div><strong>HomeServer</strong><small>Private Agent</small></div></button>
      <button class="hs-chat-new" id="hs-chat-new" type="button"><span>＋</span>New chat</button>
      <label class="hs-chat-history-search"><span>⌕</span><input id="hs-chat-history-search" type="search" value="${escapeHtml(historyQuery)}" placeholder="Search chats" aria-label="Search chats"></label>
      <div class="hs-chat-history-label"><span>Chats</span><small>${threads().length}</small></div>
      <div class="hs-chat-history">${renderThreadList()}</div>
      <button type="button" class="hs-chat-provider-summary" id="hs-chat-provider-summary"><span class="hs-provider-state ${providerTone(state)}"><i></i>${escapeHtml(humanize(state))}</span><strong>Microgifter</strong><small>${providerConnections().length} connection${providerConnections().length === 1 ? "" : "s"}</small></button>
      <div class="hs-chat-sidebar-footer">
        <button type="button" id="hs-chat-control-center"><span>←</span><div><strong>Control Center</strong><small>Return to dashboard</small></div></button>
      </div>
    </aside>
    <main class="hs-chat-main">
      <header class="hs-chat-header"><div><strong>${escapeHtml(thread?.title || "New chat")}</strong><span>${thread ? `Updated ${escapeHtml(relativeDate(thread.updated_at_utc))}` : "Private local conversation"}</span></div><div class="hs-chat-header-actions"><span class="hs-runtime-state ${shellHealth.models === "degraded" ? "warn" : ""}">${escapeHtml(shellHealth.models === "degraded" ? "Model runtime offline" : humanize(workspace?.model_runtime_state || "loading"))}</span><button type="button" id="hs-chat-refresh" title="Refresh Agent Chat" ${loading ? "disabled" : ""}>↻</button><button type="button" id="hs-chat-open-connections">Connections</button></div></header>
      ${notice && !connectionDrawerOpen ? `<div class="hs-chat-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}
      ${renderAgentGuidance()}
      <section class="hs-chat-stream" id="hs-chat-stream">${loading && !workspace ? '<div class="hs-chat-loading">Loading your private HomeServer chats…</div>' : renderMessages()}</section>
      ${renderComposer()}
    </main>
    ${renderConnectionDrawer()}
  </div>`;
}

function mount(force = false) {
  if (!isAgentPage()) return;
  const host = document.querySelector('[data-homeserver-agent-host="true"]');
  if (!host) return;
  const initialLoad = !initialized && !loading;
  if (initialLoad) {
    initialized = true;
    loading = true;
  }
  if (!force && !initialLoad && host.querySelector('[data-homeserver-chat-mounted="true"]')) return;
  host.innerHTML = renderPage();
  bindEvents();
  if (initialLoad) void refreshAll({ initial: true });
  window.setTimeout(() => {
    const stream = document.querySelector("#hs-chat-stream");
    if (stream) stream.scrollTop = stream.scrollHeight;
    autoSizeComposer();
  }, 0);
}

function scheduleMount(force = false) {
  if (mountScheduled) return;
  mountScheduled = true;
  window.requestAnimationFrame(() => {
    mountScheduled = false;
    mount(force);
  });
}

function bindEvents() {
  document.querySelectorAll("#hs-chat-logo-home,#hs-chat-control-center").forEach((button) => {
    button.addEventListener("click", () => { window.location.hash = "#dashboard"; });
  });
  document.querySelector("#hs-chat-new")?.addEventListener("click", startNewChat);
  document.querySelector("#hs-chat-refresh")?.addEventListener("click", refreshAll);
  document.querySelector("#homeserver-chat-form")?.addEventListener("submit", submitPrompt);
  document.querySelector("#hs-chat-input")?.addEventListener("input", autoSizeComposer);
  document.querySelector("#hs-chat-input")?.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      document.querySelector("#homeserver-chat-form")?.requestSubmit();
    }
  });
  document.querySelector("#hs-chat-history-search")?.addEventListener("input", (event) => {
    historyQuery = event.currentTarget.value;
    mount(true);
    const input = document.querySelector("#hs-chat-history-search");
    if (input) {
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    }
  });
  document.querySelectorAll("[data-chat-thread]").forEach((button) => button.addEventListener("click", () => {
    activeThreadId = button.dataset.chatThread || null;
    notice = null;
    mount(true);
  }));
  document.querySelectorAll("[data-chat-suggestion]").forEach((button) => button.addEventListener("click", () => {
    const input = document.querySelector("#hs-chat-input");
    if (!input) return;
    input.value = button.dataset.chatSuggestion || "";
    input.focus();
    autoSizeComposer();
  }));
  document.querySelectorAll("#hs-chat-open-connections,#hs-chat-provider-summary,#hs-chat-connection-toggle").forEach((button) => button?.addEventListener("click", () => {
    connectionDrawerOpen = true;
    notice = null;
    mount(true);
  }));
  document.querySelectorAll("[data-close-provider]").forEach((element) => element.addEventListener("click", (event) => {
    if (element.classList.contains("hs-provider-backdrop") && event.target !== element) return;
    connectionDrawerOpen = false;
    connectFormOpen = false;
    mount(true);
  }));
  document.querySelector("#hs-provider-open-connect")?.addEventListener("click", () => {
    connectFormOpen = true;
    mount(true);
  });
  document.querySelectorAll("[data-close-connect]").forEach((button) => button.addEventListener("click", () => {
    connectFormOpen = false;
    mount(true);
  }));
  document.querySelector("#hs-provider-refresh")?.addEventListener("click", refreshProvider);
  document.querySelector("#hs-provider-connect-form")?.addEventListener("submit", connectMicrogifter);
  document.querySelectorAll("[data-provider-action]").forEach((button) => button.addEventListener("click", runProviderAction));
  document.querySelectorAll("[data-mcp-config]").forEach((form) => form.addEventListener("submit", configureMcp));
  document.querySelectorAll("[data-mcp-authorize]").forEach((button) => button.addEventListener("click", authorizeMcp));
  document.querySelectorAll("[data-mcp-refresh]").forEach((button) => button.addEventListener("click", refreshMcpTools));
  document.querySelectorAll("[data-guidance-action]").forEach((button) => button.addEventListener("click", runGuidanceAction));
  document.querySelectorAll("[data-dismiss-guidance]").forEach((button) => button.addEventListener("click", dismissGuidance));
}

function autoSizeComposer() {
  const input = document.querySelector("#hs-chat-input");
  if (!input) return;
  input.style.height = "auto";
  input.style.height = `${Math.min(180, Math.max(48, input.scrollHeight))}px`;
}

function startNewChat() {
  activeThreadId = null;
  notice = { kind: "info", message: "A new private chat will be created with your first message." };
  mount(true);
  document.querySelector("#hs-chat-input")?.focus();
}

async function refreshAll(options = {}) {
  const initial = options?.initial === true;
  const generation = ++refreshGeneration;
  loading = true;
  notice = null;
  if (!initial) mount(true);
  try {
    const [nextWorkspace, nextProvider] = await Promise.all([
      invoke("homeserver_agent_workspace"),
      invoke("homeserver_microgifter_status").catch(() => null),
    ]);
    if (generation !== refreshGeneration) return;
    workspace = nextWorkspace;
    if (nextProvider) provider = nextProvider;
    ensureActiveThread();
  } catch (error) {
    if (generation !== refreshGeneration) return;
    notice = { kind: "warning", message: `HomeServer Agent is unavailable: ${String(error)}` };
  } finally {
    if (generation === refreshGeneration) {
      loading = false;
      mount(true);
    }
  }
}

async function refreshProvider() {
  connectionBusy = true;
  notice = null;
  mount(true);
  try {
    provider = await invoke("homeserver_microgifter_status");
    notice = { kind: "success", message: "Microgifter connection status refreshed." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    connectionBusy = false;
    mount(true);
  }
}

function selectedContext() {
  return [...document.querySelectorAll('input[name="hs-chat-context"]:checked')].map((input) => input.value);
}

function selectedDatasetKeys(context) {
  const keys = new Set(context.filter((value) => !["connections", "operational_data"].includes(value)));
  if (context.includes("operational_data")) {
    (workspace?.data_sources || [])
      .filter((source) => String(source.key || "").startsWith("dataset:") && !["planned_phase_5c", "paused", "not_granted"].includes(source.state))
      .forEach((source) => keys.add(source.key));
  }
  return [...keys];
}

async function submitPrompt(event) {
  event.preventDefault();
  const input = document.querySelector("#hs-chat-input");
  const prompt = input?.value?.trim() || "";
  if (!prompt || sending) return;
  const goalId = document.querySelector("#hs-chat-goal")?.value || "";
  const context = selectedContext();
  const request = {
    thread_id: activeThreadId,
    mode: document.querySelector("#hs-chat-mode")?.value || "ask",
    prompt,
    connection_ids: context.includes("connections") ? (workspace?.connections || []).map((connection) => connection.connection_id) : [],
    dataset_keys: selectedDatasetKeys(context),
    goal_ids: goalId ? [goalId] : context.includes("goals") ? goals().map((goal) => goal.goal_id) : [],
    knowledge_query: context.includes("knowledge") ? prompt : null,
    model: document.querySelector("#hs-chat-model")?.value || null,
    proposed_action: null,
    world_mission: null,
  };
  sending = true;
  notice = null;
  mount(true);
  try {
    const result = await invoke("homeserver_agent_prompt", { request });
    activeThreadId = result.thread_id;
    workspace = await invoke("homeserver_agent_workspace");
    ensureActiveThread();
    notice = result.approvals_required ? { kind: "info", message: "HomeServer answered and created a supervised approval request." } : null;
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    sending = false;
    mount(true);
    document.querySelector("#hs-chat-input")?.focus();
  }
}

async function configureMcp(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const connectionId = form.dataset.mcpConfig || "";
  const clientId = new FormData(form).get("client_id")?.toString().trim() || "";
  if (!connectionId || !clientId || connectionBusy) return;
  connectionBusy = true;
  notice = null;
  mount(true);
  try {
    workspace.integrations = await invoke("homeserver_agent_integration_action", { request: {
      action: "configure",
      connection_id: connectionId,
      client_id: clientId,
      resource_uri: "https://mcp.microgifter.com/mcp",
      authorization_server: "https://microgifter.com",
      scopes: ["profile:read", "catalog:read"],
    }});
    notice = { kind: "success", message: "MCP client configured. Authorize it with Microgifter next." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    connectionBusy = false;
    mount(true);
  }
}

async function authorizeMcp(event) {
  const connectionId = event.currentTarget.dataset.mcpAuthorize || "";
  if (!connectionId || connectionBusy) return;
  connectionBusy = true;
  notice = null;
  mount(true);
  try {
    const result = await invoke("homeserver_agent_integration_action", { request: { action: "authorize", connection_id: connectionId } });
    await invoke("homeserver_open_agent_authorization", { url: result.authorization_url });
    notice = { kind: "info", message: "Complete authorization in your browser, then return here. HomeServer will accept the secure local callback." };
    window.setTimeout(() => void refreshAll(), 6000);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    connectionBusy = false;
    mount(true);
  }
}

async function refreshMcpTools(event) {
  const connectionId = event.currentTarget.dataset.mcpRefresh || "";
  if (!connectionId || connectionBusy) return;
  connectionBusy = true;
  mount(true);
  try {
    await invoke("homeserver_agent_integration_action", { request: { action: "refresh_tools", connection_id: connectionId } });
    workspace.integrations = await invoke("homeserver_agent_integrations");
    notice = { kind: "success", message: "Microgifter MCP tools refreshed." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    connectionBusy = false;
    mount(true);
  }
}

async function runGuidanceAction(event) {
  const target = event.currentTarget.dataset.guidanceAction || "";
  if (target === "agent:connections" || target === "agent:integrations") {
    connectionDrawerOpen = true;
    mount(true);
    return;
  }
  if (target.startsWith("agent:prompt:")) {
    const input = document.querySelector("#hs-chat-input");
    if (input) {
      input.value = target.slice("agent:prompt:".length);
      input.focus();
      autoSizeComposer();
    }
    return;
  }
  if (target.startsWith("#")) window.location.hash = target;
}

async function dismissGuidance(event) {
  const promptKey = event.currentTarget.dataset.dismissGuidance || "";
  if (!promptKey) return;
  try {
    workspace.integrations = await invoke("homeserver_agent_integration_action", { request: { action: "dismiss_guidance", prompt_key: promptKey } });
    mount(true);
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
    mount(true);
  }
}

async function connectMicrogifter(event) {
  event.preventDefault();
  connectionBusy = true;
  notice = null;
  mount(true);
  const request = {
    sync_code: document.querySelector("#hs-sync-code")?.value?.trim() || "",
    device_display_name: document.querySelector("#hs-device-name")?.value?.trim() || "HomeServer",
    cloud_base_url: document.querySelector("#hs-provider-url")?.value?.trim() || DEFAULT_PROVIDER_URL,
    merchant_id: document.querySelector("#hs-merchant-id")?.value?.trim() || null,
    site_id: document.querySelector("#hs-site-id")?.value?.trim() || null,
    request_id: crypto.randomUUID(),
    replacement_id: null,
    make_default: Boolean(document.querySelector("#hs-provider-default")?.checked),
  };
  try {
    await invoke("homeserver_connect_microgifter", { request });
    provider = await invoke("homeserver_microgifter_status");
    workspace = await invoke("homeserver_agent_workspace");
    connectFormOpen = false;
    notice = { kind: "success", message: "Microgifter connected. The Sync Code was exchanged and was not retained." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    connectionBusy = false;
    mount(true);
  }
}

async function runProviderAction(event) {
  const button = event.currentTarget;
  const connectionId = button.dataset.connectionId || "";
  const action = button.dataset.providerAction;
  if (!connectionId || !action || connectionBusy) return;
  if (action === "rotate" && !window.confirm("Rotate this device credential now? The existing credential remains intact if secure storage fails.")) return;
  connectionBusy = true;
  notice = null;
  mount(true);
  try {
    if (action === "heartbeat") await invoke("homeserver_send_microgifter_heartbeat", { connectionId });
    if (action === "refresh") await invoke("homeserver_refresh_microgifter_entitlement", { connectionId });
    if (action === "rotate") await invoke("homeserver_rotate_microgifter_credentials", { connectionId });
    provider = await invoke("homeserver_microgifter_status");
    notice = { kind: "success", message: action === "heartbeat" ? "Heartbeat completed." : action === "refresh" ? "Entitlement lease refreshed." : "Device credential rotated securely." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    connectionBusy = false;
    mount(true);
  }
}

function applyShellHealth(detail) {
  shellHealth = { ...shellHealth, ...(detail || {}) };
  if (!isAgentPage()) return;
  const runtime = document.querySelector(".hs-runtime-state");
  if (!runtime) return;
  const serviceOffline = shellHealth.service === "offline";
  const modelsDegraded = shellHealth.models === "degraded";
  runtime.textContent = serviceOffline
    ? "HomeServer offline"
    : modelsDegraded
      ? "Model runtime offline"
      : humanize(workspace?.model_runtime_state || "ready");
  runtime.classList.toggle("warn", serviceOffline || modelsDegraded);
}

window.addEventListener("homeserver-shell-health", (event) => applyShellHealth(event.detail));
window.addEventListener("homeserver-agent-route", () => scheduleMount());
scheduleMount();
