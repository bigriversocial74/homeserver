import { invoke } from "@tauri-apps/api/core";
import "./pairing-onboarding.css";

const PAIRING_NODE_URL = "https://microgifter.com/account-homeserver.php?source=homeserver-agent";
const DEFAULT_PROVIDER_URL = "https://microgifter.com";
const STORAGE = {
  lastActive: "homeserver-agent-last-active-at",
  serviceState: "homeserver-observed-service-state",
  pairedState: "homeserver-observed-pairing-state",
  activity: "homeserver-observed-activity-v1",
};
const sessionBaselineAt = localStorage.getItem(STORAGE.lastActive) || null;
const state = {
  workspace: null,
  provider: null,
  status: null,
  backups: null,
  updates: null,
  loading: false,
  connecting: false,
  showCode: false,
  notificationsOpen: false,
  success: "",
  warning: "",
};
let observer = null;
let refreshTimer = null;
let activityTimer = null;
let injectQueued = false;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function parseJson(value, fallback) {
  try {
    return JSON.parse(value) ?? fallback;
  } catch {
    return fallback;
  }
}

function timestamp(value) {
  const result = new Date(value || 0).getTime();
  return Number.isFinite(result) ? result : 0;
}

function formatDate(value) {
  if (!value) return "Not yet";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function relativeDate(value) {
  const time = timestamp(value);
  if (!time) return "Not yet";
  const seconds = Math.max(0, Math.floor((Date.now() - time) / 1000));
  if (seconds < 60) return "Just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  if (seconds < 604800) return `${Math.floor(seconds / 86400)}d ago`;
  return new Date(value).toLocaleDateString();
}

function humanize(value) {
  return String(value || "unknown").replaceAll("_", " ");
}

function activityLog() {
  const value = parseJson(localStorage.getItem(STORAGE.activity) || "[]", []);
  return Array.isArray(value) ? value : [];
}

function recordActivity(type, title, detail, tone = "info") {
  const items = activityLog();
  if (items[0]?.type === type && items[0]?.detail === detail && Date.now() - timestamp(items[0]?.occurred_at_utc) < 30000) return;
  items.unshift({
    id: `${type}:${Date.now()}`,
    type,
    title,
    detail,
    tone,
    occurred_at_utc: new Date().toISOString(),
  });
  localStorage.setItem(STORAGE.activity, JSON.stringify(items.slice(0, 200)));
}

function markUserActive() {
  localStorage.setItem(STORAGE.lastActive, new Date().toISOString());
}

function connections() {
  return Array.isArray(state.provider?.connections) ? state.provider.connections : [];
}

function activeConnection() {
  return connections().find((item) => item.lifecycle_state === "active") || connections()[0] || null;
}

function isPaired() {
  return connections().some((item) => ["active", "grace", "offline", "pairing_pending", "replacing"].includes(item.lifecycle_state));
}

function pendingApprovals() {
  return (state.workspace?.approvals || []).filter((item) => item.state === "pending");
}

function newMessages() {
  if (!sessionBaselineAt) return [];
  return (state.workspace?.messages || []).filter((item) => item.role !== "user" && timestamp(item.created_at_utc) > timestamp(sessionBaselineAt));
}

function newReceipts() {
  if (!sessionBaselineAt) return [];
  return (state.provider?.recent_receipts || []).filter((item) => timestamp(item.created_at_utc) > timestamp(sessionBaselineAt));
}

function newObservedEvents() {
  if (!sessionBaselineAt) return activityLog().slice(0, 6);
  return activityLog().filter((item) => timestamp(item.occurred_at_utc) > timestamp(sessionBaselineAt));
}

function backupCount() {
  return Array.isArray(state.backups?.backups) ? state.backups.backups.length : 0;
}

function modelReady() {
  return ["running", "ready", "available"].includes(String(state.workspace?.model_runtime_state || "").toLowerCase());
}

function updateReady() {
  return String(state.updates?.state || "idle").toLowerCase() !== "failed";
}

function scopeCount() {
  const connection = activeConnection();
  if (!connection) return 0;
  const grants = Array.isArray(connection.granted_capabilities)
    ? connection.granted_capabilities.filter((item) => item.grant_state === "granted").length
    : 0;
  return Number(connection.assigned_merchant_count || 0) + Number(connection.assigned_site_count || 0) + grants;
}

function observeStates() {
  const service = state.status?.api_available ? "online" : "offline";
  const previousService = localStorage.getItem(STORAGE.serviceState);
  if (previousService !== service) {
    recordActivity(
      service === "online" ? "service.online" : "service.offline",
      service === "online" ? "HomeServer service available" : "HomeServer service unavailable",
      service === "online"
        ? "The Control Center detected the HomeServer LocalSystem service and local API."
        : "The Control Center could not reach the HomeServer LocalSystem service.",
      service === "online" ? "good" : "warn",
    );
    localStorage.setItem(STORAGE.serviceState, service);
  }

  const paired = isPaired() ? "paired" : "unpaired";
  const previousPaired = localStorage.getItem(STORAGE.pairedState);
  if (previousPaired && previousPaired !== paired) {
    recordActivity(
      paired === "paired" ? "pairing.connected" : "pairing.disconnected",
      paired === "paired" ? "Microgifter connected" : "Microgifter connection removed",
      paired === "paired"
        ? "The HomeServer agent continued the account-authorized onboarding flow."
        : "Microgifter cloud capabilities are unavailable until pairing is restored.",
      paired === "paired" ? "good" : "warn",
    );
  }
  localStorage.setItem(STORAGE.pairedState, paired);
}

function notifications() {
  const items = [];
  const approvals = pendingApprovals();
  const messages = newMessages();
  if (!state.status?.api_available) items.push({ id: "service", tone: "critical", icon: "!", title: "HomeServer service needs attention", detail: "The local service is not responding.", action: "system" });
  if (!isPaired()) items.push({ id: "pairing", tone: "warn", icon: "↗", title: "Pair HomeServer with Microgifter", detail: "Open the secure Microgifter pairing node from this chat.", action: "pairing" });
  if (approvals.length) items.push({ id: "approvals", tone: "warn", icon: "✓", title: `${approvals.length} task${approvals.length === 1 ? "" : "s"} waiting for approval`, detail: approvals[0]?.risk_summary || "Review supervised Agent Workspace tasks.", action: "approvals" });
  if (messages.length) items.push({ id: "messages", tone: "info", icon: "✦", title: `${messages.length} new agent message${messages.length === 1 ? "" : "s"}`, detail: messages.at(-1)?.content?.slice(0, 120) || "Open the latest conversation.", action: "messages", threadId: messages.at(-1)?.thread_id });
  if (isPaired() && !scopeCount()) items.push({ id: "scope", tone: "warn", icon: "◇", title: "Connection permissions need review", detail: "Confirm merchant, site, dataset, and capability access in Microgifter.", action: "pairing-node" });
  if (!backupCount()) items.push({ id: "backup", tone: "warn", icon: "▣", title: "Create the first protected backup", detail: "HomeServer onboarding is not complete without a recovery point.", action: "backups" });
  if (!modelReady()) items.push({ id: "model", tone: "warn", icon: "AI", title: "Local model runtime is not ready", detail: "Install or start the selected local model.", action: "models" });
  if (!updateReady()) items.push({ id: "updates", tone: "critical", icon: "↑", title: "Signed update system needs attention", detail: "Review the HomeServer update state.", action: "system" });
  if (!items.length) items.push({ id: "clear", tone: "good", icon: "✓", title: "HomeServer is ready", detail: "No onboarding tasks or unread Agent Workspace items require attention.", action: "none" });
  return items;
}

function onboardingSteps() {
  const connection = activeConnection();
  return [
    { title: "Microgifter pairing", detail: isPaired() ? `Connected as ${connection?.device_display_name || "HomeServer"}.` : "Connect the account-owned Microgifter pairing node.", done: isPaired(), action: "pairing", label: "Pair now" },
    { title: "Merchant, site, and capability access", detail: scopeCount() ? "Microgifter returned authorized account scope." : "Review the exact data and actions this HomeServer may use.", done: scopeCount() > 0, action: "pairing-node", label: "Review access" },
    { title: "Local model readiness", detail: modelReady() ? `Model runtime is ${humanize(state.workspace?.model_runtime_state)}.` : "Select, install, or start the local model runtime.", done: modelReady(), action: "models", label: "Model Center" },
    { title: "Protected recovery point", detail: backupCount() ? `${backupCount()} backup record${backupCount() === 1 ? "" : "s"} available.` : "Create and verify the first local backup.", done: backupCount() > 0, action: "backups", label: "Backups" },
    { title: "Signed update readiness", detail: updateReady() ? `Update state is ${humanize(state.updates?.state || "idle")}.` : "The signed update system needs attention.", done: updateReady(), action: "system", label: "Updates" },
  ];
}

function renderCodeForm() {
  if (!state.showCode) return "";
  return `<form class="hs-agent-code-form" id="hs-agent-pairing-form">
    <p class="hs-agent-form-help">Paste the one-time Sync Code created by the signed-in Microgifter account. HomeServer exchanges it once and does not retain it.</p>
    <label>Microgifter Sync Code<input id="hs-agent-sync-code" type="password" autocomplete="one-time-code" minlength="6" maxlength="160" required placeholder="Enter Sync Code"></label>
    <div class="hs-agent-code-form-grid"><label>Device name<input id="hs-agent-device-name" maxlength="120" required value="Office HomeServer"></label><label>Microgifter URL<input id="hs-agent-provider-url" type="url" maxlength="300" required value="${DEFAULT_PROVIDER_URL}"></label></div>
    <div class="hs-agent-action-row"><button class="primary" type="submit" ${state.connecting ? "disabled" : ""}>${state.connecting ? "Connecting…" : "Connect and continue"}</button><button type="button" data-hs-action="hide-code">Cancel</button></div>
  </form>`;
}

function renderPairing() {
  return `<section class="hs-agent-system-message" id="hs-agent-pairing-message"><div class="hs-agent-system-avatar">HS</div><article class="hs-agent-system-card">
    <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Pairing-first onboarding</span><h2>Connect this HomeServer to Microgifter</h2><p>I can take you directly to the secure, account-owned pairing node on the Microgifter website. Create the one-time Sync Code there, return here, and I will keep onboarding moving.</p></div><span class="hs-agent-state-pill">Not paired</span></header>
    <div class="hs-agent-system-body"><p>The pairing node stays on Microgifter because Microgifter owns account identity, device allowance, merchant/site authority, datasets, and capability grants. Your local data, models, conversations, and other wrapper connections remain on HomeServer.</p>
      <div class="hs-agent-action-row"><a class="primary" href="${PAIRING_NODE_URL}" target="_blank" rel="noopener noreferrer">Open Microgifter Pairing Node</a><button type="button" data-hs-action="show-code">I have a Sync Code</button></div>${renderCodeForm()}${state.warning ? `<div class="hs-chat-notice warning">${escapeHtml(state.warning)}</div>` : ""}</div>
  </article></section>`;
}

function renderContinuation() {
  const steps = onboardingSteps();
  const complete = steps.filter((item) => item.done).length;
  const percent = Math.round((complete / steps.length) * 100);
  return `<section class="hs-agent-system-message" id="hs-agent-onboarding-continuation"><div class="hs-agent-system-avatar">HS</div><article class="hs-agent-system-card">
    <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Onboarding continues in chat</span><h2>${percent === 100 ? "Your HomeServer is ready" : "Pairing complete — let’s finish setup"}</h2><p>${percent === 100 ? "Agent Workspace is now your default HomeServer screen. I will surface tasks, messages, service changes, and important activity here." : "You do not need to restart onboarding or leave Agent Workspace. Complete the remaining items below in any order."}</p></div><span class="hs-agent-state-pill good">${percent}% ready</span></header>
    <div class="hs-agent-system-body"><div class="hs-agent-checklist">${steps.map((item) => `<div class="hs-agent-check ${item.done ? "done" : ""}"><i>${item.done ? "✓" : "•"}</i><span><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.detail)}</small></span>${item.done ? "" : `<button type="button" data-hs-action="${escapeHtml(item.action)}">${escapeHtml(item.label)}</button>`}</div>`).join("")}</div><div class="hs-agent-progress"><span style="width:${percent}%"></span></div>${state.success ? `<div class="hs-agent-pairing-success">${escapeHtml(state.success)}</div>` : ""}</div>
  </article></section>`;
}

function summaryEvents() {
  const items = newObservedEvents().map((item) => ({ title: item.title, detail: item.detail, tone: item.tone, at: item.occurred_at_utc }));
  newReceipts().forEach((item) => items.push({ title: humanize(item.event_type), detail: `Microgifter connection receipt: ${humanize(item.result_category || "recorded")}.`, tone: item.result_category === "success" ? "good" : "warn", at: item.created_at_utc }));
  newMessages().forEach((item) => items.push({ title: "New HomeServer agent message", detail: item.content?.slice(0, 160) || "Open the conversation to review it.", tone: "info", at: item.created_at_utc }));
  return items.sort((a, b) => timestamp(b.at) - timestamp(a.at)).slice(0, 6);
}

function renderAway() {
  const messages = newMessages();
  const approvals = pendingApprovals();
  const events = summaryEvents();
  return `<section class="hs-agent-system-message" id="hs-agent-away-message"><div class="hs-agent-system-avatar">HS</div><article class="hs-agent-system-card">
    <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Activity history</span><h2>${sessionBaselineAt ? "Since you were last active" : "Your HomeServer activity starts here"}</h2><p>${sessionBaselineAt ? `Your previous Agent Workspace activity was ${formatDate(sessionBaselineAt)}. This summary uses local chat history, approvals, connection receipts, and Control Center service observations.` : "I will keep a local activity timeline and use it to summarize what changed between Agent Workspace sessions."}</p></div><span class="hs-agent-state-pill good">Local history</span></header>
    <div class="hs-agent-system-body"><div class="hs-agent-away-summary"><div><strong>${messages.length}</strong><span>new agent messages</span></div><div><strong>${approvals.length}</strong><span>tasks awaiting approval</span></div><div><strong>${events.length}</strong><span>recorded changes</span></div></div><div class="hs-agent-event-list">${events.length ? events.map((item) => `<div class="hs-agent-event ${item.tone === "good" ? "good" : item.tone === "warn" ? "warn" : ""}"><i></i><span><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.detail)}</small></span><time>${escapeHtml(relativeDate(item.at))}</time></div>`).join("") : '<div class="hs-agent-event good"><i></i><span><strong>No new activity requires attention</strong><small>HomeServer will add service, pairing, task, and message activity as it occurs.</small></span><time>Now</time></div>'}</div></div>
  </article></section>`;
}

function renderStack() {
  if (state.loading && !state.workspace && !state.provider) return "";
  return `<div class="hs-agent-onboarding-stack" data-hs-agent-onboarding>${isPaired() ? renderContinuation() : renderPairing()}${renderAway()}</div>`;
}

function renderNotificationButton() {
  const count = notifications().filter((item) => item.id !== "clear").length;
  return `<button type="button" class="hs-agent-notification-toggle" id="hs-agent-notification-toggle" aria-label="Agent notifications" aria-expanded="${state.notificationsOpen ? "true" : "false"}">Alerts${count ? `<span class="hs-agent-notification-count">${Math.min(count, 99)}</span>` : ""}</button>`;
}

function renderNotificationDrawer() {
  if (!state.notificationsOpen) return "";
  const items = notifications();
  const count = items.filter((item) => item.id !== "clear").length;
  return `<aside class="hs-agent-notification-drawer" data-hs-agent-notification-drawer><header><div><strong>Agent notifications</strong><span>${count ? `${count} task, message, or system item${count === 1 ? "" : "s"}` : "Everything looks good"}</span></div><button type="button" data-hs-action="close-notifications" aria-label="Close notifications">×</button></header><div class="hs-agent-notification-list">${items.map((item) => `<button type="button" class="hs-agent-notification-item ${escapeHtml(item.tone)}" data-hs-notification="${escapeHtml(item.action)}" ${item.threadId ? `data-thread-id="${escapeHtml(item.threadId)}"` : ""}><i>${escapeHtml(item.icon)}</i><span><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.detail)}</small></span></button>`).join("")}</div></aside>`;
}

function observeApp() {
  const root = document.querySelector("#app") || document.body;
  observer?.observe(root, { childList: true, subtree: true });
}

function queueInject() {
  if (injectQueued) return;
  injectQueued = true;
  requestAnimationFrame(() => {
    injectQueued = false;
    inject();
  });
}

function inject() {
  if (window.location.hash.replace("#", "") !== "agent") return;
  const page = document.querySelector(".hs-chat-page");
  const stream = page?.querySelector("#hs-chat-stream");
  const actions = page?.querySelector(".hs-chat-header-actions");
  const main = page?.querySelector(".hs-chat-main");
  if (!page || !stream || !actions || !main) return;

  observer?.disconnect();
  try {
    const stack = renderStack();
    stream.querySelector("[data-hs-agent-onboarding]")?.remove();
    if (stack) stream.insertAdjacentHTML("afterbegin", stack);

    actions.querySelector("#hs-agent-notification-toggle")?.remove();
    actions.insertAdjacentHTML("afterbegin", renderNotificationButton());

    main.querySelector("[data-hs-agent-notification-drawer]")?.remove();
    if (state.notificationsOpen) main.insertAdjacentHTML("beforeend", renderNotificationDrawer());
  } finally {
    observeApp();
  }
}

function routeTo(page) {
  state.notificationsOpen = false;
  window.location.hash = `#${page}`;
}

function focusPrompt(text) {
  state.notificationsOpen = false;
  queueInject();
  setTimeout(() => {
    const input = document.querySelector("#hs-chat-input");
    if (!input) return;
    input.value = text;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.focus();
  }, 0);
}

function handleNotification(action, threadId) {
  state.notificationsOpen = false;
  if (action === "pairing") {
    state.showCode = true;
    queueInject();
    setTimeout(() => document.querySelector("#hs-agent-pairing-message")?.scrollIntoView({ behavior: "smooth", block: "start" }), 0);
  } else if (action === "pairing-node") {
    window.open(PAIRING_NODE_URL, "_blank", "noopener,noreferrer");
  } else if (action === "approvals") {
    focusPrompt("Show my pending approval tasks and explain the most urgent one first.");
  } else if (action === "messages") {
    const selector = threadId && window.CSS?.escape ? `[data-chat-thread="${CSS.escape(threadId)}"]` : null;
    const thread = selector ? document.querySelector(selector) : null;
    if (thread) thread.click();
    else focusPrompt("Summarize the new HomeServer agent messages since I was last active.");
  } else if (["system", "backups", "models"].includes(action)) {
    routeTo(action);
  }
  queueInject();
}

async function connect(event) {
  event.preventDefault();
  if (state.connecting) return;
  const code = document.querySelector("#hs-agent-sync-code")?.value?.trim() || "";
  if (!code) return;
  state.connecting = true;
  state.warning = "";
  state.success = "";
  queueInject();
  try {
    await invoke("homeserver_connect_microgifter", {
      request: {
        sync_code: code,
        device_display_name: document.querySelector("#hs-agent-device-name")?.value?.trim() || "HomeServer",
        cloud_base_url: document.querySelector("#hs-agent-provider-url")?.value?.trim() || DEFAULT_PROVIDER_URL,
        merchant_id: null,
        site_id: null,
        request_id: crypto.randomUUID(),
        replacement_id: null,
        make_default: true,
      },
    });
    recordActivity("pairing.connected", "Microgifter connected", "The one-time Sync Code was exchanged and onboarding continued inside Agent Workspace.", "good");
    state.success = "Microgifter is connected. I kept your onboarding session open and moved to the remaining setup checks.";
    state.showCode = false;
    await refresh();
  } catch (error) {
    state.warning = String(error);
  } finally {
    state.connecting = false;
    queueInject();
  }
}

async function refresh() {
  if (state.loading) return;
  state.loading = true;
  queueInject();
  const results = await Promise.allSettled([
    invoke("homeserver_agent_workspace"),
    invoke("homeserver_microgifter_status"),
    invoke("homeserver_status"),
    invoke("homeserver_backups"),
    invoke("homeserver_updates"),
  ]);
  if (results[0].status === "fulfilled") state.workspace = results[0].value;
  if (results[1].status === "fulfilled") state.provider = results[1].value;
  if (results[2].status === "fulfilled") state.status = results[2].value;
  if (results[3].status === "fulfilled") state.backups = results[3].value;
  if (results[4].status === "fulfilled") state.updates = results[4].value;
  observeStates();
  state.loading = false;
  queueInject();
}

function scheduleRefresh() {
  clearInterval(refreshTimer);
  refreshTimer = setInterval(() => {
    if (document.visibilityState === "visible" && window.location.hash.replace("#", "") === "agent") void refresh();
  }, isPaired() ? 30000 : 10000);
}

document.addEventListener("click", (event) => {
  if (!(event.target instanceof Element)) return;
  const actionButton = event.target.closest("[data-hs-action]");
  if (actionButton) {
    const action = actionButton.dataset.hsAction;
    if (action === "show-code") state.showCode = true;
    else if (action === "hide-code") state.showCode = false;
    else if (action === "close-notifications") state.notificationsOpen = false;
    else if (action === "pairing") state.showCode = true;
    else if (action === "pairing-node") window.open(PAIRING_NODE_URL, "_blank", "noopener,noreferrer");
    else if (["system", "backups", "models"].includes(action)) routeTo(action);
    queueInject();
    return;
  }

  if (event.target.closest("#hs-agent-notification-toggle")) {
    state.notificationsOpen = !state.notificationsOpen;
    queueInject();
    return;
  }

  const notification = event.target.closest("[data-hs-notification]");
  if (notification) handleNotification(notification.dataset.hsNotification || "none", notification.dataset.threadId || null);
});

document.addEventListener("submit", (event) => {
  if (event.target instanceof HTMLFormElement && event.target.id === "hs-agent-pairing-form") void connect(event);
});

window.addEventListener("homeserver-shell-health", (event) => {
  if (event.detail?.service === "offline") state.status = { ...(state.status || {}), api_available: false };
  if (event.detail?.service === "online") state.status = { ...(state.status || {}), api_available: true };
  observeStates();
  queueInject();
});

window.addEventListener("homeserver-agent-route", () => setTimeout(() => { void refresh(); queueInject(); }, 0));
window.addEventListener("hashchange", () => {
  state.notificationsOpen = false;
  if (window.location.hash.replace("#", "") === "agent") void refresh();
  queueInject();
});

document.addEventListener("visibilitychange", () => {
  markUserActive();
  if (document.visibilityState === "visible" && window.location.hash.replace("#", "") === "agent") void refresh();
});

window.addEventListener("beforeunload", () => {
  markUserActive();
  recordActivity("control_center.closed", "Control Center session ended", "The local Agent Workspace window ended or reloaded.");
});

recordActivity("control_center.opened", "Control Center session opened", "Agent Workspace became the active HomeServer screen.", "good");
markUserActive();
activityTimer = setInterval(markUserActive, 60000);
observer = new MutationObserver(queueInject);
observeApp();
void refresh().then(scheduleRefresh);
queueInject();

window.__HOMESERVER_PAIRING_ONBOARDING_V1__ = {
  pairingNodeUrl: PAIRING_NODE_URL,
  refresh,
  stop() {
    clearInterval(refreshTimer);
    clearInterval(activityTimer);
    observer?.disconnect();
  },
};
