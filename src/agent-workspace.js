import { invoke } from "@tauri-apps/api/core";
import "./agent-workspace.css";

const PAGE_KEY = "agent";
let snapshot = null;
let loading = false;
let actionBusy = false;
let notice = null;
let activeThreadId = null;
let activeTab = "goals";
let modal = null;
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

function formatDate(value) {
  if (!value) return "Not yet";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function compactId(value) {
  const text = String(value || "");
  if (text.length <= 18) return text || "Not assigned";
  return `${text.slice(0, 8)}…${text.slice(-7)}`;
}

function statusClass(value) {
  return String(value || "unknown").toLowerCase().replaceAll("_", "-");
}

function statusBadge(value) {
  return `<span class="agent-status-badge ${escapeHtml(statusClass(value))}">${escapeHtml(humanize(value))}</span>`;
}

function selectedValues(selector) {
  return [...document.querySelectorAll(selector)]
    .filter((input) => input.checked)
    .map((input) => input.value);
}

function currentHash() {
  return window.location.hash.replace("#", "");
}

function isAgentPage() {
  return currentHash() === PAGE_KEY;
}

function connections() {
  return Array.isArray(snapshot?.connections) ? snapshot.connections : [];
}

function goals() {
  return Array.isArray(snapshot?.goals) ? snapshot.goals : [];
}

function plans() {
  return Array.isArray(snapshot?.plans) ? snapshot.plans : [];
}

function approvals() {
  return Array.isArray(snapshot?.approvals) ? snapshot.approvals : [];
}

function missions() {
  return Array.isArray(snapshot?.missions) ? snapshot.missions : [];
}

function threads() {
  return Array.isArray(snapshot?.threads) ? snapshot.threads : [];
}

function activeThread() {
  return threads().find((thread) => thread.thread_id === activeThreadId) || threads()[0] || null;
}

function activeMessages() {
  const thread = activeThread();
  if (!thread) return [];
  return (snapshot?.messages || []).filter((message) => message.thread_id === thread.thread_id);
}

function ensureActiveThread() {
  if (!activeThreadId || !threads().some((thread) => thread.thread_id === activeThreadId)) {
    activeThreadId = threads()[0]?.thread_id || null;
  }
}

function injectNavigation() {
  const nav = document.querySelector(".primary-nav");
  if (!nav || nav.querySelector('[data-agent-workspace-nav="true"]')) return;
  const button = document.createElement("button");
  button.type = "button";
  button.className = `nav-item agent-navigation-item ${isAgentPage() ? "active" : ""}`;
  button.dataset.agentWorkspaceNav = "true";
  button.innerHTML = `<span aria-hidden="true">✦</span><span>Agent Workspace</span>`;
  button.addEventListener("click", () => {
    window.location.hash = `#${PAGE_KEY}`;
    window.setTimeout(() => mount(true), 0);
  });
  nav.prepend(button);
}

function renderThreadList() {
  if (!threads().length) {
    return '<div class="agent-empty-records">Your first prompt creates a private local conversation.</div>';
  }
  return threads().map((thread) => `<button type="button" class="agent-thread-button ${thread.thread_id === activeThreadId ? "active" : ""}" data-agent-thread="${escapeHtml(thread.thread_id)}"><span>${escapeHtml(thread.title)}</span><small>${escapeHtml(formatDate(thread.updated_at_utc).split(",")[0])}</small></button>`).join("");
}

function renderMessages() {
  const messages = activeMessages();
  if (!messages.length) {
    return `<div class="agent-chat-empty"><div><strong>Talk to your HomeServer</strong><p>Ask about current system data, connected sites, authorized operational datasets, goals, models, or the Knowledge Vault.</p></div></div>`;
  }
  return messages.map((message) => `<article class="agent-message ${escapeHtml(message.role)}"><div class="agent-message-avatar">${message.role === "user" ? "YOU" : "HS"}</div><div class="agent-message-body"><div class="agent-message-meta">${statusBadge(message.mode)}<span>${escapeHtml(formatDate(message.created_at_utc))}</span></div><div class="agent-message-content">${escapeHtml(message.content)}</div></div></article>`).join("");
}

function renderDataSources() {
  const sources = Array.isArray(snapshot?.data_sources) ? snapshot.data_sources : [];
  return sources.map((source) => {
    const isOperationalDataset = String(source.key || "").startsWith("dataset:");
    const isConnection = Boolean(source.connection_id) && !isOperationalDataset;
    const value = isConnection ? source.connection_id : source.key;
    const name = isConnection ? "agent-connection-source" : "agent-dataset-source";
    const checked = ["system", "connections", "knowledge", "goals"].includes(source.key) || isConnection;
    const disabled = ["planned_phase_5c", "paused", "not_granted"].includes(source.state);
    return `<label class="agent-source-row"><input type="checkbox" name="${name}" value="${escapeHtml(value)}" ${checked && !disabled ? "checked" : ""} ${disabled ? "disabled" : ""}><span><strong>${escapeHtml(source.label)}</strong><small>${escapeHtml(source.detail)}</small></span><em class="agent-source-state">${escapeHtml(humanize(source.state))}</em></label>`;
  }).join("");
}

function goalOptions(includeEmpty = true) {
  const active = goals().filter((goal) => goal.state === "active");
  return `${includeEmpty ? '<option value="">No saved goal</option>' : ""}${active.map((goal) => `<option value="${escapeHtml(goal.goal_id)}">${escapeHtml(goal.title)}</option>`).join("")}`;
}

function connectionOptions(includeEmpty = true) {
  return `${includeEmpty ? '<option value="">Local-only</option>' : ""}${connections().map((connection) => `<option value="${escapeHtml(connection.connection_id)}">${escapeHtml(connection.display_name)} · ${escapeHtml(connection.site_id || connection.tenant_id || "provider-managed")}</option>`).join("")}`;
}

function modelOptions() {
  const defaultModel = snapshot?.default_chat_model;
  return `<option value="">HomeServer default${defaultModel ? ` · ${escapeHtml(defaultModel)}` : ""}</option>`;
}

function renderTabs() {
  const tabs = [
    ["goals", "Goals", goals().filter((goal) => goal.state === "active").length],
    ["approvals", "Approvals", approvals().filter((approval) => approval.state === "pending").length],
    ["plans", "Plans", plans().length],
    ["missions", "World Missions", missions().length],
    ["reports", "Reports", (snapshot?.reports || []).length],
    ["receipts", "Receipts", (snapshot?.receipts || []).length],
  ];
  return tabs.map(([key, label, count]) => `<button type="button" class="agent-workspace-tab ${activeTab === key ? "active" : ""}" data-agent-tab="${key}"><span>${label}</span><small>${count}</small></button>`).join("");
}

function renderGoalCards() {
  if (!goals().length) return '<div class="agent-empty-records">No saved goals yet.</div>';
  return goals().map((goal) => `<article class="agent-record-card"><div class="agent-record-meta">${statusBadge(goal.state)}<span>${escapeHtml(goal.target_metric || "No target metric")}</span><span>${escapeHtml(goal.target_value || "No target value")}</span></div><h3>${escapeHtml(goal.title)}</h3><p>${escapeHtml(goal.description || "No description")}</p><div class="agent-record-detail">Data: ${escapeHtml((goal.dataset_keys || []).join(", ") || "Not selected")}<br>Connections: ${Number((goal.connection_ids || []).length)} · Approval: ${escapeHtml(humanize(goal.approval_policy))}</div>${goal.state !== "archived" ? `<div class="agent-inline-actions"><button class="button ghost danger" type="button" data-agent-archive-goal="${escapeHtml(goal.goal_id)}">Archive</button></div>` : ""}</article>`).join("");
}

function planById(planId) {
  return plans().find((plan) => plan.plan_id === planId);
}

function renderApprovalCards() {
  const pendingOrRecent = approvals();
  if (!pendingOrRecent.length) return '<div class="agent-empty-records">No approval requests.</div>';
  return pendingOrRecent.map((approval) => {
    const plan = planById(approval.plan_id);
    return `<article class="agent-record-card"><div class="agent-record-meta">${statusBadge(approval.state)}<span>Expires ${escapeHtml(formatDate(approval.expires_at_utc))}</span><span>${escapeHtml(compactId(approval.plan_hash))}</span></div><h3>${escapeHtml(plan?.title || "Supervised plan")}</h3><p>${escapeHtml(approval.risk_summary)}</p><div class="agent-record-detail">Action: ${escapeHtml(humanize(plan?.action_type))}<br>Target: ${escapeHtml(plan?.connection_id ? compactId(plan.connection_id) : "Local HomeServer")}<br>Requested by: ${escapeHtml(humanize(plan?.requested_by_type))}</div><div class="agent-plan-actions">${approval.state === "pending" ? `<button class="button primary" type="button" data-agent-approve="${escapeHtml(approval.plan_id)}">Approve</button><button class="button danger" type="button" data-agent-reject="${escapeHtml(approval.plan_id)}">Reject</button>` : ""}${plan?.state === "approved" ? `<button class="button primary" type="button" data-agent-execute="${escapeHtml(approval.plan_id)}">Execute Once</button>` : ""}</div></article>`;
  }).join("");
}

function renderPlanCards() {
  if (!plans().length) return '<div class="agent-empty-records">No supervised plans.</div>';
  return plans().map((plan) => `<article class="agent-record-card"><div class="agent-record-meta">${statusBadge(plan.state)}<span>${escapeHtml(humanize(plan.risk_level))} risk</span><span>${escapeHtml(compactId(plan.plan_hash))}</span></div><h3>${escapeHtml(plan.title)}</h3><p>${escapeHtml(plan.rationale)}</p><div class="agent-record-detail">Action: ${escapeHtml(humanize(plan.action_type))}<br>Connection: ${escapeHtml(plan.connection_id ? compactId(plan.connection_id) : "Local-only")}<br>Expires: ${escapeHtml(formatDate(plan.expires_at_utc))}</div><div class="agent-plan-actions">${["draft", "awaiting_approval", "approved"].includes(plan.state) ? `<button class="button ghost danger" type="button" data-agent-cancel-plan="${escapeHtml(plan.plan_id)}">Cancel</button>` : ""}${plan.state === "approved" ? `<button class="button primary" type="button" data-agent-execute="${escapeHtml(plan.plan_id)}">Execute Once</button>` : ""}</div></article>`).join("");
}

function renderMissionCards() {
  if (!missions().length) return '<div class="agent-empty-records">No World Mission drafts.</div>';
  return missions().map((mission) => `<article class="agent-record-card"><div class="agent-record-meta">${statusBadge(mission.state)}<span>${escapeHtml(mission.world_agent_id)}</span><span>Expires ${escapeHtml(formatDate(mission.expires_at_utc))}</span></div><h3>${escapeHtml(mission.title)}</h3><p>${escapeHtml(mission.objective)}</p><div class="agent-record-detail">Allowed: ${escapeHtml((mission.allowed_operations || []).join(", "))}<br>Prohibited: ${escapeHtml((mission.prohibited_operations || []).join(", "))}<br>Dispatch: not installed in this Phase 5B slice</div>${["draft", "awaiting_approval", "ready_for_dispatch"].includes(mission.state) ? `<div class="agent-inline-actions"><button class="button ghost danger" type="button" data-agent-cancel-mission="${escapeHtml(mission.mission_id)}">Cancel Draft</button></div>` : ""}</article>`).join("");
}

function renderReportCards() {
  const reports = snapshot?.reports || [];
  if (!reports.length) return '<div class="agent-empty-records">No saved operational reports.</div>';
  return reports.map((report) => `<article class="agent-record-card"><div class="agent-record-meta"><span>${escapeHtml(formatDate(report.created_at_utc))}</span>${report.plan_id ? `<span>Plan ${escapeHtml(compactId(report.plan_id))}</span>` : ""}</div><h3>${escapeHtml(report.title)}</h3><p>${escapeHtml(report.content_markdown.slice(0, 360))}${report.content_markdown.length > 360 ? "…" : ""}</p><div class="agent-record-detail">Datasets: ${escapeHtml((report.dataset_keys || []).join(", ") || "Local context")}</div></article>`).join("");
}

function renderReceiptCards() {
  const receipts = snapshot?.receipts || [];
  if (!receipts.length) return '<div class="agent-empty-records">No execution receipts.</div>';
  return receipts.map((receipt) => `<article class="agent-record-card"><div class="agent-record-meta">${statusBadge(receipt.state)}<span>${escapeHtml(formatDate(receipt.completed_at_utc))}</span><span>${escapeHtml(receipt.result_code)}</span></div><h3>${escapeHtml(humanize(receipt.action_type))}</h3><p>${escapeHtml(receipt.result_summary)}</p><div class="agent-record-detail">Receipt: ${escapeHtml(compactId(receipt.receipt_id))}<br>Plan hash: ${escapeHtml(compactId(receipt.plan_hash))}<br>Idempotency: ${escapeHtml(compactId(receipt.idempotency_key))}</div></article>`).join("");
}

function renderActiveTab() {
  const content = {
    goals: renderGoalCards,
    approvals: renderApprovalCards,
    plans: renderPlanCards,
    missions: renderMissionCards,
    reports: renderReportCards,
    receipts: renderReceiptCards,
  }[activeTab]?.() || "";
  const title = {
    goals: "Saved Goals",
    approvals: "Local Approval Inbox",
    plans: "Supervised Plans",
    missions: "World Mission Drafts",
    reports: "Operational Reports",
    receipts: "Execution Receipts",
  }[activeTab];
  const action = activeTab === "goals" ? '<button type="button" class="button primary" data-agent-open-modal="goal">New Goal</button>' : activeTab === "plans" ? '<button type="button" class="button secondary" data-agent-open-modal="plan">Draft Plan</button>' : activeTab === "missions" ? '<button type="button" class="button secondary" data-agent-open-modal="mission">Draft World Mission</button>' : "";
  return `<section class="agent-workspace-section full"><div class="agent-workspace-section-head"><h2>${title}</h2>${action}</div><div class="agent-card-list">${content}</div></section>`;
}

function renderModal() {
  if (!modal) return "";
  if (modal === "goal") {
    return `<div class="agent-modal-backdrop" data-agent-close-modal><div class="agent-modal" role="dialog" aria-modal="true" aria-label="Create goal"><div class="agent-modal-head"><div><h2>Create a Goal</h2><p>Match current and future connected datasets against a measurable objective.</p></div><button type="button" class="icon-button" data-agent-close-modal>×</button></div><form class="agent-form" id="agent-goal-form"><label>Goal title<input id="agent-goal-title" maxlength="160" required placeholder="Increase weekday lunch revenue"></label><label>Description<textarea id="agent-goal-description" maxlength="4000" placeholder="Describe the desired operational result"></textarea></label><div class="agent-form-grid"><label>Target metric<input id="agent-goal-metric" maxlength="160" placeholder="Weekday lunch revenue"></label><label>Target value<input id="agent-goal-value" maxlength="160" placeholder="+15%"></label></div><div class="agent-form-grid"><label>Target date<input id="agent-goal-date" type="date"></label><label>Approval policy<select id="agent-goal-policy"><option value="always">Always approve actions</option><option value="read_only">Read-only analysis</option><option value="disabled">No agent actions</option></select></label></div><p class="agent-form-help">The goal is local. Connected platforms remain authoritative for their operational records.</p><div class="agent-inline-actions"><button type="submit" class="button primary" ${actionBusy ? "disabled" : ""}>Save Goal</button><button type="button" class="button ghost" data-agent-close-modal>Cancel</button></div></form></div></div>`;
  }
  if (modal === "plan") {
    return `<div class="agent-modal-backdrop" data-agent-close-modal><div class="agent-modal" role="dialog" aria-modal="true" aria-label="Draft supervised plan"><div class="agent-modal-head"><div><h2>Draft a Supervised Plan</h2><p>Nothing executes until the exact plan receives local approval.</p></div><button type="button" class="icon-button" data-agent-close-modal>×</button></div><form class="agent-form" id="agent-plan-form"><label>Plan title<input id="agent-plan-title" maxlength="180" required placeholder="Synchronize the Downtown Restaurant connection"></label><label>Why this is needed<textarea id="agent-plan-rationale" maxlength="4000" required placeholder="Explain the evidence and expected outcome"></textarea></label><div class="agent-form-grid"><label>Bounded action<select id="agent-plan-action"><option value="backup.create">Create encrypted backup</option><option value="model.health_test">Run local-model health test</option><option value="cloud.sync_connection">Synchronize one connection</option><option value="cloud.sync_all">Synchronize all active connections</option><option value="report.save">Save local report</option></select></label><label>Target connection<select id="agent-plan-connection">${connectionOptions(true)}</select></label></div><label>Goal<select id="agent-plan-goal">${goalOptions(true)}</select></label><div id="agent-plan-extra-fields"></div><p class="agent-form-help">Cloud actions reuse the current signed allowlisted sync contract. Commerce and CRM writes are not enabled.</p><div class="agent-inline-actions"><button type="submit" class="button primary" ${actionBusy ? "disabled" : ""}>Create Approval Request</button><button type="button" class="button ghost" data-agent-close-modal>Cancel</button></div></form></div></div>`;
  }
  return `<div class="agent-modal-backdrop" data-agent-close-modal><div class="agent-modal" role="dialog" aria-modal="true" aria-label="Draft World Mission"><div class="agent-modal-head"><div><h2>Draft a World Mission</h2><p>Create a bounded objective for a future World Agent. Dispatch is not enabled yet.</p></div><button type="button" class="icon-button" data-agent-close-modal>×</button></div><form class="agent-form" id="agent-mission-form"><div class="agent-form-grid"><label>Mission title<input id="agent-mission-title" maxlength="180" required placeholder="Investigate weekday group dining"></label><label>World Agent ID<input id="agent-mission-agent" maxlength="160" required placeholder="david-customer-avatar"></label></div><label>Objective<textarea id="agent-mission-objective" maxlength="4000" required placeholder="Visit qualifying Store Canvases, ask about group dining, and return recommendations"></textarea></label><div class="agent-form-grid"><label>Microgifter connection<select id="agent-mission-connection">${connectionOptions(true)}</select></label><label>Goal<select id="agent-mission-goal">${goalOptions(true)}</select></label></div><div class="agent-form-grid"><label>Maximum visits<input id="agent-mission-visits" type="number" min="1" max="25" value="5"></label><label>Maximum messages<input id="agent-mission-messages" type="number" min="1" max="100" value="10"></label></div><div class="agent-form-grid"><label>Distance limit, miles<input id="agent-mission-distance" type="number" min="1" max="100" value="8"></label><label>Expires after, minutes<input id="agent-mission-expiry" type="number" min="15" max="10080" value="240"></label></div><p class="agent-form-help">Allowed by default: discover, compare, prepare recommendations. Purchases, payments, claims, private-profile sharing, recurring commitments, campaign publishing, and bulk messages remain prohibited.</p><div class="agent-inline-actions"><button type="submit" class="button primary" ${actionBusy ? "disabled" : ""}>Save Mission Draft</button><button type="button" class="button ghost" data-agent-close-modal>Cancel</button></div></form></div></div>`;
}

function renderPage() {
  ensureActiveThread();
  const thread = activeThread();
  const pendingCount = approvals().filter((approval) => approval.state === "pending").length;
  return `<div class="agent-workspace-page" data-agent-workspace-mounted="true"><header class="agent-workspace-header"><div><h1>Agent Workspace</h1><p>Talk to your private HomeServer agent, select connected context, match evidence to saved goals, create supervised plans, and draft World Missions.</p></div><div class="agent-workspace-header-actions"><span class="agent-mode-badge">${escapeHtml(humanize(snapshot?.model_runtime_state || "loading"))} model runtime</span><button type="button" class="button secondary" id="agent-workspace-refresh" ${loading || actionBusy ? "disabled" : ""}>Refresh</button></div></header>${notice ? `<div class="agent-workspace-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}<section class="agent-workspace-boundary"><article class="agent-boundary-card"><strong>Private analytical authority</strong><span>HomeServer can use local goals, models, system state, connection metadata, Knowledge Vault context, and explicitly authorized operational evidence.</span></article><article class="agent-boundary-card"><strong>Supervised execution</strong><span>${pendingCount} pending approval${pendingCount === 1 ? "" : "s"}. MCP clients can request work but cannot approve or execute it.</span></article><article class="agent-boundary-card"><strong>World Mode boundary</strong><span>World Mission drafting is enabled. World Agent dispatch and interactive conversation operation are reserved for a later scoped phase.</span></article></section><section class="agent-workspace-shell"><aside class="agent-workspace-sidebar"><div class="agent-workspace-sidebar-head"><h2>Conversations</h2><button class="button ghost" type="button" id="agent-new-thread">New</button></div><div class="agent-thread-list">${renderThreadList()}</div><div class="agent-sidebar-divider"></div><div class="agent-workspace-tab-list">${renderTabs()}</div><div class="agent-sidebar-footer">Connected platforms remain authoritative. HomeServer stores local context, plans, approvals, reports, missions, and receipts.</div></aside><main class="agent-workspace-main"><div class="agent-chat-head"><div><strong>${escapeHtml(thread?.title || "New HomeServer conversation")}</strong><span>${thread ? `Thread ${escapeHtml(compactId(thread.thread_id))}` : "A thread will be created with your first prompt"}</span></div><span class="agent-mode-badge">Prompt-first control plane</span></div><div class="agent-chat-stream" id="agent-chat-stream">${renderMessages()}</div><form class="agent-composer" id="agent-prompt-form"><div class="agent-composer-top"><select id="agent-prompt-mode" aria-label="Agent mode"><option value="ask">Ask</option><option value="analyze">Analyze</option><option value="plan">Plan</option><option value="dispatch">Dispatch Draft</option><option value="execute">Execute Request</option></select><select id="agent-prompt-goal" aria-label="Goal">${goalOptions(true)}</select><select id="agent-prompt-model" aria-label="Model">${modelOptions()}</select></div><div class="agent-composer-context"><label class="agent-chip"><input type="checkbox" name="agent-inline-dataset" value="system" checked>System</label><label class="agent-chip"><input type="checkbox" name="agent-inline-dataset" value="connections" checked>Connections</label><label class="agent-chip"><input type="checkbox" name="agent-inline-dataset" value="knowledge" checked>Knowledge</label><label class="agent-chip"><input type="checkbox" name="agent-inline-dataset" value="goals" checked>Goals</label><label class="agent-chip"><input type="checkbox" name="agent-inline-dataset" value="operational_data">Operational data</label></div><textarea id="agent-prompt-text" maxlength="4000" required placeholder="Ask HomeServer to analyze current context, match conditions to a goal, draft a supervised plan, or prepare a World Mission…"></textarea><div class="agent-composer-bottom"><small>No external action occurs from this prompt without a separate local approval and execution step.</small><button type="submit" class="button primary" ${actionBusy ? "disabled" : ""}>Send</button></div></form></main><aside class="agent-workspace-context"><div class="agent-workspace-context-head"><h2>Data Context</h2><span class="agent-mode-badge">${connections().length} site${connections().length === 1 ? "" : "s"}</span></div><div class="agent-source-list">${renderDataSources()}</div></aside></section><section class="agent-workspace-drawer">${renderActiveTab()}</section>${renderModal()}</div>`;
}

function mount(force = false) {
  injectNavigation();
  document.querySelector('[data-agent-workspace-nav="true"]')?.classList.toggle("active", isAgentPage());
  if (!isAgentPage()) return;
  const canvas = document.querySelector(".page-canvas");
  if (!canvas) return;
  if (!force && canvas.querySelector('[data-agent-workspace-mounted="true"]')) return;
  canvas.innerHTML = loading && !snapshot ? '<div class="agent-workspace-page"><div class="agent-chat-empty"><div><strong>Loading Agent Workspace…</strong><p>Reading local goals, plans, approvals, connections, models, and World Mission drafts.</p></div></div></div>' : renderPage();
  bindEvents();
  if (!snapshot && !loading) void refresh();
  window.setTimeout(() => {
    const stream = document.querySelector("#agent-chat-stream");
    if (stream) stream.scrollTop = stream.scrollHeight;
  }, 0);
}

function bindEvents() {
  document.querySelector("#agent-workspace-refresh")?.addEventListener("click", refresh);
  document.querySelector("#agent-new-thread")?.addEventListener("click", () => {
    activeThreadId = null;
    notice = { kind: "info", message: "Your next prompt will start a new private local thread." };
    mount(true);
  });
  document.querySelectorAll("[data-agent-thread]").forEach((button) => button.addEventListener("click", () => {
    activeThreadId = button.dataset.agentThread;
    mount(true);
  }));
  document.querySelectorAll("[data-agent-tab]").forEach((button) => button.addEventListener("click", () => {
    activeTab = button.dataset.agentTab;
    mount(true);
  }));
  document.querySelectorAll("[data-agent-open-modal]").forEach((button) => button.addEventListener("click", () => {
    modal = button.dataset.agentOpenModal;
    mount(true);
    updatePlanExtraFields();
  }));
  document.querySelectorAll("[data-agent-close-modal]").forEach((element) => element.addEventListener("click", (event) => {
    if (event.target !== element && element.classList.contains("agent-modal-backdrop")) return;
    modal = null;
    mount(true);
  }));
  document.querySelector("#agent-prompt-form")?.addEventListener("submit", submitPrompt);
  document.querySelector("#agent-goal-form")?.addEventListener("submit", submitGoal);
  document.querySelector("#agent-plan-form")?.addEventListener("submit", submitPlan);
  document.querySelector("#agent-plan-action")?.addEventListener("change", updatePlanExtraFields);
  document.querySelector("#agent-mission-form")?.addEventListener("submit", submitMission);
  document.querySelectorAll("[data-agent-archive-goal]").forEach((button) => button.addEventListener("click", archiveGoal));
  document.querySelectorAll("[data-agent-approve]").forEach((button) => button.addEventListener("click", approvePlan));
  document.querySelectorAll("[data-agent-reject]").forEach((button) => button.addEventListener("click", rejectPlan));
  document.querySelectorAll("[data-agent-execute]").forEach((button) => button.addEventListener("click", executePlan));
  document.querySelectorAll("[data-agent-cancel-plan]").forEach((button) => button.addEventListener("click", cancelPlan));
  document.querySelectorAll("[data-agent-cancel-mission]").forEach((button) => button.addEventListener("click", cancelMission));
}

function updatePlanExtraFields() {
  const action = document.querySelector("#agent-plan-action")?.value;
  const target = document.querySelector("#agent-plan-extra-fields");
  if (!target) return;
  if (action === "backup.create") target.innerHTML = '<label>Backup note<input id="agent-plan-note" maxlength="500" placeholder="Approved Agent Workspace backup"></label>';
  else if (action === "model.health_test") target.innerHTML = '<label>Model name <small>optional</small><input id="agent-plan-model" maxlength="160" placeholder="Use configured default"></label>';
  else if (action === "report.save") target.innerHTML = '<label>Report title<input id="agent-plan-report-title" maxlength="180" required></label><label>Report content<textarea id="agent-plan-report-content" maxlength="30000" required></textarea></label>';
  else target.innerHTML = "";
}

async function runAction(action) {
  actionBusy = true;
  notice = null;
  mount(true);
  try {
    const message = await action();
    if (message) notice = message;
    snapshot = await invoke("homeserver_agent_workspace");
    ensureActiveThread();
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    actionBusy = false;
    modal = null;
    mount(true);
  }
}

async function refresh() {
  loading = true;
  mount(true);
  try {
    snapshot = await invoke("homeserver_agent_workspace");
    ensureActiveThread();
    notice = { kind: "info", message: "Agent Workspace refreshed from the local HomeServer service." };
  } catch (error) {
    notice = { kind: "warning", message: `Agent Workspace unavailable: ${String(error)}` };
  } finally {
    loading = false;
    mount(true);
  }
}

async function submitPrompt(event) {
  event.preventDefault();
  const prompt = document.querySelector("#agent-prompt-text")?.value?.trim() || "";
  if (!prompt) return;
  const goalId = document.querySelector("#agent-prompt-goal")?.value || "";
  const request = {
    thread_id: activeThreadId,
    mode: document.querySelector("#agent-prompt-mode")?.value || "ask",
    prompt,
    connection_ids: selectedValues('input[name="agent-connection-source"]'),
    dataset_keys: [...new Set([...selectedValues('input[name="agent-dataset-source"]'), ...selectedValues('input[name="agent-inline-dataset"]')])],
    goal_ids: goalId ? [goalId] : [],
    knowledge_query: prompt,
    model: document.querySelector("#agent-prompt-model")?.value || null,
    proposed_action: null,
    world_mission: null,
  };
  await runAction(async () => {
    const result = await invoke("homeserver_agent_prompt", { request });
    activeThreadId = result.thread_id;
    return { kind: "success", message: result.approvals_required ? "HomeServer answered and created a supervised approval request." : "HomeServer completed the local grounded response." };
  });
}

async function submitGoal(event) {
  event.preventDefault();
  const request = {
    title: document.querySelector("#agent-goal-title")?.value?.trim() || "",
    description: document.querySelector("#agent-goal-description")?.value?.trim() || "",
    target_metric: document.querySelector("#agent-goal-metric")?.value?.trim() || null,
    target_value: document.querySelector("#agent-goal-value")?.value?.trim() || null,
    target_date: document.querySelector("#agent-goal-date")?.value || null,
    connection_ids: selectedValues('input[name="agent-connection-source"]'),
    dataset_keys: selectedValues('input[name="agent-dataset-source"]'),
    constraints: {},
    allowed_actions: ["backup.create", "model.health_test", "cloud.sync_connection", "cloud.sync_all", "report.save"],
    approval_policy: document.querySelector("#agent-goal-policy")?.value || "always",
  };
  await runAction(async () => {
    await invoke("homeserver_create_agent_goal", { request });
    return { kind: "success", message: "Goal saved locally and is available to the HomeServer agent." };
  });
}

async function submitPlan(event) {
  event.preventDefault();
  const actionType = document.querySelector("#agent-plan-action")?.value || "backup.create";
  const selectedConnection = document.querySelector("#agent-plan-connection")?.value || null;
  const argumentsByAction = {
    "backup.create": { note: document.querySelector("#agent-plan-note")?.value?.trim() || null },
    "model.health_test": { model: document.querySelector("#agent-plan-model")?.value?.trim() || null },
    "cloud.sync_connection": {},
    "cloud.sync_all": {},
    "report.save": { title: document.querySelector("#agent-plan-report-title")?.value?.trim() || "", content_markdown: document.querySelector("#agent-plan-report-content")?.value?.trim() || "" },
  };
  const request = {
    thread_id: activeThreadId,
    title: document.querySelector("#agent-plan-title")?.value?.trim() || "",
    rationale: document.querySelector("#agent-plan-rationale")?.value?.trim() || "",
    action_type: actionType,
    arguments: Object.fromEntries(Object.entries(argumentsByAction[actionType] || {}).filter(([, value]) => value !== null && value !== "")),
    connection_id: actionType === "cloud.sync_connection" || actionType === "report.save" ? selectedConnection : null,
    goal_id: document.querySelector("#agent-plan-goal")?.value || null,
    dataset_keys: selectedValues('input[name="agent-dataset-source"]'),
    expires_minutes: 30,
  };
  await runAction(async () => {
    const result = await invoke("homeserver_create_agent_plan", { request });
    activeTab = "approvals";
    return { kind: "success", message: `${result.title} is awaiting one-use local approval.` };
  });
}

async function submitMission(event) {
  event.preventDefault();
  const request = {
    thread_id: activeThreadId,
    goal_id: document.querySelector("#agent-mission-goal")?.value || null,
    connection_id: document.querySelector("#agent-mission-connection")?.value || null,
    world_agent_id: document.querySelector("#agent-mission-agent")?.value?.trim() || "",
    title: document.querySelector("#agent-mission-title")?.value?.trim() || "",
    objective: document.querySelector("#agent-mission-objective")?.value?.trim() || "",
    allowed_operations: ["discover", "visit_store_canvas", "ask_questions", "compare", "request_information", "prepare_recommendation", "schedule_follow_up", "close_conversation"],
    prohibited_operations: ["purchase", "payment", "claim", "redemption", "share_private_profile", "accept_recurring_commitment", "publish_campaign", "bulk_message"],
    limits: {
      maximum_visits: Number(document.querySelector("#agent-mission-visits")?.value || 5),
      maximum_messages: Number(document.querySelector("#agent-mission-messages")?.value || 10),
      distance_limit_miles: Number(document.querySelector("#agent-mission-distance")?.value || 8),
    },
    disclosure_policy: { minimum_necessary: true, private_reasoning_local: true },
    expires_minutes: Number(document.querySelector("#agent-mission-expiry")?.value || 240),
  };
  await runAction(async () => {
    const result = await invoke("homeserver_create_world_mission", { request });
    activeTab = "missions";
    return { kind: "success", message: `${result.title} was saved as a local World Mission draft. It was not dispatched.` };
  });
}

async function archiveGoal(event) {
  const goalId = event.currentTarget.dataset.agentArchiveGoal;
  if (window.prompt("Type ARCHIVE to archive this goal:") !== "ARCHIVE") return;
  await runAction(async () => {
    await invoke("homeserver_archive_agent_goal", { goalId, confirmation: "ARCHIVE" });
    return { kind: "success", message: "Goal archived." };
  });
}

async function approvePlan(event) {
  const planId = event.currentTarget.dataset.agentApprove;
  if (window.prompt("Type APPROVE to issue one time-limited approval for this exact plan hash:") !== "APPROVE") return;
  await runAction(async () => {
    await invoke("homeserver_approve_agent_plan", { planId, confirmation: "APPROVE", reason: "Approved in the local Control Center" });
    return { kind: "success", message: "Plan approved for one bounded execution." };
  });
}

async function rejectPlan(event) {
  const planId = event.currentTarget.dataset.agentReject;
  if (window.prompt("Type REJECT to reject this plan:") !== "REJECT") return;
  await runAction(async () => {
    await invoke("homeserver_reject_agent_plan", { planId, confirmation: "REJECT", reason: "Rejected in the local Control Center" });
    return { kind: "success", message: "Plan rejected. No action executed." };
  });
}

async function executePlan(event) {
  const planId = event.currentTarget.dataset.agentExecute;
  if (window.prompt("Type EXECUTE to consume the approval and run this bounded action once:") !== "EXECUTE") return;
  await runAction(async () => {
    const receipt = await invoke("homeserver_execute_agent_plan", { planId, confirmation: "EXECUTE" });
    activeTab = "receipts";
    return { kind: receipt.state === "completed" ? "success" : "warning", message: receipt.result_summary };
  });
}

async function cancelPlan(event) {
  const planId = event.currentTarget.dataset.agentCancelPlan;
  if (!window.confirm("Cancel this unexecuted supervised plan?")) return;
  await runAction(async () => {
    await invoke("homeserver_cancel_agent_plan", { planId, confirmation: "CANCEL", reason: "Cancelled in the local Control Center" });
    return { kind: "success", message: "Plan cancelled before execution." };
  });
}

async function cancelMission(event) {
  const missionId = event.currentTarget.dataset.agentCancelMission;
  if (!window.confirm("Cancel this undispatched World Mission draft?")) return;
  await runAction(async () => {
    await invoke("homeserver_cancel_world_mission", { missionId, confirmation: "CANCEL" });
    return { kind: "success", message: "World Mission draft cancelled." };
  });
}

const app = document.querySelector("#app");
if (app) {
  const observer = new MutationObserver(() => {
    injectNavigation();
    if (isAgentPage()) mount(false);
  });
  observer.observe(app, { childList: true, subtree: true });
}

window.addEventListener("hashchange", () => window.setTimeout(() => mount(true), 0));
window.addEventListener("DOMContentLoaded", () => {
  if (initialized) return;
  initialized = true;
  window.setTimeout(() => mount(true), 0);
});
