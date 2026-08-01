import { invoke } from "@tauri-apps/api/core";
import { logoMark } from "./icons.js";
import "./shared-sidebar.css";

const AGENT_SIDEBAR_MODE = "chat-only";

let observer = null;
let observedHost = null;
let scheduled = false;
let decorating = false;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function currentPage() {
  return window.location.hash.replace("#", "") || "agent";
}

function isAgentPage() {
  return currentPage() === "agent";
}

function navigate(page) {
  if (currentPage() === page) return;
  window.location.hash = `#${page}`;
}

function createBrand() {
  const brand = document.createElement("button");
  brand.type = "button";
  brand.className = "brand-lockup shared-sidebar-brand";
  brand.setAttribute("aria-label", "Open HomeServer dashboard");
  brand.innerHTML = `${logoMark(43)}<div><strong>Microgifter</strong><span>HomeServer Agent</span></div>`;
  brand.addEventListener("click", () => navigate("dashboard"));
  return brand;
}

function threadTitle(button) {
  return button.querySelector("strong")?.textContent?.trim() || "HomeServer chat";
}

async function renameThread(button) {
  const threadId = button.dataset.chatThread || "";
  if (!threadId) return;
  const nextTitle = window.prompt("Rename this chat", threadTitle(button));
  if (nextTitle === null) return;
  const title = nextTitle.trim();
  if (!title) {
    window.alert("Chat names cannot be empty.");
    return;
  }
  try {
    const request = { action: "rename_thread", thread_id: threadId, title };
    await invoke("homeserver_create_agent_goal", { request });
    document.querySelector("#hs-chat-refresh")?.click();
  } catch (error) {
    window.alert(`Unable to rename chat: ${String(error)}`);
  }
}

async function deleteThread(button) {
  const threadId = button.dataset.chatThread || "";
  if (!threadId) return;
  if (!window.confirm(`Delete “${threadTitle(button)}” and its messages? Plans, approvals, and receipts remain preserved.`)) return;
  try {
    const request = { action: "delete_thread", thread_id: threadId, confirmation: "DELETE" };
    await invoke("homeserver_create_agent_goal", { request });
    document.querySelector("#hs-chat-refresh")?.click();
  } catch (error) {
    window.alert(`Unable to delete chat: ${String(error)}`);
  }
}

function addThreadActions(history) {
  history.querySelectorAll(":scope > .hs-chat-thread").forEach((button) => {
    const row = document.createElement("div");
    row.className = "shared-chat-row";
    button.before(row);
    row.append(button);

    const actions = document.createElement("span");
    actions.className = "shared-chat-actions";
    actions.innerHTML = `<button type="button" data-shared-chat-rename title="Rename chat" aria-label="Rename ${escapeHtml(threadTitle(button))}">✎</button><button type="button" data-shared-chat-delete title="Delete chat" aria-label="Delete ${escapeHtml(threadTitle(button))}">×</button>`;
    actions.querySelector("[data-shared-chat-rename]")?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      void renameThread(button);
    });
    actions.querySelector("[data-shared-chat-delete]")?.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      void deleteThread(button);
    });
    row.append(actions);
  });
}

function removeSidebarFooters() {
  document.querySelectorAll(
    ".app-sidebar > .server-card, .app-sidebar > .sidebar-state, .hs-chat-sidebar > .hs-chat-provider-summary, .hs-chat-sidebar > .hs-chat-sidebar-footer, .shared-agent-sidebar > .shared-sidebar-lower",
  ).forEach((element) => element.remove());
}

function removeUnexpectedControlCenterUi() {
  removeSidebarFooters();
  if (!isAgentPage()) return;
  document.querySelectorAll(
    ".agent-chat-shell > .app-sidebar:not(.hs-chat-sidebar), .agent-chat-shell .primary-nav",
  ).forEach((element) => element.remove());
}

function decorateAgentSidebar() {
  if (!isAgentPage() || decorating) return;
  removeUnexpectedControlCenterUi();

  const sidebar = document.querySelector(".hs-chat-sidebar");
  if (!sidebar || sidebar.dataset.sharedSidebar === "true") return;

  const newChat = sidebar.querySelector("#hs-chat-new");
  const search = sidebar.querySelector("#hs-chat-history-search")?.closest("label");
  const historyLabel = sidebar.querySelector(".hs-chat-history-label");
  const history = sidebar.querySelector(".hs-chat-history");
  if (!newChat || !search || !historyLabel || !history) return;

  decorating = true;
  try {
    sidebar.dataset.sharedSidebar = "true";
    sidebar.dataset.agentSidebarMode = AGENT_SIDEBAR_MODE;
    sidebar.classList.add("app-sidebar", "shared-agent-sidebar");

    const chatSection = document.createElement("section");
    chatSection.className = "shared-chat-section";
    chatSection.append(newChat, search, historyLabel, history);
    addThreadActions(history);

    sidebar.replaceChildren(createBrand(), chatSection);
  } finally {
    decorating = false;
  }
}

function scheduleDecorate() {
  if (scheduled) return;
  scheduled = true;
  window.requestAnimationFrame(() => {
    scheduled = false;
    removeUnexpectedControlCenterUi();
    decorateAgentSidebar();
    bindAgentObserver();
  });
}

function bindAgentObserver() {
  if (!isAgentPage()) {
    observer?.disconnect();
    observer = null;
    observedHost = null;
    return;
  }
  const host = document.querySelector('[data-homeserver-agent-host="true"]');
  if (!host || host === observedHost) return;
  observer?.disconnect();
  observedHost = host;
  observer = new MutationObserver(scheduleDecorate);
  observer.observe(host, { childList: true, subtree: true });
}

window.addEventListener("homeserver:rendered", scheduleDecorate);
window.addEventListener("homeserver-agent-route", scheduleDecorate);
window.addEventListener("hashchange", scheduleDecorate);
scheduleDecorate();

window.__HOMESERVER_SHARED_SIDEBAR_V1__ = {
  mode: AGENT_SIDEBAR_MODE,
  footerSections: "removed",
  refresh: scheduleDecorate,
};
window.__HOMESERVER_AGENT_SIDEBAR_CHAT_ONLY_V2__ = true;
