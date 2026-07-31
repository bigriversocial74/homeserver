import { invoke } from "@tauri-apps/api/core";

let workspace = null;
let timer = null;
let retryTimer = null;
let queued = false;
let loading = false;
let loadError = null;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function timestamp(value) {
  const result = new Date(value || 0).getTime();
  return Number.isFinite(result) ? result : 0;
}

function formatDate(value) {
  if (!value) return "Not recorded";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function relativeDate(value) {
  const time = timestamp(value);
  if (!time) return "Not recorded";
  const seconds = Math.max(0, Math.floor((Date.now() - time) / 1000));
  if (seconds < 60) return "Just now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  if (seconds < 604800) return `${Math.floor(seconds / 86400)}d ago`;
  return new Date(value).toLocaleDateString();
}

function humanize(value) {
  return String(value || "activity").replaceAll("_", " ").replaceAll(".", " ");
}

function isAgentPage() {
  return window.location.hash.replace("#", "") === "agent";
}

function activity() {
  return workspace?.activity || null;
}

function baseline() {
  return activity()?.last_user_active_at_utc || null;
}

function pendingApprovals() {
  return (workspace?.approvals || []).filter((item) => item.state === "pending");
}

function newMessages() {
  const since = timestamp(baseline());
  if (!since) return [];
  return (workspace?.messages || []).filter((item) => item.role !== "user" && timestamp(item.created_at_utc) > since);
}

function recentEvents() {
  const since = timestamp(baseline());
  const events = Array.isArray(activity()?.recent_events) ? activity().recent_events : [];
  return events
    .filter((item) => !since || timestamp(item.created_at_utc) > since)
    .slice(0, 8);
}

function eventTone(type) {
  const value = String(type || "");
  if (value.includes("failed") || value.includes("offline") || value.includes("rolled_back")) return "warn";
  if (value.includes("started") || value.includes("active") || value.includes("completed") || value.includes("connected")) return "good";
  return "";
}

function lifecycleSummary() {
  const data = activity();
  if (!data) return "Durable activity history is loading.";
  if (data.previous_session_started_at_utc && !data.previous_session_clean) {
    return `The previous HomeServer session began ${formatDate(data.previous_session_started_at_utc)} and did not record a clean shutdown before this service start.`;
  }
  if (data.previous_session_stopped_at_utc) {
    return `The previous HomeServer session stopped cleanly ${formatDate(data.previous_session_stopped_at_utc)}. The current service started ${formatDate(data.current_session_started_at_utc)}.`;
  }
  return `The current HomeServer service started ${formatDate(data.current_session_started_at_utc)}.`;
}

function renderLoading() {
  return `<section class="hs-agent-system-message" id="hs-agent-away-message" data-durable-activity-card>
    <div class="hs-agent-system-avatar">HS</div>
    <article class="hs-agent-system-card">
      <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Durable activity history</span><h2>Loading recent HomeServer activity</h2><p>Reading the local activity snapshot and service receipts.</p></div><span class="hs-agent-state-pill">Loading</span></header>
      <div class="hs-agent-system-body"><div class="hs-agent-event-list"><div class="hs-agent-event"><i></i><span><strong>Connecting to the local service</strong><small>This section updates automatically when the Agent Workspace snapshot is ready.</small></span><time>Now</time></div></div></div>
    </article>
  </section>`;
}

function renderError() {
  return `<section class="hs-agent-system-message" id="hs-agent-away-message" data-durable-activity-card>
    <div class="hs-agent-system-avatar">HS</div>
    <article class="hs-agent-system-card">
      <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Durable activity history</span><h2>Activity history is temporarily unavailable</h2><p>The Agent page could not read the local activity snapshot. It will retry automatically.</p></div><span class="hs-agent-state-pill warn">Retrying</span></header>
      <div class="hs-agent-system-body"><div class="hs-agent-event-list"><div class="hs-agent-event warn"><i></i><span><strong>Local activity request did not complete</strong><small>Agent Chat remains available while HomeServer retries the durable history request.</small></span><time>Current</time></div></div></div>
    </article>
  </section>`;
}

function render() {
  if (loadError && !activity()) return renderError();
  if (loading && !activity()) return renderLoading();

  const data = activity();
  if (!data) return renderLoading();
  const messages = newMessages();
  const approvals = pendingApprovals();
  const events = recentEvents();
  return `<section class="hs-agent-system-message" id="hs-agent-away-message" data-durable-activity-card>
    <div class="hs-agent-system-avatar">HS</div>
    <article class="hs-agent-system-card">
      <header class="hs-agent-system-head"><div><span class="hs-agent-kicker">Durable activity history</span><h2>${baseline() ? "Since you were last active" : "HomeServer lifecycle history"}</h2><p>${baseline() ? `Your previous recorded Agent Workspace activity was ${formatDate(baseline())}. ${lifecycleSummary()}` : lifecycleSummary()}</p></div><span class="hs-agent-state-pill good">Service receipts</span></header>
      <div class="hs-agent-system-body">
        <div class="hs-agent-away-summary"><div><strong>${messages.length}</strong><span>new agent messages</span></div><div><strong>${approvals.length}</strong><span>tasks awaiting approval</span></div><div><strong>${events.length}</strong><span>service events</span></div></div>
        <div class="hs-agent-event-list">${events.length ? events.map((item) => `<div class="hs-agent-event ${eventTone(item.event_type)}"><i></i><span><strong>${escapeHtml(humanize(item.event_type))}</strong><small>${escapeHtml(item.message)}</small></span><time>${escapeHtml(relativeDate(item.created_at_utc))}</time></div>`).join("") : '<div class="hs-agent-event good"><i></i><span><strong>No new durable events require attention</strong><small>Service starts, clean stops, Control Center activity, backup, restore, update, pairing, and other recorded events will appear here.</small></span><time>Current</time></div>'}</div>
        ${data.current_session_started_at_utc ? `<p class="hs-agent-form-help">Current service session started: ${escapeHtml(formatDate(data.current_session_started_at_utc))}. Previous session clean: ${data.previous_session_clean ? "Yes" : "No or not yet known"}.</p>` : ""}
      </div>
    </article>
  </section>`;
}

function inject() {
  if (!isAgentPage()) return false;
  const current = document.querySelector("#hs-agent-away-message");
  const host = document.querySelector(".hs-chat-stream");
  if (!current && !host) return false;

  const markup = render();
  if (current) current.outerHTML = markup;
  else host.insertAdjacentHTML("afterbegin", markup);
  return true;
}

function queueInject(attempt = 0) {
  if (queued) return;
  queued = true;
  requestAnimationFrame(() => {
    queued = false;
    const injected = inject();
    if (!injected && isAgentPage() && attempt < 20) {
      clearTimeout(retryTimer);
      retryTimer = setTimeout(() => queueInject(attempt + 1), 75);
    }
  });
}

async function refresh() {
  if (!isAgentPage()) return;
  loading = true;
  loadError = null;
  queueInject();
  try {
    workspace = await invoke("homeserver_agent_workspace");
  } catch (error) {
    workspace = null;
    loadError = String(error?.message || error || "activity history unavailable");
    console.warn("HomeServer durable activity history unavailable", error);
  } finally {
    loading = false;
    queueInject();
  }
}

window.addEventListener("homeserver:rendered", queueInject);
window.addEventListener("homeserver-agent-route", () => setTimeout(() => void refresh(), 0));
window.addEventListener("hashchange", () => {
  if (isAgentPage()) void refresh();
});
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible" && isAgentPage()) void refresh();
});

queueInject();
void refresh();
timer = setInterval(() => {
  if (document.visibilityState === "visible" && isAgentPage()) void refresh();
}, 30000);

window.__HOMESERVER_DURABLE_ACTIVITY_UI_V1__ = {
  refresh,
  stop() {
    clearInterval(timer);
    clearTimeout(retryTimer);
  },
};
