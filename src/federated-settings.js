import { invoke } from "@tauri-apps/api/core";
import "./federated-settings.css";

let snapshot = null;
let loading = false;
let notice = null;
let installed = false;
let observer = null;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function categoryTitle(category) {
  return ({
    appearance: "Appearance",
    regional: "Regional",
    updates: "Updates",
    notifications: "Notifications",
    privacy: "Privacy",
    commerce: "Commerce defaults",
  })[category] || String(category || "Settings").replaceAll("_", " ");
}

function authorityLabel(authority) {
  return ({
    vp3: "VP3 authority",
    homeserver: "HomeServer authority",
    shared: "Shared authority",
  })[authority] || authority;
}

function formatDate(value) {
  if (!value) return "Not yet";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString();
}

function control(setting) {
  const disabled = setting.editable_in_homeserver && !loading ? "" : " disabled";
  const key = escapeHtml(setting.setting_key);
  if (setting.value_type === "boolean") {
    return `<label class="fss-switch"><input type="checkbox" data-fss-input="${key}" data-local-revision="${setting.local_revision}"${setting.value ? " checked" : ""}${disabled}><span></span><em>${setting.value ? "Enabled" : "Disabled"}</em></label>`;
  }
  if (setting.value_type === "enum") {
    const options = (setting.allowed_values || []).map((value) => `<option value="${escapeHtml(value)}"${value === setting.value ? " selected" : ""}>${escapeHtml(value)}</option>`).join("");
    return `<select data-fss-input="${key}" data-local-revision="${setting.local_revision}"${disabled}>${options}</select>`;
  }
  if (setting.value_type === "integer") {
    return `<input type="number" data-fss-input="${key}" data-local-revision="${setting.local_revision}" value="${escapeHtml(setting.value)}"${disabled}>`;
  }
  return `<input type="text" maxlength="200" data-fss-input="${key}" data-local-revision="${setting.local_revision}" value="${escapeHtml(setting.value)}"${disabled}>`;
}

function render() {
  if (window.location.hash !== "#settings") return;
  const anchor = document.querySelector("#vp3-authority-section") || document.querySelector(".settings-layout");
  if (!anchor) return;
  let root = document.querySelector("#federated-settings-section");
  if (!root) {
    root = document.createElement("section");
    root.id = "federated-settings-section";
    root.className = "federated-settings-section";
    anchor.insertAdjacentElement("afterend", root);
  }

  const groups = new Map();
  for (const setting of snapshot?.settings || []) {
    if (!groups.has(setting.category)) groups.set(setting.category, []);
    groups.get(setting.category).push(setting);
  }
  root.innerHTML = `
    <article class="panel fss-card">
      <header class="fss-heading">
        <div><span class="fss-kicker">One configuration model · local privacy preserved</span><h2>VP3 & HomeServer Settings</h2><p>VP3 controls cloud and commercial policy. HomeServer controls local private behavior. Shared non-secret preferences synchronize through revisions and conflict receipts.</p></div>
        <div class="fss-heading-actions"><span class="fss-state ${snapshot?.configured ? "active" : "attention"}">${snapshot?.configured ? "VP3 connected" : "Local only"}</span><button id="fss-sync" class="button primary" type="button" ${loading || !snapshot?.configured ? "disabled" : ""}>Sync Now</button></div>
      </header>
      ${notice ? `<div class="fss-notice ${escapeHtml(notice.kind)}">${escapeHtml(notice.message)}</div>` : ""}
      <div class="fss-summary">
        <div><span>Cloud revision</span><strong>${escapeHtml(snapshot?.max_cloud_revision ?? 0)}</strong></div>
        <div><span>Local changes</span><strong>${escapeHtml(snapshot?.dirty_count ?? 0)}</strong></div>
        <div><span>Last sync</span><strong>${escapeHtml(formatDate(snapshot?.last_synced_at_utc))}</strong></div>
        <div><span>Snapshot</span><strong>${escapeHtml(snapshot?.snapshot_hash ? snapshot.snapshot_hash.slice(0, 12) : "Local")}</strong></div>
      </div>
      <div class="fss-groups">
        ${[...groups.entries()].map(([category, settings]) => `
          <section class="fss-group">
            <header><div><span>${escapeHtml(category)}</span><h3>${escapeHtml(categoryTitle(category))}</h3></div><small>${settings.length} settings</small></header>
            <div class="fss-list">
              ${settings.map((setting) => `
                <article class="fss-row ${setting.editable_in_homeserver ? "" : "locked"}">
                  <div class="fss-copy"><div class="fss-title"><strong>${escapeHtml(setting.label)}</strong><span class="fss-authority ${escapeHtml(setting.authority)}">${escapeHtml(authorityLabel(setting.authority))}</span>${setting.dirty ? '<span class="fss-dirty">Pending sync</span>' : ""}</div><p>${escapeHtml(setting.description)}</p><small>${escapeHtml(setting.setting_key)} · local ${setting.local_revision} · cloud ${setting.cloud_revision}${setting.last_conflict_reason ? ` · conflict: ${escapeHtml(setting.last_conflict_reason)}` : ""}</small></div>
                  <div class="fss-control">${control(setting)}</div>
                </article>`).join("")}
            </div>
          </section>`).join("") || '<div class="fss-empty">Loading settings…</div>'}
      </div>
      <div class="fss-boundary"><strong>Privacy boundary</strong><p>Only cataloged non-secret preferences synchronize. Stripe credentials, API keys, private files, prompts, conversations, models, MCP content, and local execution data never enter this settings payload.</p></div>
    </article>`;
  bind(root);
}

function bind(root) {
  root.querySelector("#fss-sync")?.addEventListener("click", synchronize);
  root.querySelectorAll("[data-fss-input]").forEach((input) => {
    input.addEventListener("change", async () => {
      const setting = snapshot?.settings?.find((item) => item.setting_key === input.dataset.fssInput);
      if (!setting || input.disabled) return;
      let value = input.value;
      if (setting.value_type === "boolean") value = input.checked;
      if (setting.value_type === "integer") value = Number.parseInt(input.value, 10);
      loading = true;
      notice = { kind: "info", message: `Saving ${setting.label} locally…` };
      render();
      try {
        snapshot = await invoke("homeserver_update_federated_setting", {
          settingKey: setting.setting_key,
          value,
          expectedLocalRevision: Number(input.dataset.localRevision || 0),
        });
        notice = { kind: "success", message: `${setting.label} saved locally${setting.authority === "homeserver" ? "." : " and queued for VP3 sync."}` };
      } catch (error) {
        notice = { kind: "warning", message: String(error) };
        await load(false);
      } finally {
        loading = false;
        render();
      }
    });
  });
}

async function synchronize() {
  loading = true;
  notice = { kind: "info", message: "Synchronizing revisioned settings with VP3…" };
  render();
  try {
    snapshot = await invoke("homeserver_sync_federated_settings");
    notice = snapshot.dirty_count
      ? { kind: "warning", message: `${snapshot.dirty_count} local setting change remains pending because VP3 reported a conflict.` }
      : { kind: "success", message: "VP3 and HomeServer settings are synchronized." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  } finally {
    loading = false;
    render();
  }
}

async function load(clearNotice = true) {
  if (window.location.hash !== "#settings") return;
  loading = true;
  if (clearNotice) notice = null;
  render();
  try {
    snapshot = await invoke("homeserver_federated_settings");
  } catch (error) {
    notice = { kind: "warning", message: `Shared settings are unavailable: ${String(error)}` };
  } finally {
    loading = false;
    render();
  }
}

function mount() {
  if (window.location.hash !== "#settings") return;
  render();
  if (!snapshot && !loading) load();
}

function install() {
  if (installed) return;
  installed = true;
  window.addEventListener("homeserver:rendered", mount);
  window.addEventListener("hashchange", mount);
  window.addEventListener("DOMContentLoaded", mount, { once: true });
  const app = document.querySelector("#app");
  if (app) {
    observer = new MutationObserver(() => {
      if (window.location.hash === "#settings" && !document.querySelector("#federated-settings-section")) {
        queueMicrotask(mount);
      }
    });
    observer.observe(app, { childList: true, subtree: true });
  }
  mount();
}

install();
