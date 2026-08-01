import { invoke } from "@tauri-apps/api/core";
import "./homeserver-agent-audio.css";

const MAX_CAPTURE_MS = 30 * 60 * 1000;
const CAPTURE_STOP_MS = MAX_CAPTURE_MS - 10 * 1000;
const MAX_CAPTURE_BYTES = 256 * 1024 * 1024;
const CAPTURE_STOP_BYTES = 250 * 1024 * 1024;
const MAX_LOCAL_RECORDINGS = 12;
const LINK_RETRY_COUNT = 24;
const LINK_RETRY_DELAY_MS = 500;

const state = {
  panelOpen: false,
  busy: false,
  operation: null,
  notice: null,
  status: null,
  stream: null,
  recorder: null,
  chunks: [],
  chunkBytes: 0,
  session: null,
  mode: null,
  startedAt: 0,
  timer: null,
  devices: [],
  selectedDeviceId: "",
  localRecordings: [],
  pendingLink: null,
  sendingSegmentId: null,
  intentionalStop: false,
  discardFailureCode: null,
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
  return Boolean(
    navigator.mediaDevices?.getUserMedia
      && window.MediaRecorder
      && window.crypto?.subtle,
  );
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
  if (state.operation) return state.operation;
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
      capabilities: {},
    };
  }
}

async function refreshDevices() {
  if (!supported()) return;
  try {
    const devices = await navigator.mediaDevices.enumerateDevices();
    state.devices = devices.filter((device) => device.kind === "audioinput");
    if (
      state.selectedDeviceId
      && !state.devices.some((device) => device.deviceId === state.selectedDeviceId)
    ) {
      state.selectedDeviceId = "";
    }
    if (!state.selectedDeviceId) {
      state.selectedDeviceId = state.devices[0]?.deviceId || "";
    }
  } catch {
    state.devices = [];
  }
}

async function reconcileOrphanedSession() {
  const activeSession = state.status?.active_session;
  if (!activeSession?.session_id || recording() || state.session) return;
  try {
    await audioAction("audio_set_state", {
      session_id: activeSession.session_id,
      state: "failed",
      failure_code: "control_center_capture_host_lost",
      detail: { capture_host: "control_center_webview" },
    });
    await refreshStatus();
  } catch {
    // A native Audio Host may own this session in a later phase.
  }
}

function recorderMimeType() {
  for (const type of [
    "audio/webm;codecs=opus",
    "audio/webm",
    "audio/mp4",
  ]) {
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
  const elapsed = state.startedAt ? Date.now() - state.startedAt : 0;
  const element = document.querySelector("[data-agent-audio-elapsed]");
  if (element) element.textContent = formatDuration(elapsed);
  if (elapsed >= CAPTURE_STOP_MS && recording() && !state.busy) {
    notify("The 30-minute local recording limit was reached. Finalizing this recording.", "info");
    void stopCapture("duration_limit");
  }
}

async function sha256(blob) {
  const digest = await crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function releaseLocalRecording(recordingItem) {
  if (recordingItem?.url) URL.revokeObjectURL(recordingItem.url);
}

function rememberLocalRecording(recordingItem) {
  state.localRecordings.unshift(recordingItem);
  while (state.localRecordings.length > MAX_LOCAL_RECORDINGS) {
    releaseLocalRecording(state.localRecordings.pop());
  }
}

function localRecording(segmentId) {
  return state.localRecordings.find(
    (recordingItem) => recordingItem.segmentId === segmentId,
  );
}

function statusSegments() {
  return Array.isArray(state.status?.segments)
    ? state.status.segments.slice(0, 12)
    : [];
}

function renderSegment(segment) {
  const local = localRecording(segment.segment_id);
  const committed = Boolean(segment.linked_message_id) || segment.state === "committed";
  const transcript = segment.transcript || "";
  const title = transcript
    ? `${transcript.slice(0, 120)}${transcript.length > 120 ? "…" : ""}`
    : `Recording ${segment.sequence_no}`;
  const sending = state.sendingSegmentId === segment.segment_id;
  return `<article class="hs-agent-audio-recording">
    <div class="hs-agent-audio-recording-head">
      <div><strong>${escapeHtml(title)}</strong><span>${escapeHtml(formatDuration(segment.duration_ms))} · ${escapeHtml(humanize(segment.state))}</span></div>
      <button type="button" data-agent-audio-delete="${escapeHtml(segment.session_id)}">Delete session</button>
    </div>
    ${local ? `<audio controls preload="metadata" src="${escapeHtml(local.url)}"></audio>` : '<small>Raw audio was ephemeral and is not retained after this Control Center session.</small>'}
    <div class="hs-agent-audio-transcript">
      <textarea rows="2" maxlength="20000" data-agent-audio-transcript="${escapeHtml(segment.segment_id)}" placeholder="Add or correct the transcript…" ${committed ? "readonly" : ""}>${escapeHtml(transcript)}</textarea>
      <button type="button" data-agent-audio-send="${escapeHtml(segment.segment_id)}" ${committed || sending ? "disabled" : ""}>${committed ? "Sent to Agent" : sending ? "Sending…" : "Send to Agent"}</button>
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
  const notice = state.notice
    ? `<div class="hs-agent-audio-notice" data-kind="${escapeHtml(state.notice.kind)}" role="status">${escapeHtml(state.notice.message)}</div>`
    : "";
  return `<section class="hs-agent-audio-panel" data-agent-audio-panel ${state.panelOpen ? "" : "hidden"} aria-label="HomeServer Ears">
    ${notice}
    <header>
      <div><span>HomeServer Ears</span><strong>Listening and recordings</strong><p>Microphone access begins only after local permission. Raw audio remains ephemeral; HomeServer stores governed status, hashes, events, and transcripts.</p></div>
      <button type="button" data-agent-audio-close aria-label="Close listening panel">×</button>
    </header>
    <div class="hs-agent-audio-state" aria-live="polite"><i class="${recording() ? "active" : ""}"></i><strong>${escapeHtml(humanize(currentStatus()))}</strong><span data-agent-audio-elapsed>${recording() ? formatDuration(Date.now() - state.startedAt) : "00:00"}</span></div>
    <label class="hs-agent-audio-device"><span>Microphone</span><select data-agent-audio-device ${recording() || state.busy ? "disabled" : ""}>${options}</select></label>
    <div class="hs-agent-audio-actions">
      <button type="button" data-agent-audio-mode="push_to_talk" ${recording() || state.busy ? "disabled" : ""}>Push to talk</button>
      <button type="button" data-agent-audio-mode="live_conversation" ${recording() || state.busy ? "disabled" : ""}>Live conversation</button>
      <button type="button" data-agent-audio-mode="voice_note" ${recording() || state.busy ? "disabled" : ""}>Voice note</button>
      <button type="button" class="danger" data-agent-audio-stop ${recording() && !state.busy ? "" : "disabled"}>Stop</button>
    </div>
    <div class="hs-agent-audio-boundary"><strong>Local-first foundation</strong><span>Cloud speech recognition is disabled. Local VAD and Whisper transcription are the next Phase 23 milestones. Recordings are capped at 30 minutes and 256 MB in memory.</span></div>
    <div class="hs-agent-audio-list">
      <div class="hs-agent-audio-list-title"><strong>Recent recordings</strong><span>${segments.length}</span></div>
      ${segments.length ? segments.map(renderSegment).join("") : '<div class="hs-agent-audio-empty">No Agent Chat recordings yet.</div>'}
    </div>
  </section>`;
}

function notify(message, kind = "info") {
  state.notice = { message: String(message), kind };
  window.dispatchEvent(
    new CustomEvent("homeserver:agent-audio-notice", {
      detail: state.notice,
    }),
  );
  const notice = document.querySelector(".hs-agent-audio-notice");
  if (notice) {
    notice.dataset.kind = kind;
    notice.textContent = String(message);
  }
}

async function setSessionState(nextState, detail = {}, failureCode = null) {
  if (!state.session?.session_id) return null;
  const session = await audioAction("audio_set_state", {
    session_id: state.session.session_id,
    state: nextState,
    failure_code: failureCode,
    detail,
  });
  state.session = session;
  return session;
}

function resetCaptureRuntime() {
  state.recorder = null;
  state.chunks = [];
  state.chunkBytes = 0;
  state.session = null;
  state.mode = null;
  state.startedAt = 0;
  state.intentionalStop = false;
  state.discardFailureCode = null;
  state.operation = null;
}

async function startCapture(mode) {
  if (state.busy || recording()) return;
  if (!supported()) {
    notify("This Control Center runtime does not expose microphone recording.", "warning");
    return;
  }

  state.busy = true;
  state.operation = "starting";
  state.notice = null;
  state.panelOpen = true;
  decorate(true);

  try {
    const audioConstraint = state.selectedDeviceId
      ? { deviceId: { exact: state.selectedDeviceId } }
      : true;
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: audioConstraint,
      video: false,
    });
    state.stream = stream;
    await refreshDevices();

    const track = stream.getAudioTracks()[0];
    if (!track) throw new Error("No microphone track was returned.");
    const settings = track.getSettings?.() || {};

    state.session = await audioAction("audio_start_session", {
      thread_id: activeThreadId(),
      mode,
      retention_mode: "transcript",
      input_device_id: settings.deviceId || state.selectedDeviceId || null,
      input_device_label: track.label || null,
      microphone_authorized: true,
      recording_authorized: true,
    });

    const mimeType = recorderMimeType();
    const recorder = mimeType
      ? new MediaRecorder(stream, { mimeType })
      : new MediaRecorder(stream);
    state.recorder = recorder;
    state.mode = mode;
    state.chunks = [];
    state.chunkBytes = 0;
    state.startedAt = Date.now();
    state.intentionalStop = false;
    state.discardFailureCode = null;

    recorder.addEventListener("dataavailable", (event) => {
      if (!event.data?.size) return;
      state.chunks.push(event.data);
      state.chunkBytes += event.data.size;
      if (state.chunkBytes >= CAPTURE_STOP_BYTES && recording() && !state.busy) {
        notify("The 256 MB local memory limit was reached. Finalizing this recording.", "info");
        void stopCapture("size_limit");
      }
    });
    recorder.addEventListener("stop", () => void finalizeCapture(), { once: true });
    track.addEventListener(
      "ended",
      () => {
        if (!state.intentionalStop && recording()) {
          void failCapture(
            "input_device_ended",
            "The selected microphone disconnected before the recording completed.",
          );
        }
      },
      { once: true },
    );

    await setSessionState("listening", {
      mode,
      microphone_label: track.label || null,
    });
    recorder.start(500);
    state.timer = window.setInterval(updateElapsed, 250);
    state.operation = null;
    notify(
      mode === "live_conversation"
        ? "Live conversation is listening."
        : mode === "voice_note"
          ? "Voice note recording started."
          : "Push-to-talk recording started.",
      "success",
    );
  } catch (error) {
    if (state.session) {
      await setSessionState(
        "failed",
        { capture_host: "control_center_webview" },
        "capture_start_failed",
      ).catch(() => null);
    }
    clearTimer();
    state.intentionalStop = true;
    stopTracks();
    resetCaptureRuntime();
    notify(`Microphone capture failed: ${String(error)}`, "warning");
  } finally {
    state.busy = false;
    decorate(true);
  }
}

async function stopCapture(reason = "user_stop") {
  if (!recording() || state.busy) return;
  state.busy = true;
  state.operation = "finalizing";
  clearTimer();
  decorate(true);

  try {
    await setSessionState("finalizing_transcript", {
      duration_ms: Date.now() - state.startedAt,
      stop_reason: reason,
    });
    state.intentionalStop = true;
    state.recorder.stop();
    notify("Finalizing local recording metadata.");
  } catch (error) {
    await failCapture(
      "capture_stop_failed",
      `Unable to stop recording cleanly: ${String(error)}`,
    );
  }
}

async function failCapture(failureCode, message) {
  clearTimer();
  state.busy = true;
  state.operation = "failing";
  state.intentionalStop = true;
  state.discardFailureCode = failureCode;
  notify(message, "warning");

  if (recording()) {
    try {
      state.recorder.stop();
      return;
    } catch {
      // Fall through to direct failure cleanup.
    }
  }

  await setSessionState(
    "failed",
    { capture_host: "control_center_webview" },
    failureCode,
  ).catch(() => null);
  stopTracks();
  resetCaptureRuntime();
  state.busy = false;
  decorate(true);
}

async function finalizeCapture() {
  const session = state.session;
  const durationMs = Math.max(0, Date.now() - state.startedAt);
  const chunks = [...state.chunks];
  const discardFailureCode = state.discardFailureCode;

  try {
    if (!session?.session_id) {
      throw new Error("The governed audio session is unavailable.");
    }

    if (discardFailureCode) {
      await setSessionState(
        "failed",
        { capture_host: "control_center_webview" },
        discardFailureCode,
      ).catch(() => null);
      return;
    }

    const mimeType = state.recorder?.mimeType || chunks[0]?.type || "audio/webm";
    const blob = new Blob(chunks, { type: mimeType });
    if (!blob.size || durationMs <= 0) {
      throw new Error("The microphone produced an empty recording.");
    }
    if (blob.size > MAX_CAPTURE_BYTES || durationMs > MAX_CAPTURE_MS + 2000) {
      throw new Error("The recording exceeded the local capture boundary.");
    }

    const segment = await audioAction("audio_finalize_segment", {
      session_id: session.session_id,
      mime_type: blob.type || "application/octet-stream",
      duration_ms: durationMs,
      byte_length: blob.size,
      content_sha256: await sha256(blob),
      transcript: null,
    });
    const url = URL.createObjectURL(blob);
    rememberLocalRecording({
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
    notify(
      "Recording complete. Add or correct the transcript, then send it through Agent Chat.",
      "success",
    );
  } catch (error) {
    await setSessionState(
      "failed",
      { capture_host: "control_center_webview" },
      "segment_finalize_failed",
    ).catch(() => null);
    notify(`Recording finalization failed: ${String(error)}`, "warning");
  } finally {
    clearTimer();
    state.intentionalStop = true;
    stopTracks();
    resetCaptureRuntime();
    state.busy = false;
    decorate(true);
  }
}

async function workspaceMessageSnapshot() {
  const workspace = await invoke("homeserver_agent_workspace");
  return {
    workspace,
    messageIds: new Set(
      (workspace.messages || [])
        .map((message) => message.message_id)
        .filter(Boolean),
    ),
  };
}

async function linkSubmittedTranscript(pending) {
  for (let attempt = 0; attempt < LINK_RETRY_COUNT; attempt += 1) {
    await new Promise((resolve) => window.setTimeout(resolve, LINK_RETRY_DELAY_MS));
    if (state.pendingLink?.token !== pending.token) return;

    try {
      const workspace = await invoke("homeserver_agent_workspace");
      const match = [...(workspace.messages || [])]
        .reverse()
        .find(
          (message) => message.role === "user"
            && message.content === pending.transcript
            && !pending.messageIds.has(message.message_id)
            && (!pending.threadId || message.thread_id === pending.threadId),
        );
      if (!match) continue;

      await audioAction("audio_update_transcript", {
        segment_id: pending.segmentId,
        transcript: pending.transcript,
        linked_message_id: match.message_id,
      });
      if (state.pendingLink?.token === pending.token) {
        state.pendingLink = null;
      }
      await refreshStatus();
      notify("Transcript linked to the new Agent Chat message.", "success");
      decorate(true);
      return;
    } catch {
      // Agent Chat may still be saving the new turn.
    }
  }

  if (state.pendingLink?.token === pending.token) {
    state.pendingLink = null;
    notify(
      "The transcript was sent, but its message linkage could not be verified. The transcript remains saved locally.",
      "warning",
    );
    decorate(true);
  }
}

async function sendTranscript(segmentId) {
  if (!segmentId || state.sendingSegmentId) return;
  const segment = statusSegments().find((item) => item.segment_id === segmentId);
  if (segment?.linked_message_id || segment?.state === "committed") {
    notify("This transcript is already linked to Agent Chat.", "info");
    return;
  }

  const input = document.querySelector(
    `[data-agent-audio-transcript="${CSS.escape(segmentId)}"]`,
  );
  const transcript = input?.value?.trim() || "";
  if (!transcript) {
    notify("Add a transcript before sending this recording to Agent Chat.", "warning");
    return;
  }

  const composer = document.querySelector("#hs-chat-input");
  const form = document.querySelector("#homeserver-chat-form");
  if (!composer || !form) {
    notify("Agent Chat composer is unavailable.", "warning");
    return;
  }
  const existingDraft = composer.value.trim();
  if (existingDraft && existingDraft !== transcript) {
    notify(
      "Agent Chat already contains an unsent draft. Clear or send that draft before transferring this transcript.",
      "warning",
    );
    return;
  }

  state.sendingSegmentId = segmentId;
  decorate(true);

  try {
    const { messageIds } = await workspaceMessageSnapshot();
    await audioAction("audio_update_transcript", {
      segment_id: segmentId,
      transcript,
      linked_message_id: null,
    });

    const pending = {
      token: crypto.randomUUID(),
      segmentId,
      transcript,
      messageIds,
      threadId: activeThreadId(),
    };
    state.pendingLink = pending;
    state.panelOpen = false;
    composer.value = transcript;
    composer.dispatchEvent(new Event("input", { bubbles: true }));
    form.requestSubmit();
    void linkSubmittedTranscript(pending);
  } catch (error) {
    notify(`Unable to send transcript: ${String(error)}`, "warning");
  } finally {
    state.sendingSegmentId = null;
    decorate(true);
  }
}

async function deleteSession(sessionId) {
  if (
    !sessionId
    || !window.confirm("Delete this completed audio session and its transcript metadata?")
  ) {
    return;
  }
  await audioAction("audio_delete_session", {
    session_id: sessionId,
    confirmation: "DELETE AUDIO SESSION",
  });
  state.localRecordings = state.localRecordings.filter((recordingItem) => {
    if (recordingItem.sessionId !== sessionId) return true;
    releaseLocalRecording(recordingItem);
    return false;
  });
  await refreshStatus();
  notify("Audio session deleted.", "success");
  decorate(true);
}

function runUiAction(action, label) {
  void Promise.resolve()
    .then(action)
    .catch((error) => {
      notify(`${label}: ${String(error)}`, "warning");
      decorate(true);
    });
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
    button.addEventListener("click", () => {
      runUiAction(
        () => startCapture(button.dataset.agentAudioMode || "push_to_talk"),
        "Unable to start recording",
      );
    });
  });
  panel.querySelector("[data-agent-audio-stop]")?.addEventListener("click", () => {
    runUiAction(() => stopCapture(), "Unable to stop recording");
  });
  panel.querySelectorAll("[data-agent-audio-send]").forEach((button) => {
    button.addEventListener("click", () => {
      runUiAction(
        () => sendTranscript(button.dataset.agentAudioSend || ""),
        "Unable to send transcript",
      );
    });
  });
  panel.querySelectorAll("[data-agent-audio-delete]").forEach((button) => {
    button.addEventListener("click", () => {
      runUiAction(
        () => deleteSession(button.dataset.agentAudioDelete || ""),
        "Unable to delete audio session",
      );
    });
  });
}

function transcriptDrafts(form) {
  return new Map(
    [...form.querySelectorAll("[data-agent-audio-transcript]")].map((textarea) => [
      textarea.dataset.agentAudioTranscript,
      textarea.value,
    ]),
  );
}

function restoreTranscriptDrafts(panel, drafts) {
  panel.querySelectorAll("[data-agent-audio-transcript]").forEach((textarea) => {
    const draft = drafts.get(textarea.dataset.agentAudioTranscript);
    if (draft !== undefined && !textarea.readOnly) textarea.value = draft;
  });
}

function decorate(force = false) {
  if (!isAgentRoute()) return;
  const form = document.querySelector("#homeserver-chat-form");
  if (!form) return;
  const alreadyDecorated = form.dataset.agentAudio === "true";
  if (alreadyDecorated && !force) return;

  const drafts = transcriptDrafts(form);
  form.querySelectorAll("[data-agent-audio-owned]").forEach((element) => element.remove());
  form.dataset.agentAudio = "true";

  const tools = form.querySelector(".hs-chat-composer-tools");
  const connectionButton = form.querySelector("#hs-chat-connection-toggle");
  if (tools) {
    const toggle = document.createElement("button");
    toggle.type = "button";
    toggle.className = "hs-chat-tool-button hs-agent-audio-toggle";
    toggle.dataset.agentAudioOwned = "true";
    toggle.setAttribute("aria-expanded", String(state.panelOpen));
    toggle.innerHTML = `Ears <span>${escapeHtml(humanize(currentStatus()))}</span>`;
    toggle.addEventListener("click", () => {
      state.panelOpen = !state.panelOpen;
      runUiAction(
        async () => {
          await refreshDevices();
          decorate(true);
        },
        "Unable to refresh microphones",
      );
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
  if (panel) {
    bindPanel(panel);
    restoreTranscriptDrafts(panel, drafts);
  }

  if (inputShell) {
    const mic = document.createElement("button");
    mic.type = "button";
    mic.className = `hs-agent-audio-mic ${recording() ? "active" : ""}`;
    mic.dataset.agentAudioOwned = "true";
    mic.disabled = state.busy;
    mic.setAttribute("aria-pressed", String(recording()));
    mic.setAttribute(
      "aria-label",
      recording() ? "Stop recording" : "Start push-to-talk recording",
    );
    mic.title = recording() ? "Stop recording" : "Push to talk";
    mic.textContent = recording() ? "■" : "●";
    mic.addEventListener("click", () => {
      runUiAction(
        () => (recording() ? stopCapture() : startCapture("push_to_talk")),
        recording() ? "Unable to stop recording" : "Unable to start recording",
      );
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

function abandonControlCenterCapture() {
  clearTimer();
  if (state.session?.session_id) {
    void audioAction("audio_set_state", {
      session_id: state.session.session_id,
      state: "failed",
      failure_code: "control_center_closed",
      detail: { capture_host: "control_center_webview" },
    }).catch(() => null);
  }
  state.intentionalStop = true;
  stopTracks();
  state.localRecordings.forEach(releaseLocalRecording);
  observer?.disconnect();
}

async function initialize() {
  await Promise.all([refreshStatus(), refreshDevices()]);
  await reconcileOrphanedSession();
  scheduleDecorate();

  const app = document.querySelector("#app");
  if (app) {
    observer = new MutationObserver(scheduleDecorate);
    observer.observe(app, { childList: true, subtree: true });
  }

  navigator.mediaDevices?.addEventListener?.("devicechange", () => {
    runUiAction(
      async () => {
        await refreshDevices();
        decorate(true);
      },
      "Unable to refresh microphones",
    );
  });
}

window.addEventListener("homeserver:rendered", scheduleDecorate);
window.addEventListener("homeserver-agent-route", scheduleDecorate);
window.addEventListener("hashchange", scheduleDecorate);
window.addEventListener("pagehide", abandonControlCenterCapture);
window.addEventListener("beforeunload", abandonControlCenterCapture);

void initialize();

window.__HOMESERVER_AGENT_AUDIO_V1__ = {
  refresh: async () => {
    await refreshStatus();
    decorate(true);
  },
  stop: stopCapture,
};
