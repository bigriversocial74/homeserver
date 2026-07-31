import { invoke } from "@tauri-apps/api/core";
import { icon } from "./icons.js";
import "./agent-runtime-control-center.css";

const RUNTIME_ROUTE = "runtime";
const REFRESH_WINDOW_MS = 15_000;

const runtimeState = {
  runtime: null,
  authority: null,
  orchestration: null,
  scheduling: null,
  governance: null,
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

function supervisedCheckpoints() {
  return values(runtimeState.orchestration?.checkpoints);
}

function activeSupervisedCheckpoints() {
  return supervisedCheckpoints().filter((checkpoint) => ["awaiting_approval", "approved", "executing"].includes(checkpoint.state));
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

function safePreview(value) {
  try {
    const text = JSON.stringify(value ?? {});
    return text.length > 320 ? `${text.slice(0, 320)}…` : text;
  } catch {
    return "Safe preview unavailable";
  }
}

function renderSupervisedCheckpoints() {
  const checkpoints = supervisedCheckpoints();
  const receipts = new Map(values(runtimeState.orchestration?.receipts).map((receipt) => [receipt.checkpoint_id, receipt]));
  if (!checkpoints.length) {
    return `<section class="panel runtime-supervised-panel"><div class="panel-title"><div>${icon("shield", 18)}<div><h2>Supervised Actions</h2><p>Approval-gated Phase 16D checkpoints inside ordered Phase 17 plans.</p></div></div><span>0 checkpoints</span></div><div class="runtime-empty compact"><strong>No supervised checkpoints</strong><p>Approval-gated actions will pause here without entering the low-risk tool worker.</p></div></section>`;
  }
  return `<section class="panel runtime-supervised-panel"><div class="panel-title"><div>${icon("shield", 18)}<div><h2>Supervised Actions</h2><p>Safe previews, exact approval evidence, execution, and reversible compensation.</p></div></div><span>${activeSupervisedCheckpoints().length} active</span></div><div class="runtime-checkpoint-grid">${checkpoints.slice(0, 30).map((checkpoint) => {
    const receipt = receipts.get(checkpoint.checkpoint_id);
    const rollbackAvailable = checkpoint.state === "completed" && checkpoint.compensation_supported && checkpoint.compensation_state === "available";
    return `<article class="runtime-checkpoint-card ${statusTone(checkpoint.state)}"><header><div><span>Step ${Number(checkpoint.sequence_number || 0)} · ${escapeHtml(humanize(checkpoint.risk_class))}</span><h3>${escapeHtml(checkpoint.title)}</h3></div>${statusBadge(checkpoint.state)}</header><p>${escapeHtml(checkpoint.rationale)}</p><div class="runtime-safe-preview"><strong>Safe proposal preview</strong><code>${escapeHtml(safePreview(checkpoint.safe_summary))}</code></div><dl><div><dt>Proposal</dt><dd>${escapeHtml(humanize(checkpoint.proposal_state))}</dd></div><div><dt>Approval</dt><dd>${escapeHtml(humanize(checkpoint.approval_state || "not available"))}</dd></div><div><dt>Adapter</dt><dd>${escapeHtml(checkpoint.tool_adapter)}</dd></div><div><dt>Expires</dt><dd>${escapeHtml(formatDate(checkpoint.expires_at_utc))}</dd></div><div><dt>Plan hash</dt><dd class="mono">${escapeHtml(compactHash(checkpoint.runtime_plan_hash))}</dd></div><div><dt>Payload hash</dt><dd class="mono">${escapeHtml(compactHash(checkpoint.payload_hash))}</dd></div></dl><footer><span>${receipt ? `Receipt ${escapeHtml(compactHash(receipt.receipt_hash))}` : `Compensation ${escapeHtml(humanize(checkpoint.compensation_state))}`}</span><div>${["awaiting_approval", "approved"].includes(checkpoint.state) ? `<button class="button secondary" type="button" data-runtime-open-agent>Review approval</button>` : ""}${rollbackAvailable ? `<button class="button danger" type="button" data-runtime-rollback-checkpoint="${escapeHtml(checkpoint.checkpoint_id)}" data-runtime-checkpoint-title="${escapeHtml(checkpoint.title)}">Rollback</button>` : ""}</div></footer></article>`;
  }).join("")}</div></section>`;
}

function renderSchedules() {
  const schedules = values(runtimeState.scheduling?.schedules);
  const runs = values(runtimeState.scheduling?.runs);
  const receipts = values(runtimeState.scheduling?.receipts);
  if (!schedules.length) {
    return `<section class="panel runtime-schedules-panel"><div class="panel-title"><div>${icon("activity", 18)}<div><h2>Authorized Schedules</h2><p>Time and safe-event triggers create fresh Phase 17/18 plans only after exact authority revalidation.</p></div></div><span>0 schedules</span></div><div class="runtime-empty compact"><strong>No authorized schedules</strong><p>Paired wrappers and trusted local clients can submit authority-bound runtime-plan templates.</p></div></section>`;
  }
  const latestRun = new Map();
  for (const run of runs) if (!latestRun.has(run.schedule_id)) latestRun.set(run.schedule_id, run);
  return `<section class="panel runtime-schedules-panel"><div class="panel-title"><div>${icon("activity", 18)}<div><h2>Authorized Schedules</h2><p>Misfire, overlap, debounce, event cursors, and immutable Phase 17/18 plan-creation evidence.</p></div></div><span>${schedules.filter((schedule) => schedule.state === "active").length} active</span></div><div class="runtime-schedule-grid">${schedules.slice(0, 30).map((schedule) => {
    const run = latestRun.get(schedule.schedule_id);
    const timing = schedule.trigger_kind === "event"
      ? `${humanize(schedule.event_topic)}${schedule.event_source_id ? ` · ${schedule.event_source_id}` : ""}`
      : schedule.next_fire_at_utc
        ? `Next ${formatDate(schedule.next_fire_at_utc)}`
        : "No future trigger";
    return `<article class="runtime-schedule-card ${statusTone(schedule.state)}"><header><div><span>${escapeHtml(humanize(schedule.trigger_kind))}</span><h3>${escapeHtml(schedule.title)}</h3></div>${statusBadge(schedule.state)}</header><p>${escapeHtml(schedule.description || "Authority-bound runtime plan schedule")}</p><dl><div><dt>Trigger</dt><dd>${escapeHtml(timing)}</dd></div><div><dt>Runs</dt><dd>${Number(schedule.run_count || 0)} / ${Number(schedule.max_runs || 0)}</dd></div><div><dt>Misfire</dt><dd>${escapeHtml(humanize(schedule.misfire_policy))}</dd></div><div><dt>Overlap</dt><dd>${escapeHtml(humanize(schedule.overlap_policy))}</dd></div><div><dt>Authority</dt><dd class="mono">${escapeHtml(compactHash(schedule.authority_hash))}</dd></div><div><dt>Template</dt><dd class="mono">${escapeHtml(compactHash(schedule.template_hash))}</dd></div></dl><footer><span>${run ? `${escapeHtml(humanize(run.state))} · ${escapeHtml(humanize(run.result_code || run.failure_code || "pending"))}` : `${receipts.filter((receipt) => receipt.schedule_id === schedule.schedule_id).length} receipts`}</span><div>${schedule.state === "active" ? `<button class="button secondary" type="button" data-schedule-pause="${escapeHtml(schedule.schedule_id)}" data-schedule-title="${escapeHtml(schedule.title)}">Pause</button>` : ""}${schedule.state === "paused" ? `<button class="button secondary" type="button" data-schedule-resume="${escapeHtml(schedule.schedule_id)}" data-schedule-title="${escapeHtml(schedule.title)}">Resume</button>` : ""}${["active", "paused", "failed"].includes(schedule.state) ? `<button class="button danger" type="button" data-schedule-cancel="${escapeHtml(schedule.schedule_id)}" data-schedule-title="${escapeHtml(schedule.title)}">Cancel</button>` : ""}</div></footer></article>`;
  }).join("")}</div><div class="runtime-schedule-boundary"><strong>Phase 17/18 plan creation only</strong><span>Private templates and full event payloads remain hidden. The scheduler cannot invoke tools, proposals, approvals, or result egress directly.</span></div></section>`;
}

function renderReceipts() {
  const receipts = values(runtimeState.runtime?.receipts);
  if (!receipts.length) return `<div class="runtime-empty compact"><strong>No runtime receipts</strong><p>Immutable per-step evidence will appear after work completes or fails.</p></div>`;
  return `<div class="runtime-receipt-list">${receipts.slice(0, 30).map((receipt) => `<div><span class="runtime-receipt-icon">${icon(receipt.outcome === "success" ? "check" : "warning", 17)}</span><p><strong>${escapeHtml(receipt.tool_key)}</strong><small>${escapeHtml(humanize(receipt.result_code))} · ${escapeHtml(relativeDate(receipt.completed_at_utc))}</small></p>${statusBadge(receipt.outcome)}<code title="Runtime receipt hash">${escapeHtml(compactHash(receipt.runtime_receipt_hash))}</code></div>`).join("")}</div>`;
}


function renderModelGovernance() {
  const governance = runtimeState.governance || {};
  const policies = values(governance.policies);
  const requests = values(governance.requests);
  const receipts = values(governance.receipts);
  const activeRequests = requests.filter((request) => ["reserved", "running"].includes(request.state));
  const boundarySafe = governance.private_prompts_exposed === false
    && governance.private_results_exposed === false
    && governance.silent_remote_fallback_allowed === false
    && governance.provider_can_grant_authority === false;
  return `<section class="panel runtime-model-governance ${boundarySafe ? "safe" : "unsafe"}">
    <div class="panel-title"><div>${icon("cpu", 18)}<div><h2>Model Inference Governance</h2><p>Exact policy, provider, model, privacy, fallback, and budget authority for every inference.</p></div></div><div class="runtime-model-actions"><span>${policies.filter((policy) => policy.state === "active").length} active policies</span><button class="button secondary" type="button" data-model-policy-create>Create policy</button></div></div>
    <div class="runtime-safety-grid">
      ${safetyItem("Private prompts", governance.private_prompts_exposed === false ? "Hash-only snapshots" : "Exposure detected", governance.private_prompts_exposed === false)}
      ${safetyItem("Private results", governance.private_results_exposed === false ? "Local private table" : "Exposure detected", governance.private_results_exposed === false)}
      ${safetyItem("Silent remote fallback", governance.silent_remote_fallback_allowed === false ? "Prohibited" : "Allowed", governance.silent_remote_fallback_allowed === false)}
      ${safetyItem("Provider authority", governance.provider_can_grant_authority === false ? "None" : "Detected", governance.provider_can_grant_authority === false)}
    </div>
    <div class="runtime-model-grid">
      <div><h3>Routing policies</h3>${policies.length ? policies.slice(0, 20).map((policy) => `<article class="runtime-model-policy"><header><div><strong>${escapeHtml(policy.purpose)}</strong><small>revision ${Number(policy.policy_revision || 0)} · ${escapeHtml(humanize(policy.subject_type))}</small></div>${statusBadge(policy.state)}</header><p>${escapeHtml(values(policy.provider_order).join(" → ") || "No providers")} · ${escapeHtml(humanize(policy.remote_context_mode))}${policy.allow_fallback ? " · fallback enabled" : " · no fallback"}</p><dl><div><dt>Input</dt><dd>${Number(policy.max_input_chars || 0)} chars</dd></div><div><dt>Output</dt><dd>${Number(policy.max_output_tokens || 0)} tokens</dd></div><div><dt>Requests</dt><dd>${Number(policy.max_requests || 0)} / ${escapeHtml(humanize(String(policy.window_seconds || 0)))}s</dd></div><div><dt>Authority</dt><dd class="mono">${escapeHtml(compactHash(policy.policy_hash))}</dd></div></dl>${policy.state === "active" ? `<button class="button danger" type="button" data-model-policy-revoke="${escapeHtml(policy.policy_id)}">Revoke</button>` : ""}</article>`).join("") : `<div class="runtime-empty compact"><strong>No routing policies</strong><p>Inference fails closed until an exact policy exists.</p></div>`}</div>
      <div><h3>Requests and receipts</h3>${activeRequests.map((request) => `<article class="runtime-model-request"><header><strong>${escapeHtml(humanize(request.data_classification))}</strong>${statusBadge(request.state)}</header><p>${escapeHtml(values(request.provider_order).join(" → "))} · ${escapeHtml(request.selected_model || request.requested_model || "model pending")}</p><code>${escapeHtml(compactHash(request.authority_hash))}</code><button class="button danger" type="button" data-model-inference-cancel="${escapeHtml(request.request_id)}">Cancel</button></article>`).join("")}${receipts.slice(0, 20).map((receipt) => `<article class="runtime-model-receipt"><header><strong>${escapeHtml(receipt.model_id || "No model selected")}</strong>${statusBadge(receipt.outcome)}</header><p>${escapeHtml(receipt.provider_key || "no provider")} · ${escapeHtml(humanize(receipt.result_code))} · ${Number(receipt.total_tokens || 0)} tokens</p><code title="Inference receipt hash">${escapeHtml(compactHash(receipt.receipt_hash))}</code></article>`).join("")}${!activeRequests.length && !receipts.length ? `<div class="runtime-empty compact"><strong>No governed inference evidence</strong><p>Policies are ready; completed or failed inference receipts will appear here.</p></div>` : ""}</div>
    </div>
  </section>`;
}

function renderRuntimePage() {
  if (!isRuntimeRoute()) return;
  ensureRuntimeNavigation();
  const canvas = document.querySelector(".page-canvas");
  if (!canvas) return;
  const runtime = runtimeState.runtime || {};
  const orchestration = runtimeState.orchestration || {};
  const scheduling = runtimeState.scheduling || {};
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
        ${metric("shield", "Pending Approvals", String(pendingApprovals().length), `${activeSupervisedCheckpoints().length} supervised checkpoints`, pendingApprovals().length ? "amber" : "green")}
        ${metric("warning", "Failures", String(failedWorkCount()), `${activeStops().length} active emergency stops`, failedWorkCount() || activeStops().length ? "amber" : "green")}
        ${metric("logs", "Receipts", String(receipts.length), `${tools.length} cataloged tools`, "teal")}
      </section>
      ${renderSafetyBoundary(runtime)}
      ${renderModelGovernance()}
      ${renderSupervisedCheckpoints(orchestration)}
      ${renderSchedules(scheduling)}
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
    invoke("homeserver_action_orchestration"),
    invoke("homeserver_agent_schedules"),
    invoke("homeserver_model_governance"),
  ]);
  if (results[0].status === "fulfilled") runtimeState.runtime = results[0].value;
  if (results[1].status === "fulfilled") runtimeState.authority = results[1].value;
  if (results[2].status === "fulfilled") runtimeState.orchestration = results[2].value;
  if (results[3].status === "fulfilled") runtimeState.scheduling = results[3].value;
  if (results[4].status === "fulfilled") runtimeState.governance = results[4].value;
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
    await invoke("homeserver_run_agent_runtime_once");
    runtimeState.orchestration = await invoke("homeserver_run_action_orchestration_once");
    runtimeState.scheduling = await invoke("homeserver_run_agent_scheduler_once");
    runtimeState.runtime = await invoke("homeserver_agent_runtime");
    runtimeState.authority = await invoke("homeserver_agent_authority");
    runtimeState.governance = await invoke("homeserver_model_governance");
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

async function mutateSchedule(button, action) {
  if (runtimeState.busy) return;
  const attribute = `schedule${action[0].toUpperCase()}${action.slice(1)}`;
  const scheduleId = button.dataset[attribute] || "";
  const title = button.dataset.scheduleTitle || "this schedule";
  const command = `${action.toUpperCase()} SCHEDULE ${scheduleId}`;
  const confirmation = window.prompt(`Type ${command} to ${action} ${title}:`);
  if (confirmation !== command) return;
  const reason = window.prompt(`Reason to ${action} this schedule:`, `${humanize(action)} from Agent Runtime Control Center`);
  if (reason === null || !reason.trim()) return;
  runtimeState.busy = true;
  runtimeState.error = null;
  renderRuntimePage();
  try {
    runtimeState.scheduling = await invoke(`homeserver_${action}_agent_schedule`, {
      scheduleId,
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

async function rollbackSupervisedCheckpoint(button) {
  if (runtimeState.busy) return;
  const checkpointId = button.dataset.runtimeRollbackCheckpoint || "";
  const title = button.dataset.runtimeCheckpointTitle || "this action";
  const confirmation = window.prompt(`Type ROLLBACK ACTION ${checkpointId} to compensate ${title}:`);
  if (confirmation !== `ROLLBACK ACTION ${checkpointId}`) return;
  const reason = window.prompt("Reason for rollback:", "Rolled back from Agent Runtime Control Center");
  if (reason === null || !reason.trim()) return;
  runtimeState.busy = true;
  runtimeState.error = null;
  renderRuntimePage();
  try {
    runtimeState.orchestration = await invoke("homeserver_rollback_supervised_action", {
      checkpointId,
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


async function createModelPolicy() {
  if (runtimeState.busy) return;
  const subjectType = window.prompt("Policy subject: local_control_center or agent_assignment", "local_control_center");
  if (!subjectType) return;
  let agentId = null;
  let assignmentId = null;
  if (subjectType === "agent_assignment") {
    agentId = window.prompt("Agent ID:");
    assignmentId = window.prompt("Assignment ID:");
    if (!agentId || !assignmentId) return;
  }
  const purpose = window.prompt("Exact inference purpose:", "agent_workspace");
  if (!purpose) return;
  const providers = window.prompt("Ordered providers, comma separated:", "ollama");
  if (!providers) return;
  const providerOrder = providers.split(",").map((value) => value.trim()).filter(Boolean);
  const remote = providerOrder.includes("openrouter");
  const budget = remote ? Number(window.prompt("Maximum spend in micro-USD for the policy window:", "1000000")) : 0;
  const policy = {
    subject_type: subjectType,
    agent_id: agentId,
    assignment_id: assignmentId,
    purpose,
    allowed_data_classes: ["public", "safe_receipt", "security_metadata", "private_derived"],
    provider_order: providerOrder,
    allowed_models: [],
    allow_fallback: providerOrder.length > 1,
    remote_context_mode: remote ? "public_only" : "deny",
    require_zdr: true,
    max_input_chars: 30000,
    max_output_tokens: 1024,
    window_seconds: 86400,
    max_requests: 10000,
    max_total_tokens: 10000000,
    max_spend_microusd: Number.isFinite(budget) ? Math.max(0, budget) : 0,
    reason: "Created from Agent Runtime Control Center",
    expires_minutes: 525600,
  };
  runtimeState.busy = true;
  renderRuntimePage();
  try {
    runtimeState.governance = await invoke("homeserver_create_model_policy", { policy });
    runtimeState.lastLoadedAt = Date.now();
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    renderRuntimePage();
  }
}

async function revokeModelPolicy(button) {
  if (runtimeState.busy) return;
  const policyId = button.dataset.modelPolicyRevoke || "";
  const confirmation = window.prompt(`Type REVOKE MODEL POLICY ${policyId} to revoke this policy:`);
  if (confirmation !== `REVOKE MODEL POLICY ${policyId}`) return;
  const reason = window.prompt("Reason for revocation:", "Revoked from Agent Runtime Control Center");
  if (!reason?.trim()) return;
  runtimeState.busy = true;
  renderRuntimePage();
  try {
    runtimeState.governance = await invoke("homeserver_revoke_model_policy", { policyId, confirmation, reason: reason.trim() });
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    renderRuntimePage();
  }
}

async function cancelModelInference(button) {
  if (runtimeState.busy) return;
  const requestId = button.dataset.modelInferenceCancel || "";
  const confirmation = window.prompt(`Type CANCEL INFERENCE ${requestId} to cancel this request:`);
  if (confirmation !== `CANCEL INFERENCE ${requestId}`) return;
  const reason = window.prompt("Reason for cancellation:", "Cancelled from Agent Runtime Control Center");
  if (!reason?.trim()) return;
  runtimeState.busy = true;
  renderRuntimePage();
  try {
    runtimeState.governance = await invoke("homeserver_cancel_model_inference", { requestId, confirmation, reason: reason.trim() });
  } catch (error) {
    runtimeState.error = String(error);
  } finally {
    runtimeState.busy = false;
    renderRuntimePage();
  }
}

document.addEventListener("click", (event) => {
  if (!(event.target instanceof Element)) return;
  const createPolicy = event.target.closest("[data-model-policy-create]");
  if (createPolicy) {
    event.preventDefault();
    void createModelPolicy();
    return;
  }
  const revokePolicy = event.target.closest("[data-model-policy-revoke]");
  if (revokePolicy) {
    event.preventDefault();
    void revokeModelPolicy(revokePolicy);
    return;
  }
  const cancelInference = event.target.closest("[data-model-inference-cancel]");
  if (cancelInference) {
    event.preventDefault();
    void cancelModelInference(cancelInference);
    return;
  }
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
  const rollbackButton = event.target.closest("[data-runtime-rollback-checkpoint]");
  if (rollbackButton) {
    event.preventDefault();
    void rollbackSupervisedCheckpoint(rollbackButton);
    return;
  }
  const pauseSchedule = event.target.closest("[data-schedule-pause]");
  if (pauseSchedule) {
    event.preventDefault();
    void mutateSchedule(pauseSchedule, "pause");
    return;
  }
  const resumeSchedule = event.target.closest("[data-schedule-resume]");
  if (resumeSchedule) {
    event.preventDefault();
    void mutateSchedule(resumeSchedule, "resume");
    return;
  }
  const cancelSchedule = event.target.closest("[data-schedule-cancel]");
  if (cancelSchedule) {
    event.preventDefault();
    void mutateSchedule(cancelSchedule, "cancel");
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
