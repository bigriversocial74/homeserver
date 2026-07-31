import { invoke } from "@tauri-apps/api/core";
import { icon, logoMark } from "./icons.js";
import "./shared-sidebar.css";

const MENU_ITEMS = [
  ["agent", "HomeServer Agent", "integrations"],
  ["home", "Home", "home"],
  ["dashboard", "Dashboard", "dashboard"],
  ["models", "Model Center", "model"],
  ["apps", "Apps", "apps"],
  ["knowledge", "Knowledge Vault", "vault"],
  ["backups", "Backups", "backup"],
  ["integrations", "Integrations & Agents", "integrations"],
  ["settings", "Settings", "settings"],
  ["sync", "Sync Cloud", "cloud"],
  ["system", "System", "system"],
];

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

function menuMarkup(activePage, shared = false) {
  const attribute = shared ? "data-shared-page" : "data-page";
  return MENU_ITEMS.map(([key, label, iconName]) => `<button type="button" class="nav-item ${activePage === key ? "active" : ""}" ${attribute}="${key}">${icon(iconName, 19)}<span>${escapeHtml(label)}</span></button>`).join("");
}

function reorderPrimaryNavigation() {
  if (isAgentPage()) return;
  const nav = document.querySelector(".app-sidebar .primary-nav");
  if (!nav) return;
  const buttons = new Map([...nav.querySelectorAll("[data-page]")].map((button) => [button.dataset.page, button]));
  for (const [key] of MENU_ITEMS) {
    const button = buttons.get(key);
    if (button) nav.append(button);
  }
}

function createBrand() {
  const brand = document.createElement("button");
  brand.type = "button";
  brand.className = "brand-lockup shared-sidebar-brand";
  brand.setAttribute("aria-label", "Open HomeServer dashboard");
  brand.innerHTML = `${logoMark(43)}<div><strong>Microgifter</strong><span>HomeServer</span></div>`;
  brand.addEventListener("click", () => navigate("dashboard"));
  return brand;
}

function createNavigation() {
  const nav = document.createElement("nav");
  nav.className = "primary-nav shared-sidebar-navigation";
  nav.setAttribute("aria-label", "HomeServer pages");
  nav.innerHTML = menuMarkup("agent", true);
  nav.querySelectorAll("[data-shared-page]").forEach((button) => {
    button.addEventListener("click", () => navigate(button.dataset.sharedPage));
  });
  return nav;
}

function createServerCard() {
  const card = document.createElement("div");
  card.className = "server-card shared-sidebar-server-card";
  card.innerHTML = `<div class="server-card-top"><div class="server-glyph">${icon("system", 22)}</div><div><strong>HomeServer</strong><span><i class="live-dot"></i>Online</span></div></div><div class="server-divider"></div><small>Local Control Center</small><button type="button" class="text-button" data-shared-system>View system status</button>`;
  card.querySelector("[data-shared-system]")?.addEventListener("click", () => navigate("system"));
  return card;
}

function createSidebarState() {
  const state = document.createElement("div");
  state.className = "sidebar-state";
  state.innerHTML = '<span class="state-orb healthy"></span><span>online</span>';
  return state;
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

function decorateAgentSidebar() {
  if (!isAgentPage() || decorating) return;
  const sidebar = document.querySelector(".hs-chat-sidebar");
  if (!sidebar || sidebar.dataset.sharedSidebar === "true") return;

  const newChat = sidebar.querySelector("#hs-chat-new");
  const search = sidebar.querySelector("#hs-chat-history-search")?.closest("label");
  const historyLabel = sidebar.querySelector(".hs-chat-history-label");
  const history = sidebar.querySelector(".hs-chat-history");
  const provider = sidebar.querySelector("#hs-chat-provider-summary");
  if (!newChat || !search || !historyLabel || !history) return;

  decorating = true;
  try {
    sidebar.dataset.sharedSidebar = "true";
    sidebar.classList.add("app-sidebar", "shared-agent-sidebar");

    const chatSection = document.createElement("section");
    chatSection.className = "shared-chat-section";
    chatSection.append(newChat, search, historyLabel, history);
    addThreadActions(history);

    const lower = document.createElement("div");
    lower.className = "shared-sidebar-lower";
    if (provider) lower.append(provider);
    lower.append(createServerCard(), createSidebarState());

    sidebar.replaceChildren(createBrand(), createNavigation(), chatSection, lower);
  } finally {
    decorating = false;
  }
}

function scheduleDecorate() {
  if (scheduled) return;
  scheduled = true;
  window.requestAnimationFrame(() => {
    scheduled = false;
    reorderPrimaryNavigation();
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
  menu: MENU_ITEMS.map(([key]) => key),
  refresh: scheduleDecorate,
};
