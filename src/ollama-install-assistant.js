import { invoke } from "@tauri-apps/api/core";
import "./ollama-install-assistant.css";

const OLLAMA_WINDOWS_PAGE = "https://ollama.com/download/windows";
const OLLAMA_SETUP_URL = "https://ollama.com/download/OllamaSetup.exe";
const OLLAMA_INSTALL_COMMAND = "irm https://ollama.com/install.ps1 | iex";
const OLLAMA_VERSION_COMMAND = "ollama --version";
const OLLAMA_API_COMMAND = "Invoke-RestMethod http://127.0.0.1:11434/api/version";
const REFRESH_AFTER_MS = 20_000;

let lastSnapshot = null;
let lastCheckedAt = 0;
let requestInFlight = false;
let mountQueued = false;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function isModelCenterVisible() {
  return window.location.hash === "#models" && Boolean(document.querySelector(".model-center-grid"));
}

function runtimeReady(snapshot) {
  return snapshot?.runtime?.state === "running";
}

function runtimeVersion(snapshot) {
  return snapshot?.runtime?.version ? `Ollama ${snapshot.runtime.version}` : "Ollama detected";
}

async function copyText(value, button) {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
    } else {
      const input = document.createElement("textarea");
      input.value = value;
      input.setAttribute("readonly", "");
      input.style.position = "fixed";
      input.style.opacity = "0";
      document.body.append(input);
      input.select();
      if (!document.execCommand("copy")) throw new Error("Clipboard copy was rejected.");
      input.remove();
    }
    const previous = button.textContent;
    button.textContent = "Copied";
    button.classList.add("copied");
    window.setTimeout(() => {
      button.textContent = previous;
      button.classList.remove("copied");
    }, 1600);
  } catch (error) {
    window.prompt("Copy this value:", value);
  }
}

function commandBlock(label, command, copyId) {
  return `<div class="ollama-command-block"><span>${escapeHtml(label)}</span><code>${escapeHtml(command)}</code><button type="button" class="button secondary" data-ollama-copy="${copyId}">Copy</button></div>`;
}

function unavailableMarkup(errorMessage = "") {
  return `<div class="ollama-assistant-heading">
      <div class="ollama-assistant-mark" aria-hidden="true">O</div>
      <div><span class="ollama-eyebrow">Local runtime setup</span><h2>Install Ollama for Model Center</h2><p>HomeServer could not reach Ollama at <code>127.0.0.1:11434</code>. Choose the official Windows installer or use PowerShell.</p></div>
      <span class="ollama-state warning">Not detected</span>
    </div>
    ${errorMessage ? `<div class="ollama-inline-warning">${escapeHtml(errorMessage)}</div>` : ""}
    <div class="ollama-install-grid">
      <article class="ollama-install-card primary-path">
        <div class="ollama-step-number">1</div>
        <div><span class="ollama-eyebrow">Recommended</span><h3>Official Windows installer</h3><p>Download <strong>OllamaSetup.exe</strong> from Ollama. The standard installer runs in your Windows account and keeps Ollama updated.</p></div>
        <div class="ollama-action-row"><a class="button primary" href="${OLLAMA_SETUP_URL}" target="_blank" rel="noopener noreferrer">Download official installer</a><button type="button" class="button secondary" data-ollama-copy="setup-url">Copy link</button></div>
        <small>Official source: <span class="mono">ollama.com</span> · Windows 10 or later</small>
      </article>
      <article class="ollama-install-card">
        <div class="ollama-step-number">2</div>
        <div><span class="ollama-eyebrow">Terminal option</span><h3>Install with PowerShell</h3><p>Open PowerShell as your normal Windows user, paste the official command, and approve the Ollama installer.</p></div>
        ${commandBlock("Official install command", OLLAMA_INSTALL_COMMAND, "install-command")}
        <small>HomeServer displays this command but does not execute remote scripts automatically.</small>
      </article>
      <article class="ollama-install-card verification-card">
        <div class="ollama-step-number">3</div>
        <div><span class="ollama-eyebrow">Verify</span><h3>Confirm the local runtime</h3><p>After setup finishes, verify the CLI and the loopback API, then return here and check again.</p></div>
        ${commandBlock("CLI version", OLLAMA_VERSION_COMMAND, "version-command")}
        ${commandBlock("Local API", OLLAMA_API_COMMAND, "api-command")}
        <button type="button" class="button primary full" data-ollama-refresh>Check Ollama again</button>
      </article>
    </div>
    <details class="ollama-help-details"><summary>Repair, update, or change model storage</summary><div><p>Run the latest official installer again to repair or update Ollama. Windows registers Ollama under <strong>Installed apps</strong> for uninstall. Models are normally stored under <code>%HOMEPATH%\\.ollama</code>.</p><p>To use another drive, set the user-level <code>OLLAMA_MODELS</code> environment variable before downloading models, restart Ollama, and refresh Model Center.</p><a href="${OLLAMA_WINDOWS_PAGE}" target="_blank" rel="noopener noreferrer">Open official Windows documentation</a></div></details>`;
}

function readyMarkup(snapshot) {
  const installed = snapshot?.installed_models?.length || 0;
  return `<div class="ollama-assistant-heading ready">
      <div class="ollama-assistant-mark" aria-hidden="true">O</div>
      <div><span class="ollama-eyebrow">Local runtime setup</span><h2>Ollama is installed and connected</h2><p>HomeServer is communicating with the fixed loopback runtime. Model prompts and inventory remain local.</p></div>
      <span class="ollama-state ready">Connected</span>
    </div>
    <div class="ollama-ready-grid"><div><span>Runtime</span><strong>${escapeHtml(runtimeVersion(snapshot))}</strong></div><div><span>Endpoint</span><strong class="mono">127.0.0.1:11434</strong></div><div><span>Installed models</span><strong>${installed}</strong></div><div><span>Boundary</span><strong>Local only</strong></div></div>
    <div class="ollama-action-row ready-actions"><button type="button" class="button secondary" data-ollama-refresh>Recheck runtime</button><a class="button ghost" href="${OLLAMA_WINDOWS_PAGE}" target="_blank" rel="noopener noreferrer">Ollama documentation</a></div>`;
}

function bindAssistantEvents(container) {
  const values = {
    "setup-url": OLLAMA_SETUP_URL,
    "install-command": OLLAMA_INSTALL_COMMAND,
    "version-command": OLLAMA_VERSION_COMMAND,
    "api-command": OLLAMA_API_COMMAND,
  };

  container.querySelectorAll("[data-ollama-copy]").forEach((button) => {
    button.addEventListener("click", () => copyText(values[button.dataset.ollamaCopy] || "", button));
  });
  container.querySelectorAll("[data-ollama-refresh]").forEach((button) => {
    button.addEventListener("click", () => refreshAssistant(true));
  });
}

function renderAssistant(snapshot = lastSnapshot, errorMessage = "") {
  if (!isModelCenterVisible()) return;
  const before = document.querySelector(".model-center-grid");
  if (!before) return;

  let container = document.querySelector("#ollama-install-assistant");
  if (!container) {
    container = document.createElement("section");
    container.id = "ollama-install-assistant";
    container.className = "ollama-install-assistant panel";
    before.parentElement?.insertBefore(container, before);
  }

  container.dataset.runtimeState = runtimeReady(snapshot) ? "ready" : "missing";
  container.dataset.checking = requestInFlight ? "true" : "false";
  container.innerHTML = requestInFlight && !snapshot
    ? `<div class="ollama-assistant-loading"><span></span><strong>Checking for Ollama…</strong></div>`
    : runtimeReady(snapshot) ? readyMarkup(snapshot) : unavailableMarkup(errorMessage);
  bindAssistantEvents(container);
}

async function refreshAssistant(announce = false) {
  if (requestInFlight || !isModelCenterVisible()) return;
  requestInFlight = true;
  renderAssistant(lastSnapshot);
  try {
    lastSnapshot = await invoke("homeserver_models");
    lastCheckedAt = Date.now();
    renderAssistant(lastSnapshot);
    if (announce && !runtimeReady(lastSnapshot)) {
      document.querySelector("#ollama-install-assistant")?.scrollIntoView({ behavior: "smooth", block: "start" });
    }
  } catch (error) {
    lastCheckedAt = Date.now();
    renderAssistant(lastSnapshot, `Unable to check the local runtime: ${String(error)}`);
  } finally {
    requestInFlight = false;
    renderAssistant(lastSnapshot);
  }
}

function mountAssistant() {
  mountQueued = false;
  if (!isModelCenterVisible()) return;
  renderAssistant(lastSnapshot);
  if (!lastSnapshot || Date.now() - lastCheckedAt > REFRESH_AFTER_MS) refreshAssistant(false);
}

function queueMount() {
  if (mountQueued) return;
  mountQueued = true;
  window.requestAnimationFrame(mountAssistant);
}

const app = document.querySelector("#app");
if (app) {
  new MutationObserver(queueMount).observe(app, { childList: true, subtree: true });
}
window.addEventListener("hashchange", queueMount);
queueMount();
