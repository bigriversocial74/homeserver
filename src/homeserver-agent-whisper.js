import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { audioBlobToWhisperPcm } from "./homeserver-whisper-codec.js";
import { WhisperSegmentQueue } from "./homeserver-whisper-queue.js";
import "./homeserver-agent-whisper.css";

const state = {
  status: null,
  busy: false,
  importing: false,
  active: null,
  expectedSha256: "",
  language: "en",
  notice: null,
  partials: new Map(),
  queue: new WhisperSegmentQueue(),
};

let observer = null;
let scheduled = false;
let unlistenProgress = null;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function validSha256(value) {
  return /^[a-f0-9]{64}$/i.test(String(value || "").trim());
}

function notify(message, kind = "info") {
  state.notice = { message: String(message), kind };
  scheduleRender();
}

async function refreshStatus() {
  try {
    state.status = await invoke("homeserver_whisper_status");
  } catch (error) {
    state.status = {
      model_ready: false,
      verification_state: "unavailable",
      active_transcription_id: null,
      capabilities: {},
    };
    notify(`Local Whisper status is unavailable: ${String(error)}`, "warning");
  }
  scheduleRender();
  return state.status;
}

async function importModel() {
  const expected = state.expectedSha256.trim().toLowerCase();
  if (
    !validSha256(expected)
    || state.importing
    || state.active
    || state.busy
    || state.queue.length
  ) {
    notify("Enter the trusted model's 64-character SHA-256 before importing it.", "warning");
    return;
  }
  state.importing = true;
  scheduleRender();
  try {
    const result = await invoke("homeserver_import_whisper_model", {
      expectedSha256: expected,
      confirmation: `IMPORT WHISPER MODEL ${expected}`,
    });
    if (result) {
      state.status = result;
      notify("Local Whisper model imported and verified.", "success");
    } else {
      notify("Local Whisper model import was cancelled.");
    }
  } catch (error) {
    notify(`Local Whisper model import failed: ${String(error)}`, "warning");
  } finally {
    state.importing = false;
    scheduleRender();
  }
}

async function removeModel() {
  if (state.active || state.busy || state.importing || state.queue.length) return;
  try {
    state.status = await invoke("homeserver_remove_whisper_model", {
      confirmation: "REMOVE LOCAL WHISPER MODEL",
    });
    state.expectedSha256 = "";
    notify("Local Whisper model removed from this Control Center.");
  } catch (error) {
    notify(`Unable to remove the local Whisper model: ${String(error)}`, "warning");
  }
  scheduleRender();
}

async function updateGovernedTranscript(segmentId, transcript, receipt) {
  await invoke("homeserver_agent_integration_action", {
    request: {
      action: "audio_update_transcript",
      segment_id: segmentId,
      transcript,
      linked_message_id: null,
      transcription_id: receipt.transcription_id,
      transcription_engine: receipt.engine,
      transcription_model_sha256: receipt.model_sha256,
      transcription_language: receipt.language,
      transcription_final: true,
      raw_audio_retained: false,
    },
  });
}

function updateTranscriptElement(segmentId, text, final = false) {
  const textarea = document.querySelector(
    `[data-agent-audio-transcript="${CSS.escape(segmentId)}"]`,
  );
  if (!textarea || textarea.readOnly) return;
  textarea.value = text;
  textarea.dataset.localWhisper = final ? "final" : "partial";
  textarea.dispatchEvent(new Event("input", { bubbles: true }));
}

function queueSegment(detail) {
  const result = state.queue.enqueue(detail);
  if (result.accepted) {
    notify(
      `Queued ${result.length} local utterance${result.length === 1 ? "" : "s"} for transcription.`,
    );
    return;
  }
  if (result.reason === "capacity") {
    notify(
      "The bounded local transcription queue is full; this utterance was not queued.",
      "warning",
    );
  }
}

function drainQueuedSegment() {
  if (state.active || state.busy) return;
  const next = state.queue.shift();
  if (next) void transcribeLocalSegment(next);
}

async function transcribeLocalSegment(detail) {
  if (!detail?.segment_id || !(detail.blob instanceof Blob)) return;
  if (state.active?.segmentId === detail.segment_id) return;
  if (state.active || state.busy) {
    queueSegment(detail);
    return;
  }
  const status = await refreshStatus();
  if (!status?.model_ready) {
    notify("The utterance is ready, but a verified local Whisper model has not been imported.", "warning");
    return;
  }

  state.busy = true;
  state.active = {
    segmentId: detail.segment_id,
    transcriptionId: null,
    progress: 0,
  };
  state.partials.delete(detail.segment_id);
  notify("Preparing the utterance for local Whisper transcription.");
  scheduleRender();

  try {
    const pcm = await audioBlobToWhisperPcm(detail.blob);
    notify("Running whisper.cpp locally. Audio is not sent to a cloud service.", "success");
    const receipt = await invoke("homeserver_whisper_transcribe", {
      request: {
        segment_id: detail.segment_id,
        pcm16_base64: pcm.pcm16_base64,
        sample_rate_hz: pcm.sample_rate_hz,
        channels: pcm.channels,
        language: state.language,
      },
    });
    if (receipt.segment_id !== detail.segment_id || !receipt.transcript?.trim()) {
      throw new Error("Local Whisper returned an invalid final transcript receipt.");
    }
    await updateGovernedTranscript(detail.segment_id, receipt.transcript.trim(), receipt);
    updateTranscriptElement(detail.segment_id, receipt.transcript.trim(), true);
    notify("Final local transcript is ready and remains editable before sending to the Agent.", "success");
    await window.__HOMESERVER_AGENT_AUDIO_V1__?.refresh?.().catch(() => null);
  } catch (error) {
    const message = String(error);
    notify(
      message.toLowerCase().includes("cancel")
        ? "Local Whisper transcription was cancelled."
        : `Local Whisper transcription failed: ${message}`,
      "warning",
    );
  } finally {
    state.active = null;
    await refreshStatus();
    state.busy = false;
    scheduleRender();
    drainQueuedSegment();
  }
}

async function cancelActive() {
  let transcriptionId = state.active?.transcriptionId
    || state.status?.active_transcription_id;
  if (!transcriptionId && state.active) {
    const status = await refreshStatus();
    transcriptionId = status?.active_transcription_id || null;
  }
  if (!transcriptionId) {
    notify("Local Whisper has not entered the cancellable decode stage yet.");
    return;
  }
  try {
    await invoke("homeserver_cancel_whisper_transcription", {
      transcriptionId,
    });
    notify("Local Whisper cancellation requested.");
  } catch (error) {
    notify(`Unable to cancel local Whisper: ${String(error)}`, "warning");
  }
}

function onProgress(event) {
  const payload = event?.payload;
  if (!payload?.segment_id || !payload.transcription_id) return;
  if (state.active && state.active.segmentId === payload.segment_id) {
    state.active.transcriptionId = payload.transcription_id;
    if (Number.isFinite(payload.progress) && payload.progress >= 0) {
      state.active.progress = Math.max(0, Math.min(100, payload.progress));
    }
  }
  if (payload.kind === "partial" && payload.partial_transcript?.trim()) {
    const partial = payload.partial_transcript.trim().slice(0, 20_000);
    state.partials.set(payload.segment_id, partial);
    updateTranscriptElement(payload.segment_id, partial, false);
  }
  if (payload.kind === "final" && payload.partial_transcript?.trim()) {
    state.partials.set(payload.segment_id, payload.partial_transcript.trim().slice(0, 20_000));
  }
  scheduleRender();
}

function renderWhisperPanel() {
  const audioPanel = document.querySelector("[data-agent-audio-panel]");
  if (!audioPanel) return;
  let panel = audioPanel.querySelector("[data-agent-whisper]");
  if (!panel) {
    panel = document.createElement("section");
    panel.className = "hs-agent-whisper";
    panel.dataset.agentWhisper = "true";
    const boundary = audioPanel.querySelector(".hs-agent-audio-boundary");
    (boundary || audioPanel.querySelector("header"))?.insertAdjacentElement("afterend", panel);
  }

  const ready = Boolean(state.status?.model_ready);
  const active = state.active;
  const modelHash = state.status?.model_sha256 || "";
  const progress = active?.progress || 0;
  panel.innerHTML = `
    <div class="hs-agent-whisper-head">
      <div>
        <span>Local transcription</span>
        <strong>whisper.cpp</strong>
      </div>
      <em data-ready="${ready}">${ready ? "model verified" : "model required"}</em>
    </div>
    <p>Transcription runs inside Control Center from 16 kHz mono PCM. Raw audio is ephemeral and no cloud speech fallback is permitted.</p>
    <label>
      <span>Trusted model SHA-256</span>
      <input type="text" maxlength="64" autocomplete="off" spellcheck="false" data-agent-whisper-sha value="${escapeHtml(state.expectedSha256)}" placeholder="64-character model hash" ${state.importing || active ? "disabled" : ""} />
    </label>
    <label class="compact">
      <span>Language</span>
      <select data-agent-whisper-language ${active ? "disabled" : ""}>
        <option value="en" ${state.language === "en" ? "selected" : ""}>English</option>
        <option value="auto" ${state.language === "auto" ? "selected" : ""}>Auto detect</option>
      </select>
    </label>
    <div class="hs-agent-whisper-actions">
      <button type="button" data-agent-whisper-import ${!validSha256(state.expectedSha256) || state.importing || state.busy || active || state.queue.length ? "disabled" : ""}>${state.importing ? "Importing…" : ready ? "Replace model" : "Import model"}</button>
      <button type="button" data-agent-whisper-remove ${!ready || state.importing || state.busy || active || state.queue.length ? "disabled" : ""}>Remove model</button>
      <button type="button" class="danger" data-agent-whisper-cancel ${!active ? "disabled" : ""}>Cancel transcription</button>
    </div>
    ${modelHash ? `<small>Model ${escapeHtml(modelHash.slice(0, 16))}… · verified again before each transcription</small>` : ""}
    ${active ? `<div class="hs-agent-whisper-progress"><i style="width:${progress}%"></i><span>${progress}%</span></div>` : ""}
    ${state.queue.length ? `<small>${state.queue.length} utterance${state.queue.length === 1 ? "" : "s"} queued locally · ${state.queue.byteLength.toLocaleString()} bytes ephemeral</small>` : ""}
    ${state.notice ? `<div class="hs-agent-whisper-notice" data-kind="${escapeHtml(state.notice.kind)}">${escapeHtml(state.notice.message)}</div>` : ""}
  `;
}

function hydratePartials() {
  for (const [segmentId, partial] of state.partials) {
    const textarea = document.querySelector(
      `[data-agent-audio-transcript="${CSS.escape(segmentId)}"]`,
    );
    if (textarea && !textarea.readOnly && !textarea.value.trim()) {
      textarea.value = partial;
      textarea.dataset.localWhisper = "partial";
    }
  }
}

function scheduleRender() {
  if (scheduled) return;
  scheduled = true;
  window.requestAnimationFrame(() => {
    scheduled = false;
    renderWhisperPanel();
    hydratePartials();
  });
}

function onClick(event) {
  const button = event.target.closest?.("button");
  if (!button) return;
  if (button.hasAttribute("data-agent-whisper-import")) void importModel();
  if (button.hasAttribute("data-agent-whisper-remove")) void removeModel();
  if (button.hasAttribute("data-agent-whisper-cancel")) void cancelActive();
}

function onInput(event) {
  if (event.target.matches?.("[data-agent-whisper-sha]")) {
    state.expectedSha256 = event.target.value.replace(/[^a-fA-F0-9]/g, "").slice(0, 64);
    scheduleRender();
  }
  if (event.target.matches?.("[data-agent-whisper-language]")) {
    state.language = event.target.value === "auto" ? "auto" : "en";
  }
}

async function initialize() {
  document.addEventListener("click", onClick);
  document.addEventListener("input", onInput);
  window.addEventListener("homeserver:vad-segment-finalized", (event) => {
    void transcribeLocalSegment(event.detail);
  });
  window.addEventListener("homeserver:rendered", scheduleRender);
  window.addEventListener("homeserver-agent-route", scheduleRender);
  unlistenProgress = await listen("homeserver-whisper-progress", onProgress);
  const app = document.querySelector("#app");
  if (app) {
    observer = new MutationObserver(scheduleRender);
    observer.observe(app, { childList: true, subtree: true });
  }
  await refreshStatus();
  scheduleRender();
}

window.addEventListener("pagehide", () => {
  state.queue.clear();
  unlistenProgress?.();
  observer?.disconnect();
});

void initialize();

window.__HOMESERVER_AGENT_WHISPER_V1__ = {
  refresh: refreshStatus,
  cancel: cancelActive,
  status: () => state.status,
};
