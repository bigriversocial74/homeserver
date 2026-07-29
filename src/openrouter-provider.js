import { invoke } from "@tauri-apps/api/core";
import "./openrouter-provider.css";

let snapshot = null;
let catalog = [];
let testResult = null;
let loading = false;
let notice = null;
let installed = false;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function dollarsFromMicros(value) {
  if (value == null) return "";
  return (Number(value) / 1_000_000).toFixed(2);
}

function microsFromDollars(value) {
  const text = String(value || "").trim();
  if (!text) return null;
  const amount = Number(text);
  if (!Number.isFinite(amount) || amount < 0) throw new Error("Monthly budget must be a positive dollar amount.");
  return Math.round(amount * 1_000_000);
}

function numberOrNull(value) {
  const text = String(value || "").trim();
  if (!text) return null;
  const number = Number(text);
  if (!Number.isInteger(number) || number < 1) throw new Error("Monthly request limit must be a positive whole number.");
  return number;
}

function providerState() {
  if (!snapshot) return "Loading";
  if (!snapshot.api_key_configured) return "Not connected";
  if (!snapshot.enabled) return "Configured";
  if (!snapshot.allow_remote_context) return "Enabled · context blocked";
  return "Enabled for Agent Workspace";
}

function modelOptions(selected) {
  const seen = new Set();
  return catalog
    .filter((model) => model?.id && !seen.has(model.id) && seen.add(model.id))
    .map((model) => `<option value="${escapeHtml(model.id)}" ${model.id === selected ? "selected" : ""}>${escapeHtml(model.name || model.id)}</option>`)
    .join("");
}

function fallbackText() {
  return Array.isArray(snapshot?.fallback_models) ? snapshot.fallback_models.join(", ") : "";
}

function render() {
  if (window.location.hash !== "#models") return;
  const target = document.querySelector(".model-center-grid");
  if (!target) return;
  let root = document.querySelector("#openrouter-provider-section");
  if (!root) {
    root = document.createElement("section");
    root.id = "openrouter-provider-section";
    root.className = "openrouter-provider-section";
    target.insertAdjacentElement("afterend", root);
  }
  const selectedModel = snapshot?.default_model || "";
  const budget = dollarsFromMicros(snapshot?.monthly_budget_microusd);
  const spend = dollarsFromMicros(snapshot?.monthly_spend_microusd || 0) || "0.00";
  const requestLimit = snapshot?.monthly_request_limit ?? "";
  const requestCount = Number(snapshot?.monthly_request_count || 0);
  root.innerHTML = `
    <article class="panel openrouter-provider-card">
      <div class="openrouter-provider-heading">
        <div>
          <p class="provider-kicker">Optional hosted model provider</p>
          <h2>OpenRouter</h2>
          <span>Use one locally stored API key to access user-selected hosted models while HomeServer remains the agent harness.</span>
        </div>
        <span class="provider-state ${snapshot?.enabled ? "is-enabled" : ""}">${escapeHtml(providerState())}</span>
      </div>
      ${notice ? `<div class="openrouter-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}
      <div class="openrouter-provider-grid">
        <form id="openrouter-settings-form" class="openrouter-settings-form">
          <label><span>API key</span><input id="openrouter-api-key" type="password" autocomplete="off" maxlength="512" placeholder="${snapshot?.api_key_configured ? "Stored in Windows Credential Manager · leave blank to keep" : "Paste OpenRouter API key"}"></label>
          <label><span>Default model</span><div class="model-picker-row"><select id="openrouter-default-model"><option value="">Select model</option>${modelOptions(selectedModel)}</select><input id="openrouter-default-model-manual" list="openrouter-model-list" maxlength="190" value="${escapeHtml(selectedModel)}" placeholder="provider/model-slug"><button id="openrouter-load-catalog" class="button ghost" type="button" ${loading ? "disabled" : ""}>Load catalog</button></div></label>
          <datalist id="openrouter-model-list">${catalog.map((model) => `<option value="${escapeHtml(model.id)}">${escapeHtml(model.name || model.id)}</option>`).join("")}</datalist>
          <label><span>Fallback models</span><input id="openrouter-fallback-models" type="text" maxlength="1600" value="${escapeHtml(fallbackText())}" placeholder="model/one, model/two"></label>
          <div class="openrouter-limit-grid">
            <label><span>Monthly budget (USD)</span><input id="openrouter-monthly-budget" type="number" min="0" step="0.01" value="${escapeHtml(budget)}" placeholder="No local cap"></label>
            <label><span>Monthly requests</span><input id="openrouter-monthly-requests" type="number" min="1" step="1" value="${escapeHtml(requestLimit)}" placeholder="No local cap"></label>
            <label><span>Max output tokens</span><input id="openrouter-max-output" type="number" min="16" max="4096" step="16" value="${Number(snapshot?.max_output_tokens || 800)}"></label>
          </div>
          <div class="openrouter-policy-grid">
            <label><span>Routing priority</span><select id="openrouter-routing-sort"><option value="price" ${snapshot?.routing_sort === "price" ? "selected" : ""}>Lowest price</option><option value="throughput" ${snapshot?.routing_sort === "throughput" ? "selected" : ""}>Highest throughput</option><option value="latency" ${snapshot?.routing_sort === "latency" ? "selected" : ""}>Lowest latency</option></select></label>
            <label><span>Provider data policy</span><select id="openrouter-data-collection"><option value="deny" ${snapshot?.data_collection !== "allow" ? "selected" : ""}>Deny data collection</option><option value="allow" ${snapshot?.data_collection === "allow" ? "selected" : ""}>Allow</option></select></label>
          </div>
          <label class="openrouter-check"><input id="openrouter-enabled" type="checkbox" ${snapshot?.enabled ? "checked" : ""}><span>Enable OpenRouter provider</span></label>
          <label class="openrouter-check privacy"><input id="openrouter-remote-context" type="checkbox" ${snapshot?.allow_remote_context ? "checked" : ""}><span>Allow selected Agent Workspace prompts and bounded evidence to leave this HomeServer</span></label>
          <label class="openrouter-check"><input id="openrouter-provider-fallbacks" type="checkbox" ${snapshot?.allow_provider_fallbacks !== false ? "checked" : ""}><span>Allow OpenRouter provider and configured model fallbacks</span></label>
          <label class="openrouter-check"><input id="openrouter-zdr" type="checkbox" ${snapshot?.zdr_only ? "checked" : ""}><span>Require Zero Data Retention endpoints</span></label>
          <div class="openrouter-actions"><button class="button primary" type="submit" ${loading ? "disabled" : ""}>Save OpenRouter</button><button id="openrouter-disconnect" class="button danger" type="button" ${!snapshot?.api_key_configured || loading ? "disabled" : ""}>Disconnect</button></div>
        </form>
        <aside class="openrouter-provider-aside">
          <div class="openrouter-usage-card"><span>Current month</span><strong>$${escapeHtml(spend)}</strong><small>${requestCount} request${requestCount === 1 ? "" : "s"}${budget ? ` · $${escapeHtml(budget)} local cap` : " · no local dollar cap"}</small></div>
          <form id="openrouter-test-form" class="openrouter-test-form"><h3>Remote connection test</h3><p>This sends only the text entered below. It does not include Knowledge Vault documents or operational evidence.</p><textarea id="openrouter-test-prompt" maxlength="1000" rows="4" placeholder="Reply with one sentence confirming the connection." required></textarea><button class="button secondary" type="submit" ${!snapshot?.enabled || !snapshot?.allow_remote_context || loading ? "disabled" : ""}>Run remote test</button></form>
          ${testResult ? `<div class="openrouter-test-result"><strong>${escapeHtml(testResult.resolved_model)}</strong><p>${escapeHtml(testResult.output)}</p><small>${Number(testResult.total_tokens || 0)} tokens · ${Number(testResult.duration_ms || 0)} ms</small></div>` : ""}
          <div class="openrouter-privacy-card"><strong>Privacy boundary</strong><p>The API key stays in Windows Credential Manager. SQLite stores configuration and usage receipts only—never the key, prompt, response, Knowledge Vault content, or conversation text.</p></div>
        </aside>
      </div>
    </article>`;
  bind(root);
}

function bind(root) {
  root.querySelector("#openrouter-default-model")?.addEventListener("change", (event) => {
    const manual = root.querySelector("#openrouter-default-model-manual");
    if (manual) manual.value = event.currentTarget.value;
  });
  root.querySelector("#openrouter-load-catalog")?.addEventListener("click", loadCatalog);
  root.querySelector("#openrouter-settings-form")?.addEventListener("submit", saveSettings);
  root.querySelector("#openrouter-test-form")?.addEventListener("submit", runTest);
  root.querySelector("#openrouter-disconnect")?.addEventListener("click", disconnect);
}

async function loadStatus() {
  if (window.location.hash !== "#models") return;
  try {
    snapshot = await invoke("homeserver_openrouter_status");
    notice = null;
  } catch (error) {
    notice = { kind: "warning", message: `OpenRouter provider unavailable: ${String(error)}` };
  }
  render();
}

async function loadCatalog() {
  loading = true;
  notice = null;
  render();
  try {
    const result = await invoke("homeserver_openrouter_catalog");
    catalog = Array.isArray(result?.models) ? result.models : [];
    notice = { kind: "success", message: `Loaded ${catalog.length} models allowed by this OpenRouter account.` };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    loading = false;
    render();
  }
}

async function saveSettings(event) {
  event.preventDefault();
  const root = event.currentTarget;
  const enabled = Boolean(root.querySelector("#openrouter-enabled")?.checked);
  const allowRemoteContext = Boolean(root.querySelector("#openrouter-remote-context")?.checked);
  let remoteContextConfirmation = null;
  if (allowRemoteContext && !snapshot?.allow_remote_context) {
    remoteContextConfirmation = window.prompt("This allows selected Agent Workspace prompts and bounded evidence to leave HomeServer. Type SEND REMOTE to continue:");
    if (remoteContextConfirmation !== "SEND REMOTE") return;
  } else if (allowRemoteContext) {
    remoteContextConfirmation = "SEND REMOTE";
  }
  const defaultModel = root.querySelector("#openrouter-default-model-manual")?.value?.trim() || null;
  const fallbackModels = String(root.querySelector("#openrouter-fallback-models")?.value || "").split(",").map((value) => value.trim()).filter(Boolean);
  loading = true;
  notice = null;
  render();
  try {
    snapshot = await invoke("homeserver_configure_openrouter", {
      apiKey: root.querySelector("#openrouter-api-key")?.value?.trim() || null,
      enabled,
      allowRemoteContext,
      remoteContextConfirmation,
      defaultModel,
      fallbackModels,
      monthlyBudgetMicrousd: microsFromDollars(root.querySelector("#openrouter-monthly-budget")?.value),
      monthlyRequestLimit: numberOrNull(root.querySelector("#openrouter-monthly-requests")?.value),
      maxOutputTokens: Number(root.querySelector("#openrouter-max-output")?.value || 800),
      routingSort: root.querySelector("#openrouter-routing-sort")?.value || "price",
      allowProviderFallbacks: Boolean(root.querySelector("#openrouter-provider-fallbacks")?.checked),
      dataCollection: root.querySelector("#openrouter-data-collection")?.value || "deny",
      zdrOnly: Boolean(root.querySelector("#openrouter-zdr")?.checked),
    });
    notice = { kind: "success", message: "OpenRouter settings saved locally." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    loading = false;
    render();
  }
}

async function runTest(event) {
  event.preventDefault();
  const prompt = event.currentTarget.querySelector("#openrouter-test-prompt")?.value?.trim() || "";
  if (!prompt) return;
  const confirmation = window.prompt("This test sends the entered text to OpenRouter. Type TEST REMOTE to continue:");
  if (confirmation !== "TEST REMOTE") return;
  loading = true;
  notice = null;
  render();
  try {
    testResult = await invoke("homeserver_test_openrouter", {
      model: snapshot?.default_model || null,
      prompt,
      confirmation,
    });
    snapshot = await invoke("homeserver_openrouter_status");
    notice = { kind: "success", message: "OpenRouter remote test completed." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    loading = false;
    render();
  }
}

async function disconnect() {
  const confirmation = window.prompt("Type DISCONNECT to remove the OpenRouter API key from Windows Credential Manager:");
  if (confirmation !== "DISCONNECT") return;
  loading = true;
  notice = null;
  render();
  try {
    snapshot = await invoke("homeserver_disconnect_openrouter", { confirmation });
    catalog = [];
    testResult = null;
    notice = { kind: "success", message: "OpenRouter disconnected and its local credential was removed." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    loading = false;
    render();
  }
}

function mount() {
  if (window.location.hash !== "#models") return;
  render();
  if (!snapshot && !loading) loadStatus();
}

function install() {
  if (installed) return;
  installed = true;
  window.addEventListener("homeserver:rendered", mount);
  window.addEventListener("hashchange", mount);
  window.addEventListener("DOMContentLoaded", mount, { once: true });
  mount();
}

install();
