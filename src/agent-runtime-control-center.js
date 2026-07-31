import { invoke } from "@tauri-apps/api/core";
import { icon } from "./icons.js";
import "./agent-runtime-control-center.css";

const RUNTIME_ROUTE = "runtime";
const REFRESH_WINDOW_MS = 15_000;

const runtimeState = {
  runtime: null,
  authority: null,
  busy: false,
  error: null,
  lastLoadedAt: 0,
};

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function values(value) {
  return Array.isArray(value) ? value : [];
}

function routeName() {
  return window.location.hash.replace("#", "");
}

function isRuntimeRoute() {
  return routeName() === RUNTIME_ROUTE;
}

function humanize(value) {
  return String(value || "unknown").replaceAll("_", " ");
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
  return `${Math.floor(seconds / 86400)}d ago`;
}

function compactHash(value) {
  const text = String(value || "");
  if (!text) return "Not recorded";
  if (text.length <= 18) return text;
  return `${text.slice(0, 8)}…${text.slice(-8)}`;
}

function statusTone(state) {
  const value = String(state || "unknown").toLowerCase();
  if (["active", "completed", "succeeded", "running", "ready", "approved"].includes(value)) return "success";
  if (["failed", "cancelled", "revoked", "expired", "dead_letter"].includes(value)) return "danger";
  if (["queued", "waiting", "leased", "pending", "awaiting_approval", "suspended"].includes(value)) return "warning";
  return "neutral";
}

function statusBadge(state, label = null) {
  const text = label || humanize(state);
  return `<span class="runtime-status ${statusTone(state)}"><i></i>${escapeHtml(text)}</span>`;
}

function metric(iconName, label, value, detail, tone = "blue") {
  return `<article class="metric-card tone-${tone}"><div class="metric-icon">${icon(iconName, 22)}</div><div><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong><small>${escapeHtml(detail)}</small></div></article>`;
}

function ensureRuntimeNavigation() {
  const nav = document.querySelector(".primary-nav");
  if (!nav) return;
  let button = nav.querySelector("[data-agent-runtime-route]");
  if (!button) {
    button = document.createElement("button");
    button.type = "button";
    button.className = "nav-item";
    button.dataset.agentRuntimeRoute = "true";
    button.innerHTML = `${icon("activity", 19)}<span>Agent Runtime</span>`;
    const integrations = nav.querySelector('[data-page="integrations"]');
    if (integrations) integrations.insertAdjacentElement("afterend", button);
    else nav.append(button);
  }
  const active = isRuntimeRoute();
  button.classList.toggle("active", active);
  if (active) {
    nav.querySelectorAll(".nav-item").forEach((item) => {
      if (item !== button) item.classList.remove("active");
    });
  }
}

function safetyItem(label, detail, passed) {
  return `<div class="runtime-safety-item ${passed ? "passed" : "failed"}"><span>${icon(passed ? "check" : "warning", 17)}</span><div><strong>${escapeHtml(label)}</strong><small>${escapeHtml(detail)}</small></div></div>`;
}

function activePlans() {
  return values(runtimeState.runtime?.plans).filter((plan) => ["queued", "running"].includes(plan.state));
}

function queuedSteps() {
  return values(runtimeState.runtime?.steps).filter((step) => ["queued", "waiting", "leased", "running"].includes(step.state));
}

function pendingApprovals() {
  return values(runtimeState.authority?.approvals).filter((approval) => approval.state === "pending");
}

function activeStops() {
  return values(runtimeState.authority?.emergency_stops).filter((stop) => stop.state === "active");
}

function failedWorkCount() {
  const plans = values(runtimeState.runtime?.plans).filter((plan) => plan.state === "failed").length;
  const steps = values(runtimeState.runtime?.steps).filter((step) => step.state === "failed").length;
  return plans + steps;
}

function renderSafetyBoundary(runtime) {
  const inputSafe = runtime?.private_inputs_exposed === false;
  const resultSafe = runtime?.private_results_exposed === false;
  const bypassSafe = runtime?.direct_tool_bypass_allowed === false;
  const egressSafe = runtime?.phase16e_egress_required === true;
  const safe = inputSafe && resultSafe && bypassSafe && egressSafe;
  return `<section class="runtime-boundary ${safe ? "safe" : "unsafe"}">
    <div class="runtime-boundary-copy"><span>${icon(safe ? "shield" : "warning", 23)}</span><div><strong>${safe ? "Phase 16 authority boundary is intact" : "Runtime boundary needs attention"}</strong><p>The Control Center exposes only safe metadata. Private inputs and full private results remain local and every successful result must pass Phase 16E egress.</p></div></div>
    <div class="runtime-safety-grid">
      ${safetyItem("Private inputs", inputSafe ? "Hidden from snapshots" : "Exposure detected", inputSafe)}
      ${safetyItem("Private results", resultSafe ? "Hash-only evidence" : "Exposure detected", resultSafe)}
      ${safetyItem("Direct tool bypass", bypassSafe ? "Disabled" : "Enabled", bypassSafe)}
      ${safetyItem("Phase 16E egress", egressSafe ? "Required" : "Not required", egressSafe)}
    </div>
  </section>`;
}

function renderPlans() {
  const plans = values(runtimeState.runtime?.plans);
  const steps = values(runtimeState.runtime?.steps);
  if (!plans.length) {
    return `<div class="runtime-empty">${icon("activity", 30)}<strong>No runtime plans yet</strong><p>Authorized low-risk work will appear here after an assigned agent submits Phase 16C jobs.</p></div>`;
  }
  return `<div class="runtime-plan-list">${plans.slice(0, 20).map((plan) => {
    const planSteps = steps.filter((step) => step.plan_id === plan.plan_id).sort((a, b) => Number(a.sequence_number) - Number(b.sequence_number));
    const percent = plan.step_count ? Math.round((Number(plan.completed_step_count || 0) / Number(plan.step_count)) * 100) : 0;
    const cancellable = ["queued", "running"].includes(plan.state);
    return `<article class="runtime-plan-card">
      <div class="runtime-plan-heading"><div><span>Agent plan</span><h3>${escapeHtml(plan.title)}</h3></div>${statusBadge(plan.state)}</div>
      <p>${escapeHtml(plan.objective)}</p>
      <div class="runtime-plan-progress"><div><span>${Number(plan.completed_step_count || 0)} of ${Number(plan.step_count || 0)} steps</span><strong>${percent}%</strong></div><div class="runtime-progress"><i style="width:${Math.max(0, Math.min(100, percent))}%"></i></div></div>
      <div class="runtime-step-chips">${planSteps.map((step) => `<span class="${statusTone(step.state)}"><b>${Number(step.sequence_number)}</b>${escapeHtml(humanize(step.tool_key))}</span>`).join("")}</div>
      <footer><span>Updated ${escapeHtml(relativeDate(plan.updated_at_utc))}${plan.failure_code ? ` · ${escapeHtml(humanize(plan.failure_code))}` : ""}</span>${cancellable ? `<button class="button danger" type="button" data-runtime-cancel-plan="${escapeHtml(plan.plan_id)}" data-runtime-plan-title="${escapeHtml(plan.title)}">Cancel plan</button>` : ""}</footer>
    </article>`;
  }).join("")}</div>`;
}

function renderStepQueue() {
  const steps = values(runtimeState.runtime?.steps);
  const planMap = new Map(values(runtimeState.runtime?.plans).map((plan) => [plan.plan_id, plan.title]));
  if (!steps.length) return `<div class="runtime-empty compact"><strong>No step history</strong><p>Queued and completed tool steps will appear here.</p></div>`;
  return `<div class="runtime-table"><div class="runtime-table-head"><span>Order</span><span>Plan</span><span>Tool</span><span>State</span><span>Result</span><span>Updated</span></div>${steps.slice(0, 40).map((step) => `<div class="runtime-table-row"><strong>${Number(step.sequence_number)}</strong><span>${escapeHtml(planMap.get(step.plan_id) || "Runtime plan")}</span><code>${escapeHtml(step.tool_key)}</code>${statusBadge(step.state)}<span>${escapeHtml(humanize(step.result_code || step.failure_code || "pending"))}</span><span>${escapeHtml(relativeDate(step.completed_at_utc || step.started_at_utc || step.created_at_utc))}</span></div>`).join("")}</div>`;
}

function renderTools() {
  const tools = values(runtimeState.runtime?.tools);
  if (!tools.length) return `<div class="runtime-empty compact"><strong>No active runtime tools</strong><p>The local catalog is unavailable.</p></div>`;
  return `<div class="runtime-tool-grid">${tools.map((tool) => `<article><div><span>${icon(tool.risk_class === "read_only" ? "eye" : "shield", 18)}</span>${statusBadge(tool.state)}</div><h3>${escapeHtml(tool.tool_key)}</h3><p>${escapeHtml(tool.description)}</p><dl><div><dt>Risk</dt><dd>${escapeHtml(humanize(tool.risk_class))}</dd></div><div><dt>Approval</dt><dd>${escapeHtml(humanize(tool.approval_requirement))}</dd></div><div><dt>Timeout</dt><dd>${Number(tool.max_execution_seconds || 0)} seconds</dd></div><div><dt>Version</dt><dd>${escapeHtml(tool.version)}</dd></div></dl></article>`).join("")}</div>`;
}

function renderAuthority() {
  const authority = runtimeState.authority || {};
  const agents = values(authority.agents);
  const assignments = values(authority.assignments);
  const proposals = values(authority.proposals);
  const approvals = pendingApprovals();
  const stops = activeStops();
  const proposalMap = new Map(proposals.map((proposal) => [proposal.proposal_id, proposal]));
  return `<div class="runtime-authority-grid">
    <article class="panel"><div class="panel-title"><div>${icon("agent", 18)}<div><h2>Authorized Agents</h2><p>Current Phase 16D identities and wrapper assignments.</p></div></div><span>${agents.filter((agent) => agent.state === "active").length} active</span></div><div class="runtime-authority-list">${agents.length ? agents.slice(0, 12).map((agent) => `<div><span class="runtime-authority-icon">${icon("agent", 17)}</span><p><strong>${escapeHtml(agent.display_name)}</strong><small>${escapeHtml(agent.purpose)} · autonomy ${Number(agent.autonomy_level || 0)} · ${assignments.filter((assignment) => assignment.agent_id === agent.agent_id && assignment.state === "active").length} active assignments</small></p>${statusBadge(agent.state)}</div>`).join("") : `<div class="runtime-empty compact"><strong>No HomeServer agents</strong><p>Create and assign an agent before runtime work can begin.</p></div>`}</div></article>
    <article class="panel"><div class="panel-title"><div>${icon("shield", 18)}<div><h2>Approvals & Stops</h2><p>Sensitive actions stay on the separate supervised lifecycle.</p></div></div><span>${approvals.length + stops.length} need attention</span></div><div class="runtime-supervision-list">${approvals.map((approval) => { const proposal = proposalMap.get(approval.proposal_id); return `<div class="approval"><span>${icon("warning", 17)}</span><p><strong>${escapeHtml(proposal?.title || "Action approval")}</strong><small>${escapeHtml(humanize(proposal?.action_type || "sensitive action"))} · expires ${escapeHtml(formatDate(approval.expires_at_utc))}</small></p>${statusBadge(approval.state)}</div>`; }).join("")}${stops.map((stop) => `<div class="stop"><span>${icon("power", 17)}</span><p><strong>${escapeHtml(humanize(stop.scope_type))} emergency stop</strong><small>${escapeHtml(stop.reason)}${stop.expires_at_utc ? ` · expires ${escapeHtml(formatDate(stop.expires_at_utc))}` : ""}</small></p>${statusBadge(stop.state)}</div>`).join("")}${!approvals.length && !stops.length ? `<div class="runtime-empty compact"><strong>No pending approvals or active stops</strong><p>Supervised authority is clear.</p></div>` : ""}</div><button class="button secondary full" type="button" data-runtime-open-agent>${icon("agent", 16)}Open Agent Control Center</button></article>
  </div>`;
}

function renderReceipts() {
  const receipts = values(runtimeState.runtime?.receipts);
  if (!receipts.length) return `<div class="runtime-empty compact"><strong>No runtime receipts</strong><p>Immutable per-step evidence will appear after work completes or fails.</p></div>`;
  return `<div class="runtime-receipt-list">${receipts.slice(0, 30).map((receipt) => `<div><span class="runtime-receipt-icon">${icon(receipt.outcome === "success" ? "check" : "warning", 17)}</span><p><strong>${escapeHtml(receipt.tool_key)}</strong><small>${escapeHtml(humanize(receipt.result_code))} · ${escapeHtml(relativeDate(receipt.completed_at_utc))}</small></p>${statusBadge(receipt.outcome)}<code title="Runtime receipt hash">${escapeHtml(compactHash(receipt.runtime_receipt_hash))}</code></div>`).join("")}</div>`;
}

function renderRuntimePage() {
  if (!isRuntimeRoute()) return;
  ensureRuntimeNavigation();
  const canvas = document.querySelector(".page-canvas");
  if (!canvas) return;
  const runtime = runtimeState.runtime || {};
  const tools = values(runtime.tools);
  const receipts = values(runtime.receipts);
  const agents = values(runtimeState.authority?.agents);
  const loading = runtimeState.busy && !runtimeState.runtime;
  canvas.innerHTML = `<div class="runtime-control-center">
    ${runtimeState.error ? `<div class="notice warning"><strong>Agent Runtime:</strong> ${escapeHtml(runtimeState.error)}</div>` : ""}
    <header class="page-header"><div><h1>Agent Runtime</h1><p>Authorized local plans, tool execution, approvals, emergency stops, and immutable evidence.</p></div><div class="page-actions"><button class="button secondary" type="button" data-runtime-refresh ${runtimeState.busy ? "disabled" : ""}>${icon("refresh", 16)}Refresh</button><button class="button primary" type="button" data-runtime-run-once ${runtimeState.busy ? "disabled" : ""}>${icon("play", 16)}Run one cycle</button></div></header>
    ${loading ? `<div class="runtime-loading"><span></span><strong>Loading authorized runtime state…</strong></div>` : `
      <section class="metrics six-up">
        ${metric("activity", "Runtime", humanize(runtime.runtime_state || "offline"), runtime.worker_id ? "Persistent local worker" : "Worker unavailable", runtime.runtime_state === "active" ? "green" : "amber")}
        ${metric("agent", "Active Agents", String(agents.filter((agent) => agent.state === "active").length), `${agents.length} registered identities`, "blue")}
        ${metric("play", "Active Plans", String(activePlans().length), `${queuedSteps().length} queued or running steps`, activePlans().length ? "purple" : "gray")}
        ${metric("shield", "Pending Approvals", String(pendingApprovals().length), "Sensitive lifecycle only", pendingApprovals().length ? "amber" : "green")}
        ${metric("warning", "Failures", String(failedWorkCount()), `${activeStops().length} active emergency stops`, failedWorkCount() || activeStops().length ? "amber" : "green")}
        ${metric("logs", "Receipts", String(receipts.length), `${tools.length} cataloged tools`, "teal")}
      </section>
      ${renderSafetyBoundary(runtime)}
      <section class="runtime-primary-grid"><article class="panel runtime-plans-panel"><div class="panel-title"><div>${icon("activity", 18)}<div><h2>Runtime Plans</h2><p>Ordered low-risk work submitted through the Phase 16C job contract.</p></div></div><span>${values(runtime.plans).length} plans</span></div>${renderPlans()}</article><article class="panel runtime-tools-panel"><div class="panel-title"><div>${icon("apps", 18)}<div><h2>Tool Catalog</h2><p>HomeServer-owned, versioned, risk-classified adapters.</p></div></div><span>${tools.length} tools</span></div>${renderTools()}</article></section>
      ${renderAuthority()}
      <section class="runtime-bottom-grid"><article class="panel"><div class="panel-title"><div>${icon("activity", 18)}<div><h2>Step Queue & History</h2><p>Exact order, state, tool, result code, and completion time.</p></div></div><span>${values(runtime.steps).length} steps</span></div>${renderStepQueue()}</article><article class="panel"><div class="panel-title"><div>${icon("logs", 18)}<div><h2>Immutable Runtime Receipts</h2><p>Hash-only evidence linked to the Phase 16C job receipt.</p></div></div><span>${receipts.length} receipts</span></div>${renderReceipts()}</article></section>
    `}
    <footer class="app-footer"><span>Local-only authorized runtime</span><span>${runtimeState.lastLoadedAt ? `Updated: ${escapeHtml(new Date(runtimeState.lastLoadedAt).toLocaleString())}` : "Not loaded"}</span></footer>
  </div>`;
}

async function refreshRuntime(quiet = false) {
  if (runtimeState.busy) return;
  runtimeState.busy = true;
  runtimeState.error = null;
  if (!quiet) renderRuntimePage();
  const results = await Promise.allSettled([
    invoke("homeserver_agent_runtime"),
    invoke("homeserver_agent_authority"),
  ]);
  if (results[0].status === "fulfilled") runtimeState.runtime = results[0].value;
  if (results[1].status === "fulfilled") runtimeState.authority = results[1].value;
  const errors = results.filter((result) => result.status === "rejected").map((result) => String(result.reason));
  runtimeState.error = errors.length ? errors.join(" · ") : null;
  runtimeState.lastLoadedAt = Date.now();
  runtimeState.busy = false;
  renderRuntimePage();
}

async function runOneCycle() {
  if (runtimeState.busy) return;
  runtimeState.busy = true;
  runtimeState.error = null;
  renderRuntimePage();
  try {
    runtimeState.runtime = await invoke("homeserver_run_agent_runtime_once");
    const authority = await invoke("homeserver_agent_authority");
    runtimeState.authority = authority;
    runtimeState.lastLoadedAt = Date.now();
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    renderRuntimePage();
  }
}

async function cancelRuntimePlan(button) {
  if (runtimeState.busy) return;
  const planId = button.dataset.runtimeCancelPlan || "";
  const title = button.dataset.runtimePlanTitle || "this plan";
  const confirmation = window.prompt(`Type CANCEL PLAN ${planId} to cancel ${title}:`);
  if (confirmation !== `CANCEL PLAN ${planId}`) return;
  const reason = window.prompt("Reason for cancellation:", "Cancelled from Agent Runtime Control Center");
  if (reason === null || !reason.trim()) return;
  runtimeState.busy = true;
  runtimeState.error = null;
  renderRuntimePage();
  try {
    runtimeState.runtime = await invoke("homeserver_cancel_agent_runtime_plan", {
      planId,
      confirmation,
      reason: reason.trim(),
    });
    runtimeState.lastLoadedAt = Date.now();
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    renderRuntimePage();
  }
}

document.addEventListener("click", (event) => {
  if (!(event.target instanceof Element)) return;
  const routeButton = event.target.closest("[data-agent-runtime-route]");
  if (routeButton) {
    event.preventDefault();
    window.location.hash = `#${RUNTIME_ROUTE}`;
    renderRuntimePage();
    void refreshRuntime(false);
    return;
  }
  const refreshButton = event.target.closest("[data-runtime-refresh]");
  if (refreshButton) {
    event.preventDefault();
    void refreshRuntime(false);
    return;
  }
  const runButton = event.target.closest("[data-runtime-run-once]");
  if (runButton) {
    event.preventDefault();
    void runOneCycle();
    return;
  }
  const cancelButton = event.target.closest("[data-runtime-cancel-plan]");
  if (cancelButton) {
    event.preventDefault();
    void cancelRuntimePlan(cancelButton);
    return;
  }
  const agentButton = event.target.closest("[data-runtime-open-agent]");
  if (agentButton) {
    event.preventDefault();
    window.location.hash = "#agent";
  }
});

window.addEventListener("hashchange", () => {
  ensureRuntimeNavigation();
  if (isRuntimeRoute()) {
    renderRuntimePage();
    if (Date.now() - runtimeState.lastLoadedAt > REFRESH_WINDOW_MS) void refreshRuntime(true);
  }
});

window.addEventListener("homeserver:rendered", () => {
  ensureRuntimeNavigation();
  if (!isRuntimeRoute()) return;
  renderRuntimePage();
  if (Date.now() - runtimeState.lastLoadedAt > REFRESH_WINDOW_MS) void refreshRuntime(true);
});

ensureRuntimeNavigation();
if (isRuntimeRoute()) void refreshRuntime(false);
