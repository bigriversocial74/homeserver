import { invoke } from "@tauri-apps/api/core";
import "./homeserver-agent-audio.css";

const state = {
  panelOpen: false,
  busy: false,
  status: null,
  stream: null,
  recorder: null,
  chunks: [],
  session: null,
  mode: null,
  startedAt: 0,
  timer: null,
  devices: [],
  selectedDeviceId: "",
  localRecordings: [],
  pendingLink: null,
};

let observer = null;
let scheduled = false;

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function humanize(value) {
  return String(value || "ready").replaceAll("_", " ");
}

function isAgentRoute() {
  return (window.location.hash.replace("#", "") || "agent") === "agent";
}

function supported() {
  return Boolean(navigator.mediaDevices?.getUserMedia && window.MediaRecorder && window.crypto?.subtle);
}

function recording() {
  return Boolean(state.recorder && state.recorder.state !== "inactive");
}

function activeThreadId() {
  return document.querySelector(".hs-chat-thread.active")?.dataset.chatThread || null;
}

function formatDuration(milliseconds) {
  const totalSeconds = Math.max(0, Math.floor(Number(milliseconds || 0) / 1000));
  const minutes = Math.floor(totalSeconds / 60).toString().padStart(2, "0");
  const seconds = (totalSeconds % 60).toString().padStart(2, "0");
  return `${minutes}:${seconds}`;
}

function currentStatus() {
  if (recording()) return "recording";
  if (state.busy) return "starting";
  return state.status?.active_session?.state || state.status?.host_state || "ready";
}

async function audioAction(action, fields = {}) {
  return invoke("homeserver_agent_integration_action", {
    request: { action, ...fields },
  });
}

async function refreshStatus() {
  try {
    state.status = await audioAction("audio_status");
  } catch {
    state.status = {
      host_state: "unavailable",
      active_session: null,
      sessions: [],
      segments: [],
      events: [],
    };
  }
}

async function refreshDevices() {
  if (!supported()) return;
  try {
    const devices = await navigator.mediaDevices.enumerateDevices();
    state.devices = devices.filter((device) => device.kind === "audioinput");
    if (state.selectedDeviceId && !state.devices.some((device) => device.deviceId === state.selectedDeviceId)) {
      state.selectedDeviceId = "";
    }
    if (!state.selectedDeviceId) state.selectedDeviceId = state.devices[0]?.deviceId || "";
  } catch {
    state.devices = [];
  }
}

function recorderMimeType() {
  for (const type of ["audio/webm;codecs=opus", "audio/webm", "audio/mp4"]) {
    if (MediaRecorder.isTypeSupported?.(type)) return type;
  }
  return "";
}

function stopTracks() {
  state.stream?.getTracks().forEach((track) => track.stop());
  state.stream = null;
}

function clearTimer() {
  if (state.timer) window.clearInterval(state.timer);
  state.timer = null;
}

function updateElapsed() {
  const element = document.querySelector("[data-agent-audio-elapsed]");
  if (element && state.startedAt) element.textContent = formatDuration(Date.now() - state.startedAt);
}

async function sha256(blob) {
  const digest = await crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function localRecording(segmentId) {
  return state.localRecordings.find((recordingItem) => recordingItem.segmentId === segmentId);
}

function statusSegments() {
  return Array.isArray(state.status?.segments) ? state.status.segments.slice(0, 12) : [];
}

function renderSegment(segment) {
  const local = localRecording(segment.segment_id);
  const title = segment.transcript || `Recording ${segment.sequence_no}`;
  return `<article class="hs-agent-audio-recording">
    <div class="hs-agent-audio-recording-head">
      <div><strong>${escapeHtml(title)}</strong><span>${escapeHtml(formatDuration(segment.duration_ms))} · ${escapeHtml(humanize(segment.state))}</span></div>
      <button type="button" data-agent-audio-delete="${escapeHtml(segment.session_id)}">Delete</button>
    </div>
    ${local ? `<audio controls preload="metadata" src="${escapeHtml(local.url)}"></audio>` : '<small>Raw audio was ephemeral and is not retained after this Control Center session.</small>'}
    <div class="hs-agent-audio-transcript">
      <textarea rows="2" maxlength="20000" data-agent-audio-transcript="${escapeHtml(segment.segment_id)}" placeholder="Add or correct the transcript…">${escapeHtml(segment.transcript || "")}</textarea>
      <button type="button" data-agent-audio-send="${escapeHtml(segment.segment_id)}">Send to Agent</button>
    </div>
  </article>`;
}

function renderPanel() {
  const segments = statusSegments();
  const options = state.devices.length
    ? state.devices
        .map(
          (device, index) => `<option value="${escapeHtml(device.deviceId)}" ${device.deviceId === state.selectedDeviceId ? "selected" : ""}>${escapeHtml(device.label || `Microphone ${index + 1}`)}</option>`,
        )
        .join("")
    : '<option value="">Default microphone</option>';
  return `<section class="hs-agent-audio-panel" data-agent-audio-panel ${state.panelOpen ? "" : "hidden"}>
    <header>
      <div><span>HomeServer Ears</span><strong>Listening and recordings</strong><p>Microphone access begins only after local permission. Raw audio remains ephemeral; HomeServer stores governed status, hashes, events, and transcripts.</p></div>
      <button type="button" data-agent-audio-close aria-label="Close listening panel">×</button>
    </header>
    <div class="hs-agent-audio-state"><i class="${recording() ? "active" : ""}"></i><strong>${escapeHtml(humanize(currentStatus()))}</strong><span data-agent-audio-elapsed>${recording() ? formatDuration(Date.now() - state.startedAt) : "00:00"}</span></div>
    <label class="hs-agent-audio-device"><span>Microphone</span><select data-agent-audio-device>${options}</select></label>
    <div class="hs-agent-audio-actions">
      <button type="button" data-agent-audio-mode="push_to_talk" ${recording() || state.busy ? "disabled" : ""}>Push to talk</button>
      <button type="button" data-agent-audio-mode="live_conversation" ${recording() || state.busy ? "disabled" : ""}>Live conversation</button>
      <button type="button" data-agent-audio-mode="voice_note" ${recording() || state.busy ? "disabled" : ""}>Voice note</button>
      <button type="button" class="danger" data-agent-audio-stop ${recording() ? "" : "disabled"}>Stop</button>
    </div>
    <div class="hs-agent-audio-boundary"><strong>Local-first foundation</strong><span>Cloud speech recognition is disabled. Local VAD and Whisper transcription are the next Phase 23 milestones.</span></div>
    <div class="hs-agent-audio-list">
      <div class="hs-agent-audio-list-title"><strong>Recent recordings</strong><span>${segments.length}</span></div>
      ${segments.length ? segments.map(renderSegment).join("") : '<div class="hs-agent-audio-empty">No Agent Chat recordings yet.</div>'}
    </div>
  </section>`;
}

function notify(message, kind = "info") {
  window.dispatchEvent(
    new CustomEvent("homeserver:agent-audio-notice", {
      detail: { message, kind },
    }),
  );
  const panel = document.querySelector("[data-agent-audio-panel]");
  if (!panel) return;
  let notice = panel.querySelector(".hs-agent-audio-notice");
  if (!notice) {
    notice = document.createElement("div");
    notice.className = "hs-agent-audio-notice";
    panel.prepend(notice);
  }
  notice.dataset.kind = kind;
  notice.textContent = message;
}

async function setSessionState(nextState, detail = {}, failureCode = null) {
  if (!state.session?.session_id) return null;
  return audioAction("audio_set_state", {
    session_id: state.session.session_id,
    state: nextState,
    failure_code: failureCode,
    detail,
  });
}

async function startCapture(mode) {
  if (state.busy || recording()) return;
  if (!supported()) {
    notify("This Control Center runtime does not expose microphone recording.", "warning");
    return;
  }
  state.busy = true;
  state.panelOpen = true;
  decorate(true);
  try {
    const audioConstraint = state.selectedDeviceId
      ? { deviceId: { exact: state.selectedDeviceId } }
      : true;
    const stream = await navigator.mediaDevices.getUserMedia({ audio: audioConstraint, video: false });
    state.stream = stream;
    await refreshDevices();
    const track = stream.getAudioTracks()[0];
    const settings = track?.getSettings?.() || {};
    state.session = await audioAction("audio_start_session", {
      thread_id: activeThreadId(),
      mode,
      retention_mode: "ephemeral",
      input_device_id: settings.deviceId || state.selectedDeviceId || null,
      input_device_label: track?.label || null,
      microphone_authorized: true,
      recording_authorized: true,
    });
    const mimeType = recorderMimeType();
    state.recorder = mimeType ? new MediaRecorder(stream, { mimeType }) : new MediaRecorder(stream);
    state.mode = mode;
    state.chunks = [];
    state.startedAt = Date.now();
    state.recorder.addEventListener("dataavailable", (event) => {
      if (event.data?.size) state.chunks.push(event.data);
    });
    state.recorder.addEventListener("stop", () => void finalizeCapture());
    await setSessionState("listening", { mode, microphone_label: track?.label || null });
    state.recorder.start(500);
    state.timer = window.setInterval(updateElapsed, 250);
    notify(
      mode === "live_conversation"
        ? "Live conversation is listening."
        : mode === "voice_note"
          ? "Voice note recording started."
          : "Push-to-talk recording started.",
      "success",
    );
  } catch (error) {
    if (state.session) await setSessionState("failed", {}, "capture_start_failed").catch(() => null);
    clearTimer();
    stopTracks();
    state.recorder = null;
    state.session = null;
    notify(`Microphone capture failed: ${String(error)}`, "warning");
  } finally {
    state.busy = false;
    decorate(true);
  }
}

async function stopCapture() {
  if (!recording() || state.busy) return;
  state.busy = true;
  clearTimer();
  try {
    await setSessionState("finalizing_transcript", {
      duration_ms: Date.now() - state.startedAt,
    });
    state.recorder.stop();
    stopTracks();
    notify("Finalizing local recording metadata.");
  } catch (error) {
    await setSessionState("failed", {}, "capture_stop_failed").catch(() => null);
    stopTracks();
    state.recorder = null;
    state.session = null;
    state.busy = false;
    notify(`Unable to stop recording cleanly: ${String(error)}`, "warning");
    decorate(true);
  }
}

async function finalizeCapture() {
  const session = state.session;
  const durationMs = Math.max(0, Date.now() - state.startedAt);
  const chunks = [...state.chunks];
  try {
    const mimeType = state.recorder?.mimeType || chunks[0]?.type || "audio/webm";
    const blob = new Blob(chunks, { type: mimeType });
    const segment = await audioAction("audio_finalize_segment", {
      session_id: session.session_id,
      mime_type: blob.type || "application/octet-stream",
      duration_ms: durationMs,
      byte_length: blob.size,
      content_sha256: await sha256(blob),
      transcript: null,
    });
    const url = URL.createObjectURL(blob);
    state.localRecordings.unshift({
      segmentId: segment.segment_id,
      sessionId: session.session_id,
      url,
      blob,
    });
    await setSessionState("stopped", {
      segment_id: segment.segment_id,
      raw_audio_retained: false,
    });
    await refreshStatus();
    notify("Recording complete. Add or correct the transcript, then send it through Agent Chat.", "success");
  } catch (error) {
    await setSessionState("failed", {}, "segment_finalize_failed").catch(() => null);
    notify(`Recording finalization failed: ${String(error)}`, "warning");
  } finally {
    clearTimer();
    stopTracks();
    state.recorder = null;
    state.chunks = [];
    state.session = null;
    state.mode = null;
    state.startedAt = 0;
    state.busy = false;
    decorate(true);
  }
}

async function linkSubmittedTranscript() {
  const pending = state.pendingLink;
  if (!pending) return;
  for (let attempt = 0; attempt < 12; attempt += 1) {
    await new Promise((resolve) => window.setTimeout(resolve, 400));
    try {
      const workspace = await invoke("homeserver_agent_workspace");
      const match = [...(workspace.messages || [])]
        .reverse()
        .find((message) => message.role === "user" && message.content === pending.transcript);
      if (!match) continue;
      await audioAction("audio_update_transcript", {
        segment_id: pending.segmentId,
        transcript: pending.transcript,
        linked_message_id: match.message_id,
      });
      state.pendingLink = null;
      await refreshStatus();
      decorate(true);
      return;
    } catch {
      // Agent Chat may still be saving the turn; retry within the bounded window.
    }
  }
}

async function sendTranscript(segmentId) {
  const input = document.querySelector(`[data-agent-audio-transcript="${CSS.escape(segmentId)}"]`);
  const transcript = input?.value?.trim() || "";
  if (!transcript) {
    notify("Add a transcript before sending this recording to Agent Chat.", "warning");
    return;
  }
  await audioAction("audio_update_transcript", {
    segment_id: segmentId,
    transcript,
    linked_message_id: null,
  });
  const composer = document.querySelector("#hs-chat-input");
  const form = document.querySelector("#homeserver-chat-form");
  if (!composer || !form) {
    notify("Agent Chat composer is unavailable.", "warning");
    return;
  }
  state.pendingLink = { segmentId, transcript };
  state.panelOpen = false;
  composer.value = transcript;
  composer.dispatchEvent(new Event("input", { bubbles: true }));
  form.requestSubmit();
  void linkSubmittedTranscript();
}

async function deleteSession(sessionId) {
  if (!sessionId || !window.confirm("Delete this completed audio session and its transcript metadata?")) return;
  await audioAction("audio_delete_session", {
    session_id: sessionId,
    confirmation: "DELETE AUDIO SESSION",
  });
  state.localRecordings = state.localRecordings.filter((recordingItem) => {
    if (recordingItem.sessionId !== sessionId) return true;
    URL.revokeObjectURL(recordingItem.url);
    return false;
  });
  await refreshStatus();
  notify("Audio session deleted.", "success");
  decorate(true);
}

function bindPanel(panel) {
  panel.querySelector("[data-agent-audio-close]")?.addEventListener("click", () => {
    state.panelOpen = false;
    decorate(true);
  });
  panel.querySelector("[data-agent-audio-device]")?.addEventListener("change", (event) => {
    state.selectedDeviceId = event.currentTarget.value || "";
  });
  panel.querySelectorAll("[data-agent-audio-mode]").forEach((button) => {
    button.addEventListener("click", () => void startCapture(button.dataset.agentAudioMode || "push_to_talk"));
  });
  panel.querySelector("[data-agent-audio-stop]")?.addEventListener("click", () => void stopCapture());
  panel.querySelectorAll("[data-agent-audio-send]").forEach((button) => {
    button.addEventListener("click", () => void sendTranscript(button.dataset.agentAudioSend || ""));
  });
  panel.querySelectorAll("[data-agent-audio-delete]").forEach((button) => {
    button.addEventListener("click", () => void deleteSession(button.dataset.agentAudioDelete || ""));
  });
}

function decorate(force = false) {
  if (!isAgentRoute()) return;
  const form = document.querySelector("#homeserver-chat-form");
  if (!form) return;
  const alreadyDecorated = form.dataset.agentAudio === "true";
  if (alreadyDecorated && !force) return;

  form.querySelectorAll("[data-agent-audio-owned]").forEach((element) => element.remove());
  form.dataset.agentAudio = "true";

  const tools = form.querySelector(".hs-chat-composer-tools");
  const connectionButton = form.querySelector("#hs-chat-connection-toggle");
  if (tools) {
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "hs-chat-tool-button hs-agent-audio-toggle";
    toggle.dataset.agentAudioOwned = "true";
    toggle.innerHTML = `Ears <span>${escapeHtml(humanize(currentStatus()))}</span>`;
    toggle.addEventListener("click", () => {
      state.panelOpen = !state.panelOpen;
      void refreshDevices().then(() => decorate(true));
    });
    if (connectionButton) tools.insertBefore(toggle, connectionButton);
    else tools.append(toggle);
  }

  const panelWrapper = document.createElement("div");
  panelWrapper.dataset.agentAudioOwned = "true";
  panelWrapper.innerHTML = renderPanel();
  const inputShell = form.querySelector(".hs-chat-input-shell");
  if (inputShell) form.insertBefore(panelWrapper, inputShell);
  else form.append(panelWrapper);
  const panel = panelWrapper.querySelector("[data-agent-audio-panel]");
  if (panel) bindPanel(panel);

  if (inputShell) {
    const mic = document.createElement("button");
    mic.type = "button";
    mic.className = `hs-agent-audio-mic ${recording() ? "active" : ""}`;
    mic.dataset.agentAudioOwned = "true";
    mic.setAttribute("aria-label", recording() ? "Stop recording" : "Start push-to-talk recording");
    mic.title = recording() ? "Stop recording" : "Push to talk";
    mic.textContent = recording() ? "■" : "●";
    mic.addEventListener("click", () => {
      if (recording()) void stopCapture();
      else void startCapture("push_to_talk");
    });
    inputShell.prepend(mic);
  }
}

function scheduleDecorate() {
  if (scheduled) return;
  scheduled = true;
  window.requestAnimationFrame(() => {
    scheduled = false;
    decorate();
  });
}

async function initialize() {
  await Promise.all([refreshStatus(), refreshDevices()]);
  scheduleDecorate();
  const app = document.querySelector("#app");
  if (app) {
    observer = new MutationObserver(scheduleDecorate);
    observer.observe(app, { childList: true, subtree: true });
  }
}

window.addEventListener("homeserver:rendered", scheduleDecorate);
window.addEventListener("homeserver-agent-route", scheduleDecorate);
window.addEventListener("hashchange", scheduleDecorate);
window.addEventListener("beforeunload", () => {
  clearTimer();
  stopTracks();
  state.localRecordings.forEach((recordingItem) => URL.revokeObjectURL(recordingItem.url));
  observer?.disconnect();
});

void initialize();

window.__HOMESERVER_AGENT_AUDIO_V1__ = {
  refresh: async () => {
    await refreshStatus();
    decorate(true);
  },
  stop: stopCapture,
};
