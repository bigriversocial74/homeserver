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
const MAX_ACTIVITY_EVENTS = 200;

const sessionBaselineAt = localStorage.getItem(STORAGE.lastActive) || null;
let workspace = null;
let provider = null;
let status = null;
let backups = null;
let updates = null;
let loading = false;
let connecting = false;
let showCodeForm = false;
let notificationDrawerOpen = false;
let successMessage = "";
let warningMessage = "";
let observer = null;
let injectQueued = false;
let activeRefreshTimer = null;
let activityTimer = null;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function safeJson(value, fallback) {
  try {
    const parsed = JSON.parse(value);
    return parsed ?? fallback;
  } catch {
    return fallback;
  }
}

function dateValue(value) {
  const timestamp = new Date(value || 0).getTime();
  return Number.isFinite(timestamp) ? timestamp : 0;
}

function formatDate(value) {
  if (!value) return "Not yet";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function relativeDate(value) {
  const timestamp = dateValue(value);
  if (!timestamp) return "Not yet";
  const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
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
  const items = safeJson(localStorage.getItem(STORAGE.activity) || "[]", []);
  return Array.isArray(items) ? items : [];
}

function recordActivity(type, title, detail, tone = "info", occurredAt = new Date().toISOString()) {
  const items = activityLog();
  const latest = items[0];
  if (latest && latest.type === type && latest.detail === detail && Date.now() - dateValue(latest.occurred_at_utc) < 30_000) return;
  items.unshift({
    id: `${type}:${occurredAt}:${Math.random().toString(36).slice(2, 8)}`,
    type,
    title,
    detail,
    tone,
    occurred_at_utc: occurredAt,
  });
  localStorage.setItem(STORAGE.activity, JSON.stringify(items.slice(0, MAX_ACTIVITY_EVENTS)));
}

function markUserActive() {
  localStorage.setItem(STORAGE.lastActive, new Date().toISOString());
}

function observeServiceState(nextStatus) {
  const next = nextStatus?.api_available ? "online" : "offline";
  const previous = localStorage.getItem(STORAGE.serviceState);
  if (previous !== next) {
    recordActivity(
      next === "online" ? "service.online" : "service.offline",
      next === "online" ? "HomeServer service available" : "HomeServer service unavailable",
      next === "online"
        ? "The Control Center detected the HomeServer LocalSystem service and local API."
        : "The Control Center could not reach the HomeServer LocalSystem service.",
      next === "online" ? "good" : "warn",
    );
    localStorage.setItem(STORAGE.serviceState, next);
  }
}

function providerConnections() {
  return Array.isArray(provider?.connections) ? provider.connections : [];
}

function activeConnection() {
  return providerConnections().find((connection) => connection.lifecycle_state === "active") || providerConnections()[0] || null;
}

function isPaired() {
  return providerConnections().some((connection) => ["active", "grace", "offline", "pairing_pending", "replacing"].includes(connection.lifecycle_state));
}

function observePairingState() {
  const next = isPaired() ? "paired" : "unpaired";
  const previous = localStorage.getItem(STORAGE.pairedState);
  if (previous && previous !== next) {
    recordActivity(
      next === "paired" ? "pairing.connected" : "pairing.disconnected",
      next === "paired" ? "Microgifter connected" : "Microgifter connection removed",
      next === "paired"
        ? "The HomeServer agent can now continue account-authorized onboarding."
        : "Microgifter cloud capabilities are unavailable until pairing is restored.",
      next === "paired" ? "good" : "warn",
    );
  }
  localStorage.setItem(STORAGE.pairedState, next);
}

function approvals() {
  return Array.isArray(workspace?.approvals) ? workspace.approvals : [];
}

function pendingApprovals() {
  return approvals().filter((approval) => approval.state === "pending");
}

function assistantMessagesSinceBaseline() {
  if (!sessionBaselineAt) return [];
  return (workspace?.messages || []).filter((message) => message.role !== "user" && dateValue(message.created_at_utc) > dateValue(sessionBaselineAt));
}

function providerReceiptsSinceBaseline() {
  if (!sessionBaselineAt) return [];
  return (provider?.recent_receipts || []).filter((receipt) => dateValue(receipt.created_at_utc) > dateValue(sessionBaselineAt));
}

function observedEventsSinceBaseline() {
  if (!sessionBaselineAt) return activityLog().slice(0, 6);
  return activityLog().filter((event) => dateValue(event.occurred_at_utc) > dateValue(sessionBaselineAt));
}

function backupCount() {
  return Array.isArray(backups?.backups) ? backups.backups.length : 0;
}

function modelReady() {
  return ["running", "ready", "available"].includes(String(workspace?.model_runtime_state || "").toLowerCase());
}

function updateReady() {
  const state = String(updates?.state || "idle").toLowerCase();
  return !["failed"].includes(state);
}

function assignedScopeCount(connection) {
  if (!connection) return 0;
  const capabilities = Array.isArray(connection.granted_capabilities)
    ? connection.granted_capabilities.filter((item) => item.grant_state === "granted").length
    : 0;
  return Number(connection.assigned_merchant_count || 0) + Number(connection.assigned_site_count || 0) + capabilities;
}

function notificationItems() {
  const items = [];
  const connection = activeConnection();
  const newMessages = assistantMessagesSinceBaseline();
  const pending = pendingApprovals();

  if (!status?.api_available) {
    items.push({ id: "service-offline", tone: "critical", icon: "!", title: "HomeServer service needs attention", detail: "The local service is not responding.", action: "system" });
  }
  if (!isPaired()) {
    items.push({ id: "pairing-required", tone: "warn", icon: "↗", title: "Pair HomeServer with Microgifter", detail: "Open the secure Microgifter pairing node from Agent Workspace.", action: "pairing" });
  }
  if (pending.length) {
    items.push({ id: "pending-approvals", tone: "warn", icon: "✓", title: `${pending.length} task${pending.length === 1 ? "" : "s"} waiting for approval`, detail: pending[0]?.risk_summary || "Review supervised Agent Workspace tasks.", action: "approvals" });
  }
  if (newMessages.length) {
    items.push({ id: "new-agent-messages", tone: "info", icon: "✦", title: `${newMessages.length} new agent message${newMessages.length === 1 ? "" : "s"}`, detail: newMessages.at(-1)?.content?.slice(0, 120) || "Open the latest HomeServer conversation.", action: "messages", threadId: newMessages.at(-1)?.thread_id });
  }
  if (isPaired() && assignedScopeCount(connection) === 0) {
    items.push({ id: "scope-review", tone: "warn", icon: "◇", title: "Connection permissions need review", detail: "Confirm merchant, site, dataset, and capability access in Microgifter.", action: "pairing-node" });
  }
  if (!backupCount()) {
    items.push({ id: "backup-required", tone: "warn", icon: "▣", title: "Create the first protected backup", detail: "HomeServer onboarding is not complete without a recovery point.", action: "backups" });
  }
  if (!modelReady()) {
    items.push({ id: "model-runtime", tone: "warn", icon: "AI", title: "Local model runtime is not ready", detail: "Install or start the selected local model.", action: "models" });
  }
  if (!updateReady()) {
    items.push({ id: "update-failed", tone: "critical", icon: "↑", title: "Signed update system needs attention", detail: "Review the HomeServer update state.", action: "system" });
  }
  if (!items.length) {
    items.push({ id: "all-clear", tone: "good", icon: "✓", title: "HomeServer is ready", detail: "No onboarding tasks or unread Agent Workspace items require attention.", action: "none" });
  }
  return items;
}

function onboardingSteps() {
  const connection = activeConnection();
  return [
    {
      key: "pairing",
      title: "Microgifter pairing",
      detail: isPaired() ? `Connected as ${connection?.device_display_name || "HomeServer"}.` : "Connect the account-owned Microgifter pairing node.",
      done: isPaired(),
      action: "pairing",
      label: "Pair now",
    },
    {
      key: "scope",
      title: "Merchant, site, and capability access",
      detail: assignedScopeCount(connection) > 0 ? "Microgifter returned authorized account scope." : "Review the exact data and actions this HomeServer may use.",
      done: assignedScopeCount(connection) > 0,
      action: "pairing-node",
      label: "Review access",
    },
    {
      key: "model",
      title: "Local model readiness",
      detail: modelReady() ? `Model runtime is ${humanize(workspace?.model_runtime_state)}.` : "Select, install, or start the local model runtime.",
      done: modelReady(),
      action: "models",
      label: "Model Center",
    },
    {
      key: "backup",
      title: "Protected recovery point",
      detail: backupCount() ? `${backupCount()} backup record${backupCount() === 1 ? "" : "s"} available.` : "Create and verify the first local backup.",
      done: backupCount() > 0,
      action: "backups",
      label: "Backups",
    },
    {
      key: "updates",
      title: "Signed update readiness",
      detail: updateReady() ? `Update state is ${humanize(updates?.state || "idle")}.` : "The signed update system needs attention.",
      done: updateReady(),
      action: "system",
      label: "Updates",
    },
  ];
}

function renderCodeForm() {
  if (!showCodeForm) return "";
  return `<form class="hs-agent-code-form" id="hs-agent-pairing-form">
    <p class="hs-agent-form-help">Paste the one-time Sync Code created by the signed-in Microgifter account. HomeServer exchanges it once and does not retain it.</p>
    <label>Microgifter Sync Code<input id="hs-agent-sync-code" type="password" autocomplete="one-time-code" minlength="6" maxlength="160" required placeholder="Enter Sync Code"></label>
    <div class="hs-agent-code-form-grid">
      <label>Device name<input id="hs-agent-device-name" maxlength="120" required value="Office HomeServer"></label>
      <label>Microgifter URL<input id="hs-agent-provider-url" type="url" maxlength="300" required value="${DEFAULT_PROVIDER_URL}"></label>
    </div>
    <div class="hs-agent-action-row"><button class="primary" type="submit" ${connecting ? "disabled" : ""}>${connecting ? "Connecting…" : "Connect and continue"}</button><button type="button" data-hs-onboarding-action="hide-code">Cancel</button></div>
  </form>`;
}

function renderPairingMessage() {
  return `<section class="hs-agent-system-message" id="hs-agent-pairing-message">
    <div class="hs-agent-system-avatar">HS</div>
    <article class="hs-agent-system-card">
      <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Pairing-first onboarding</span><h2>Connect this HomeServer to Microgifter</h2><p>I can take you directly to the secure, account-owned pairing node on the Microgifter website. Create the one-time Sync Code there, return here, and I will keep onboarding moving.</p></div><span class="hs-agent-state-pill">Not paired</span></header>
      <div class="hs-agent-system-body">
        <p>The pairing node stays on Microgifter because Microgifter owns account identity, device allowance, merchant/site authority, datasets, and capability grants. Your local data, models, conversations, and other wrapper connections remain on HomeServer.</p>
        <div class="hs-agent-action-row"><a class="primary" href="${PAIRING_NODE_URL}" target="_blank" rel="noopener noreferrer" data-hs-pairing-node>Open Microgifter Pairing Node</a><button type="button" data-hs-onboarding-action="show-code">I have a Sync Code</button></div>
        ${renderCodeForm()}
        ${warningMessage ? `<div class="hs-chat-notice warning">${escapeHtml(warningMessage)}</div>` : ""}
      </div>
    </article>
  </section>`;
}

function renderOnboardingContinuation() {
  const steps = onboardingSteps();
  const complete = steps.filter((step) => step.done).length;
  const percent = Math.round((complete / steps.length) * 100);
  return `<section class="hs-agent-system-message" id="hs-agent-onboarding-continuation">
    <div class="hs-agent-system-avatar">HS</div>
    <article class="hs-agent-system-card">
      <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Onboarding continues in chat</span><h2>${percent === 100 ? "Your HomeServer is ready" : "Pairing complete — let’s finish setup"}</h2><p>${percent === 100 ? "Agent Workspace is now your default HomeServer screen. I will surface tasks, messages, service changes, and important activity here." : "You do not need to restart onboarding or leave Agent Workspace. Complete the remaining items below in any order."}</p></div><span class="hs-agent-state-pill good">${percent}% ready</span></header>
      <div class="hs-agent-system-body">
        <div class="hs-agent-checklist">${steps.map((step) => `<div class="hs-agent-check ${step.done ? "done" : ""}"><i>${step.done ? "✓" : "•"}</i><span><strong>${escapeHtml(step.title)}</strong><small>${escapeHtml(step.detail)}</small></span>${step.done ? "" : `<button type="button" data-hs-onboarding-action="${escapeHtml(step.action)}">${escapeHtml(step.label)}</button>`}</div>`).join("")}</div>
        <div class="hs-agent-progress"><span style="width:${percent}%"></span></div>
        ${successMessage ? `<div class="hs-agent-pairing-success">${escapeHtml(successMessage)}</div>` : ""}
      </div>
    </article>
  </section>`;
}

function summaryEvents() {
  const items = [];
  observedEventsSinceBaseline().forEach((event) => items.push({ title: event.title, detail: event.detail, tone: event.tone, occurred_at_utc: event.occurred_at_utc }));
  providerReceiptsSinceBaseline().forEach((receipt) => items.push({ title: humanize(receipt.event_type), detail: `Microgifter connection receipt: ${humanize(receipt.result_category || "recorded")}.`, tone: receipt.result_category === "success" ? "good" : "warn", occurred_at_utc: receipt.created_at_utc }));
  assistantMessagesSinceBaseline().forEach((message) => items.push({ title: "New HomeServer agent message", detail: message.content?.slice(0, 160) || "Open the conversation to review it.", tone: "info", occurred_at_utc: message.created_at_utc }));
  return items.sort((a, b) => dateValue(b.occurred_at_utc) - dateValue(a.occurred_at_utc)).slice(0, 6);
}

function renderAwayMessage() {
  const messages = assistantMessagesSinceBaseline();
  const pending = pendingApprovals();
  const events = summaryEvents();
  return `<section class="hs-agent-system-message" id="hs-agent-away-message">
    <div class="hs-agent-system-avatar">HS</div>
    <article class="hs-agent-system-card">
      <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Activity history</span><h2>${sessionBaselineAt ? "Since you were last active" : "Your HomeServer activity starts here"}</h2><p>${sessionBaselineAt ? `Your previous Agent Workspace activity was ${formatDate(sessionBaselineAt)}. This summary uses local chat history, approvals, connection receipts, and Control Center service observations.` : "I will keep a local activity timeline and use it to summarize what changed between Agent Workspace sessions."}</p></div><span class="hs-agent-state-pill good">Local history</span></header>
      <div class="hs-agent-system-body">
        <div class="hs-agent-away-summary"><div><strong>${messages.length}</strong><span>new agent messages</span></div><div><strong>${pending.length}</strong><span>tasks awaiting approval</span></div><div><strong>${events.length}</strong><span>recorded changes</span></div></div>
        <div class="hs-agent-event-list">${events.length ? events.map((event) => `<div class="hs-agent-event ${event.tone === "good" ? "good" : event.tone === "warn" ? "warn" : ""}"><i></i><span><strong>${escapeHtml(event.title)}</strong><small>${escapeHtml(event.detail)}</small></span><time>${escapeHtml(relativeDate(event.occurred_at_utc))}</time></div>`).join("") : '<div class="hs-agent-event good"><i></i><span><strong>No new activity requires attention</strong><small>HomeServer will add service, pairing, task, and message activity as it occurs.</small></span><time>Now</time></div>'}</div>
      </div>
    </article>
  </section>`;
}

function renderOnboardingStack() {
  if (loading && !workspace && !provider) return "";
  return `<div class="hs-agent-onboarding-stack" data-hs-agent-onboarding>${isPaired() ? renderOnboardingContinuation() : renderPairingMessage()}${renderAwayMessage()}</div>`;
}

function renderNotificationButton() {
  const count = notificationItems().filter((item) => item.id !== "all-clear").length;
  return `<button type="button" class="hs-agent-notification-toggle" id="hs-agent-notification-toggle" aria-label="Agent notifications" aria-expanded="${notificationDrawerOpen ? "true" : "false"}">Alerts${count ? `<span class="hs-agent-notification-count">${Math.min(count, 99)}</span>` : ""}</button>`;
}

function renderNotificationDrawer() {
  if (!notificationDrawerOpen) return "";
  const items = notificationItems();
  const count = items.filter((item) => item.id !== "all-clear").length;
  return `<aside class="hs-agent-notification-drawer" data-hs-agent-notification-drawer><header><div><strong>Agent notifications</strong><span>${count ? `${count} task${count === 1 ? "" : "s"}, message${count === 1 ? "" : "s"}, or system item${count === 1 ? "" : "s"}` : "Everything looks good"}</span></div><button type="button" data-hs-onboarding-action="close-notifications" aria-label="Close notifications">×</button></header><div class="hs-agent-notification-list">${items.map((item) => `<button type="button" class="hs-agent-notification-item ${escapeHtml(item.tone)}" data-hs-notification-action="${escapeHtml(item.action)}" ${item.threadId ? `data-thread-id="${escapeHtml(item.threadId)}"` : ""}><i>${escapeHtml(item.icon)}</i><span><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.detail)}</small></span></button>`).join("")}</div></aside>`;
}

function queueInject() {
  if (injectQueued) return;
  injectQueued = true;
  window.requestAnimationFrame(() => {
    injectQueued = false;
    inject();
  });
}

function inject() {
  if (window.location.hash.replace("#", "") !== "agent") return;
  const page = document.querySelector(".hs-chat-page");
  const stream = page?.querySelector("#hs-chat-stream");
  const headerActions = page?.querySelector(".hs-chat-header-actions");
  const main = page?.querySelector(".hs-chat-main");
  if (!page || !stream || !headerActions || !main) return;

  const stackHtml = renderOnboardingStack();
  const existingStack = stream.querySelector("[data-hs-agent-onboarding]");
  if (stackHtml) {
    if (existingStack) existingStack.outerHTML = stackHtml;
    else stream.insertAdjacentHTML("afterbegin", stackHtml);
  } else {
    existingStack?.remove();
  }

  const existingToggle = headerActions.querySelector("#hs-agent-notification-toggle");
  if (existingToggle) existingToggle.outerHTML = renderNotificationButton();
  else headerActions.insertAdjacentHTML("afterbegin", renderNotificationButton());

  main.querySelector("[data-hs-agent-notification-drawer]")?.remove();
  if (notificationDrawerOpen) main.insertAdjacentHTML("beforeend", renderNotificationDrawer());
}

function routeTo(page) {
  notificationDrawerOpen = false;
  window.location.hash = `#${page}`;
}

function focusChatPrompt(text) {
  notificationDrawerOpen = false;
  queueInject();
  window.setTimeout(() => {
    const input = document.querySelector("#hs-chat-input");
    if (!input) return;
    input.value = text;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.focus();
  }, 0);
}

function handleNotificationAction(action, threadId) {
  notificationDrawerOpen = false;
  if (action === "pairing" || action === "pairing-node") {
    document.querySelector("#hs-agent-pairing-message,#hs-agent-onboarding-continuation")?.scrollIntoView({ behavior: "smooth", block: "start" });
    if (action === "pairing-node") window.open(PAIRING_NODE_URL, "_blank", "noopener,noreferrer");
  } else if (action === "approvals") {
    focusChatPrompt("Show my pending approval tasks and explain the most urgent one first.");
  } else if (action === "messages") {
    const threadButton = threadId ? document.querySelector(`[data-chat-thread="${CSS.escape(threadId)}"]`) : null;
    if (threadButton) threadButton.click();
    else focusChatPrompt("Summarize the new HomeServer agent messages since I was last active.");
  } else if (["system", "backups", "models"].includes(action)) {
    routeTo(action);
  }
  queueInject();
}

async function connectWithSyncCode(event) {
  event.preventDefault();
  if (connecting) return;
  const syncCode = document.querySelector("#hs-agent-sync-code")?.value?.trim() || "";
  const deviceName = document.querySelector("#hs-agent-device-name")?.value?.trim() || "HomeServer";
  const providerUrl = document.querySelector("#hs-agent-provider-url")?.value?.trim() || DEFAULT_PROVIDER_URL;
  if (!syncCode) return;

  connecting = true;
  warningMessage = "";
  successMessage = "";
  queueInject();
  try {
    await invoke("homeserver_connect_microgifter", {
      request: {
        sync_code: syncCode,
        device_display_name: deviceName,
        cloud_base_url: providerUrl,
        merchant_id: null,
        site_id: null,
        request_id: crypto.randomUUID(),
        replacement_id: null,
        make_default: true,
      },
    });
    recordActivity("pairing.connected", "Microgifter connected", "The one-time Sync Code was exchanged and onboarding continued inside Agent Workspace.", "good");
    successMessage = "Microgifter is connected. I kept your onboarding session open and moved to the remaining setup checks.";
    showCodeForm = false;
    await refreshData();
  } catch (error) {
    warningMessage = String(error);
  } finally {
    connecting = false;
    queueInject();
  }
}

async function refreshData() {
  if (loading) return;
  loading = true;
  queueInject();
  const results = await Promise.allSettled([
    invoke("homeserver_agent_workspace"),
    invoke("homeserver_microgifter_status"),
    invoke("homeserver_status"),
    invoke("homeserver_backups"),
    invoke("homeserver_updates"),
  ]);

  if (results[0].status === "fulfilled") workspace = results[0].value;
  if (results[1].status === "fulfilled") provider = results[1].value;
  if (results[2].status === "fulfilled") status = results[2].value;
  if (results[3].status === "fulfilled") backups = results[3].value;
  if (results[4].status === "fulfilled") updates = results[4].value;

  observeServiceState(status);
  observePairingState();
  loading = false;
  queueInject();
}

function scheduleRefresh() {
  window.clearInterval(activeRefreshTimer);
  activeRefreshTimer = window.setInterval(() => {
    if (document.visibilityState === "visible" && window.location.hash.replace("#", "") === "agent") void refreshData();
  }, isPaired() ? 30_000 : 10_000);
}

document.addEventListener("click", (event) => {
  const actionButton = event.target instanceof Element ? event.target.closest("[data-hs-onboarding-action]") : null;
  if (actionButton) {
    const action = actionButton.dataset.hsOnboardingAction;
    if (action === "show-code") showCodeForm = true;
    else if (action === "hide-code") showCodeForm = false;
    else if (action === "close-notifications") notificationDrawerOpen = false;
    else if (action === "pairing") {
      showCodeForm = true;
      document.querySelector("#hs-agent-pairing-message")?.scrollIntoView({ behavior: "smooth", block: "start" });
    } else if (action === "pairing-node") window.open(PAIRING_NODE_URL, "_blank", "noopener,noreferrer");
    else if (["system", "backups", "models"].includes(action)) routeTo(action);
    queueInject();
    return;
  }

  const toggle = event.target instanceof Element ? event.target.closest("#hs-agent-notification-toggle") : null;
  if (toggle) {
    notificationDrawerOpen = !notificationDrawerOpen;
    queueInject();
    return;
  }

  const notification = event.target instanceof Element ? event.target.closest("[data-hs-notification-action]") : null;
  if (notification) {
    handleNotificationAction(notification.dataset.hsNotificationAction || "none", notification.dataset.threadId || null);
  }
});

document.addEventListener("submit", (event) => {
  if (event.target instanceof HTMLFormElement && event.target.id === "hs-agent-pairing-form") void connectWithSyncCode(event);
});

window.addEventListener("homeserver-shell-health", (event) => {
  if (event.detail?.service === "offline") observeServiceState({ api_available: false });
  if (event.detail?.service === "online") observeServiceState({ api_available: true });
  queueInject();
});

window.addEventListener("homeserver-agent-route", () => {
  window.setTimeout(() => {
    void refreshData();
    queueInject();
  }, 0);
});

window.addEventListener("hashchange", () => {
  notificationDrawerOpen = false;
  if (window.location.hash.replace("#", "") === "agent") {
    void refreshData();
    queueInject();
  }
});

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") {
    markUserActive();
    if (window.location.hash.replace("#", "") === "agent") void refreshData();
  } else {
    markUserActive();
  }
});

window.addEventListener("beforeunload", () => {
  markUserActive();
  recordActivity("control_center.closed", "Control Center session ended", "The local Agent Workspace window ended or reloaded.", "info");
});

recordActivity("control_center.opened", "Control Center session opened", "Agent Workspace became the active HomeServer screen.", "good");
markUserActive();
activityTimer = window.setInterval(markUserActive, 60_000);
observer = new MutationObserver(queueInject);
observer.observe(document.querySelector("#app") || document.body, { childList: true, subtree: true });
void refreshData().then(scheduleRefresh);
queueInject();

window.__HOMESERVER_PAIRING_ONBOARDING_V1__ = {
  refresh: refreshData,
  pairingNodeUrl: PAIRING_NODE_URL,
  stop() {
    window.clearInterval(activeRefreshTimer);
    window.clearInterval(activityTimer);
    observer?.disconnect();
  },
};
