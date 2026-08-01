from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(path: str, old: str, new: str) -> None:
    file_path = ROOT / path
    text = file_path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{path}: expected exactly one match, found {count}: {old[:120]!r}")
    file_path.write_text(text.replace(old, new, 1), encoding="utf-8")


def replace_regex_once(path: str, pattern: str, replacement: str) -> None:
    file_path = ROOT / path
    text = file_path.read_text(encoding="utf-8")
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.DOTALL)
    if count != 1:
        raise RuntimeError(f"{path}: expected one regex match, found {count}: {pattern}")
    file_path.write_text(updated, encoding="utf-8")


def append_once(path: str, marker: str, content: str) -> None:
    file_path = ROOT / path
    text = file_path.read_text(encoding="utf-8")
    if marker in text:
        return
    file_path.write_text(text.rstrip() + "\n\n" + content.strip() + "\n", encoding="utf-8")


# Service registration.
replace_once(
    "crates/homeserver-service/src/app.rs",
    '#[path = "activity.rs"]\nmod activity;\n',
    '#[path = "audio_runtime.rs"]\nmod audio_runtime;\n\n#[path = "activity.rs"]\nmod activity;\n',
)
replace_once(
    "crates/homeserver-service/src/app.rs",
    "    agent_integrations, agent_runtime, backup, config::AppConfig, database, document_extraction,\n",
    "    agent_integrations, agent_runtime, audio_runtime, backup, config::AppConfig, database, document_extraction,\n",
)
replace_once(
    "crates/homeserver-service/src/app.rs",
    "    agent_runtime::initialize(&connection)?;\n    agent_integrations::initialize(&connection)?;\n    mcp_runtime::initialize(&connection)?;\n",
    "    agent_runtime::initialize(&connection)?;\n    agent_integrations::initialize(&connection)?;\n    audio_runtime::initialize(&connection)?;\n    mcp_runtime::initialize(&connection)?;\n",
)
replace_once(
    "crates/homeserver-service/src/app.rs",
    "            .merge(agent_runtime::router(state.clone()))\n            .merge(agent_integrations::router(state.clone()))\n            .merge(mcp_runtime::router(state.clone())),\n",
    "            .merge(agent_runtime::router(state.clone()))\n            .merge(agent_integrations::router(state.clone()))\n            .merge(audio_runtime::router(state.clone()))\n            .merge(mcp_runtime::router(state.clone())),\n",
)
replace_once(
    "crates/homeserver-service/src/app.rs",
    "                        if let Err(error) = agent_integrations::maintain_history(&connection) {\n                            warn!(?error, \"scheduled unified Agent retention failed\");\n                        }\n",
    "                        if let Err(error) = agent_integrations::maintain_history(&connection) {\n                            warn!(?error, \"scheduled unified Agent retention failed\");\n                        }\n                        if let Err(error) = audio_runtime::maintain_history(&connection) {\n                            warn!(?error, \"scheduled Agent audio retention failed\");\n                        }\n",
)

# Fix source formatting details before rustfmt.
replace_once(
    "crates/homeserver-service/src/audio_runtime.rs",
    "use anyhow::{bail, ensure, Context, Result};",
    "use anyhow::{ensure, Context, Result};",
)
replace_once(
    "crates/homeserver-service/src/audio_runtime.rs",
    "            i64::from(terminal),",
    "            if terminal { 1_i64 } else { 0_i64 },",
)

# Tauri command registration.
replace_once("src-tauri/src/lib.rs", "mod agent;\n", "mod agent;\nmod audio;\n")
replace_once(
    "src-tauri/src/lib.rs",
    "            agent::homeserver_open_agent_authorization,\n            operational::homeserver_operational_data,\n",
    "            agent::homeserver_open_agent_authorization,\n            audio::homeserver_audio_status,\n            audio::homeserver_audio_action,\n            operational::homeserver_operational_data,\n",
)

# Agent Chat state.
replace_once(
    "src/homeserver-agent-chat.js",
    'let shellHealth = { service: "online", models: "unknown" };\n',
    '''let shellHealth = { service: "online", models: "unknown" };
let audioStatus = null;
let audioPanelOpen = false;
let selectedAudioDeviceId = "";
let pendingAudioSegmentId = null;
const audioCapture = {
  busy: false,
  stream: null,
  recorder: null,
  chunks: [],
  session: null,
  mode: null,
  startedAt: 0,
  timer: null,
  devices: [],
  localRecordings: [],
};
''',
)

AUDIO_COMPOSER = r'''function audioSupported() {
  return Boolean(navigator.mediaDevices?.getUserMedia && window.MediaRecorder);
}

function audioActive() {
  return Boolean(audioCapture.recorder && audioCapture.recorder.state !== "inactive");
}

function formatDuration(milliseconds) {
  const total = Math.max(0, Math.floor(Number(milliseconds || 0) / 1000));
  const minutes = Math.floor(total / 60).toString().padStart(2, "0");
  const seconds = (total % 60).toString().padStart(2, "0");
  return `${minutes}:${seconds}`;
}

function audioSessionState() {
  if (audioActive()) return "recording";
  if (audioCapture.busy) return "starting";
  return audioStatus?.active_session?.state || "ready";
}

function recentAudioSegments() {
  return Array.isArray(audioStatus?.segments) ? audioStatus.segments.slice(0, 12) : [];
}

function renderAudioRecording(segment) {
  const local = audioCapture.localRecordings.find((recording) => recording.segment_id === segment.segment_id);
  return `<article class="hs-audio-recording">
    <div class="hs-audio-recording-head"><div><strong>${escapeHtml(segment.transcript || `Recording ${segment.sequence_no}`)}</strong><span>${escapeHtml(formatDuration(segment.duration_ms))} · ${escapeHtml(humanize(segment.state))}</span></div><button type="button" data-audio-delete="${escapeHtml(segment.session_id)}">Delete</button></div>
    ${local?.url ? `<audio controls preload="metadata" src="${escapeHtml(local.url)}"></audio>` : '<small>Raw audio was ephemeral and is no longer available for playback. Metadata remains local.</small>'}
    <div class="hs-audio-transcript-row"><textarea id="hs-audio-transcript-${escapeHtml(segment.segment_id)}" rows="2" maxlength="20000" placeholder="Add or correct the transcript…">${escapeHtml(segment.transcript || "")}</textarea><button type="button" data-audio-send="${escapeHtml(segment.segment_id)}">Send to Agent</button></div>
  </article>`;
}

function renderAudioPanel() {
  if (!audioPanelOpen) return "";
  const segments = recentAudioSegments();
  const state = audioSessionState();
  const deviceOptions = audioCapture.devices.length
    ? audioCapture.devices.map((device, index) => `<option value="${escapeHtml(device.deviceId)}" ${device.deviceId === selectedAudioDeviceId ? "selected" : ""}>${escapeHtml(device.label || `Microphone ${index + 1}`)}</option>`).join("")
    : '<option value="">Default microphone</option>';
  return `<section class="hs-audio-panel" aria-label="HomeServer ears and conversation controls">
    <header><div><span>HomeServer Ears</span><strong>Listening and recordings</strong><p>Audio capture runs only after local permission. Raw audio is ephemeral in Phase 23A; session metadata and transcripts stay on HomeServer.</p></div><button type="button" id="hs-audio-close">×</button></header>
    <div class="hs-audio-status"><span class="hs-audio-status-dot ${audioActive() ? "active" : ""}"></span><strong>${escapeHtml(humanize(state))}</strong><span class="hs-audio-elapsed">${audioActive() ? formatDuration(Date.now() - audioCapture.startedAt) : "00:00"}</span></div>
    <label class="hs-audio-device"><span>Microphone</span><select id="hs-audio-device">${deviceOptions}</select></label>
    <div class="hs-audio-actions">
      <button type="button" data-audio-mode="push_to_talk" ${audioActive() || audioCapture.busy ? "disabled" : ""}>Push to talk</button>
      <button type="button" data-audio-mode="live_conversation" ${audioActive() || audioCapture.busy ? "disabled" : ""}>Live conversation</button>
      <button type="button" data-audio-mode="voice_note" ${audioActive() || audioCapture.busy ? "disabled" : ""}>Voice note</button>
      <button type="button" class="danger" id="hs-audio-stop" ${audioActive() ? "" : "disabled"}>Stop</button>
    </div>
    <div class="hs-audio-privacy"><strong>Local-first recording</strong><span>No cloud speech service is used. Local VAD and Whisper transcription are the next Phase 23 milestones.</span></div>
    <div class="hs-audio-recordings"><div class="hs-audio-recordings-title"><strong>Recent recordings</strong><span>${segments.length}</span></div>${segments.length ? segments.map(renderAudioRecording).join("") : '<div class="hs-audio-empty">No Agent Chat recordings yet.</div>'}</div>
  </section>`;
}

function renderComposer() {
  return `<form class="hs-chat-composer" id="homeserver-chat-form">
    <div class="hs-chat-composer-tools">
      <select id="hs-chat-mode" aria-label="Agent mode"><option value="ask">Ask</option><option value="analyze">Analyze</option><option value="plan">Plan</option><option value="dispatch">Dispatch draft</option><option value="execute">Execution request</option></select>
      <select id="hs-chat-goal" aria-label="Goal">${goalOptions()}</select>
      <select id="hs-chat-model" aria-label="Model">${modelOptions()}</select>
      <button type="button" class="hs-chat-tool-button" id="hs-chat-audio-toggle">Ears <span>${escapeHtml(humanize(audioSessionState()))}</span></button>
      <button type="button" class="hs-chat-tool-button" id="hs-chat-connection-toggle">Connections <span>${providerConnections().length}</span></button>
    </div>
    ${renderAudioPanel()}
    <div class="hs-chat-input-shell">
      <button class="hs-chat-mic ${audioActive() ? "active" : ""}" id="hs-chat-mic" type="button" aria-label="${audioActive() ? "Stop recording" : "Start push-to-talk recording"}" title="${audioActive() ? "Stop recording" : "Push to talk"}">${audioActive() ? "■" : "●"}</button>
      <textarea id="hs-chat-input" maxlength="4000" rows="1" required placeholder="Message your HomeServer agent…" aria-label="Message HomeServer"></textarea>
      <button class="hs-chat-send" type="submit" aria-label="Send message" ${sending ? "disabled" : ""}>${sending ? "…" : "↑"}</button>
    </div>
    <div class="hs-chat-composer-footer">
      <div class="hs-chat-context-chips">
        <label><input type="checkbox" name="hs-chat-context" value="system" checked>System</label>
        <label><input type="checkbox" name="hs-chat-context" value="connections" checked>Connections</label>
        <label><input type="checkbox" name="hs-chat-context" value="knowledge" checked>Knowledge</label>
        <label><input type="checkbox" name="hs-chat-context" value="goals" checked>Goals</label>
        <label><input type="checkbox" name="hs-chat-context" value="operational_data" checked>Operational data</label>
      </div>
      <small>External actions still require a separate local approval.</small>
    </div>
  </form>`;
}
'''
replace_regex_once(
    "src/homeserver-agent-chat.js",
    r"function renderComposer\(\) \{.*?\n\}\n\nfunction renderConnectionCard",
    AUDIO_COMPOSER + "\nfunction renderConnectionCard",
)

# Bind audio controls in Agent Chat.
replace_once(
    "src/homeserver-agent-chat.js",
    '  document.querySelector("#homeserver-chat-form")?.addEventListener("submit", submitPrompt);\n',
    '''  document.querySelector("#homeserver-chat-form")?.addEventListener("submit", submitPrompt);
  document.querySelector("#hs-chat-audio-toggle")?.addEventListener("click", () => {
    audioPanelOpen = !audioPanelOpen;
    mount(true);
    if (audioPanelOpen) void refreshAudioDevices();
  });
  document.querySelector("#hs-audio-close")?.addEventListener("click", () => {
    audioPanelOpen = false;
    mount(true);
  });
  document.querySelector("#hs-chat-mic")?.addEventListener("click", () => {
    if (audioActive()) void stopAudioCapture();
    else void startAudioCapture("push_to_talk");
  });
  document.querySelectorAll("[data-audio-mode]").forEach((button) => button.addEventListener("click", () => void startAudioCapture(button.dataset.audioMode || "push_to_talk")));
  document.querySelector("#hs-audio-stop")?.addEventListener("click", () => void stopAudioCapture());
  document.querySelector("#hs-audio-device")?.addEventListener("change", (event) => { selectedAudioDeviceId = event.currentTarget.value || ""; });
  document.querySelectorAll("[data-audio-send]").forEach((button) => button.addEventListener("click", () => void sendAudioTranscript(button.dataset.audioSend || "")));
  document.querySelectorAll("[data-audio-delete]").forEach((button) => button.addEventListener("click", () => void deleteAudioSession(button.dataset.audioDelete || "")));
''',
)

# Include audio status in the normal Agent Chat refresh.
replace_once(
    "src/homeserver-agent-chat.js",
    '''    const [nextWorkspace, nextProvider] = await Promise.all([
      invoke("homeserver_agent_workspace"),
      invoke("homeserver_microgifter_status").catch(() => null),
    ]);
''',
    '''    const [nextWorkspace, nextProvider, nextAudio] = await Promise.all([
      invoke("homeserver_agent_workspace"),
      invoke("homeserver_microgifter_status").catch(() => null),
      invoke("homeserver_audio_status").catch(() => null),
    ]);
''',
)
replace_once(
    "src/homeserver-agent-chat.js",
    '''    workspace = nextWorkspace;
    if (nextProvider) provider = nextProvider;
    ensureActiveThread();
''',
    '''    workspace = nextWorkspace;
    if (nextProvider) provider = nextProvider;
    if (nextAudio) audioStatus = nextAudio;
    ensureActiveThread();
''',
)

# Link a manual or future local transcript to the Agent message it produced.
replace_once(
    "src/homeserver-agent-chat.js",
    '''    const result = await invoke("homeserver_agent_prompt", { request });
    activeThreadId = result.thread_id;
    workspace = await invoke("homeserver_agent_workspace");
''',
    '''    const result = await invoke("homeserver_agent_prompt", { request });
    activeThreadId = result.thread_id;
    if (pendingAudioSegmentId) {
      await invoke("homeserver_audio_action", { request: {
        action: "update_transcript",
        segment_id: pendingAudioSegmentId,
        transcript: prompt,
        linked_message_id: result.user_message_id,
      }}).catch(() => null);
      pendingAudioSegmentId = null;
      audioStatus = await invoke("homeserver_audio_status").catch(() => audioStatus);
    }
    workspace = await invoke("homeserver_agent_workspace");
''',
)

AUDIO_FUNCTIONS = r'''
async function refreshAudioDevices() {
  if (!audioSupported()) return;
  try {
    const devices = await navigator.mediaDevices.enumerateDevices();
    audioCapture.devices = devices.filter((device) => device.kind === "audioinput");
    if (selectedAudioDeviceId && !audioCapture.devices.some((device) => device.deviceId === selectedAudioDeviceId)) selectedAudioDeviceId = "";
    if (!selectedAudioDeviceId) selectedAudioDeviceId = audioCapture.devices[0]?.deviceId || "";
    if (audioPanelOpen) mount(true);
  } catch (error) {
    notice = { kind: "warning", message: `Unable to enumerate microphones: ${String(error)}` };
    mount(true);
  }
}

function recorderMimeType() {
  const candidates = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4"];
  return candidates.find((type) => MediaRecorder.isTypeSupported?.(type)) || "";
}

function stopAudioTracks() {
  audioCapture.stream?.getTracks().forEach((track) => track.stop());
  audioCapture.stream = null;
}

function clearAudioTimer() {
  if (audioCapture.timer) window.clearInterval(audioCapture.timer);
  audioCapture.timer = null;
}

function updateAudioTimer() {
  const elapsed = document.querySelector(".hs-audio-elapsed");
  if (elapsed && audioCapture.startedAt) elapsed.textContent = formatDuration(Date.now() - audioCapture.startedAt);
}

async function setAudioSessionState(state, detail = {}, failureCode = null) {
  if (!audioCapture.session?.session_id) return null;
  return invoke("homeserver_audio_action", { request: {
    action: "set_state",
    session_id: audioCapture.session.session_id,
    state,
    failure_code: failureCode,
    detail,
  }});
}

async function sha256Blob(blob) {
  const digest = await crypto.subtle.digest("SHA-256", await blob.arrayBuffer());
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function startAudioCapture(mode) {
  if (audioCapture.busy || audioActive()) return;
  if (!audioSupported()) {
    notice = { kind: "warning", message: "This Control Center runtime does not expose microphone recording." };
    mount(true);
    return;
  }
  audioCapture.busy = true;
  audioPanelOpen = true;
  notice = null;
  mount(true);
  try {
    const audioConstraint = selectedAudioDeviceId ? { deviceId: { exact: selectedAudioDeviceId } } : true;
    const stream = await navigator.mediaDevices.getUserMedia({ audio: audioConstraint, video: false });
    audioCapture.stream = stream;
    await refreshAudioDevices();
    const track = stream.getAudioTracks()[0];
    const settings = track?.getSettings?.() || {};
    const session = await invoke("homeserver_audio_action", { request: {
      action: "start_session",
      thread_id: activeThreadId,
      mode,
      retention_mode: "ephemeral",
      input_device_id: settings.deviceId || selectedAudioDeviceId || null,
      input_device_label: track?.label || null,
      microphone_authorized: true,
      recording_authorized: true,
    }});
    const mimeType = recorderMimeType();
    const recorder = mimeType ? new MediaRecorder(stream, { mimeType }) : new MediaRecorder(stream);
    audioCapture.session = session;
    audioCapture.mode = mode;
    audioCapture.recorder = recorder;
    audioCapture.chunks = [];
    audioCapture.startedAt = Date.now();
    recorder.addEventListener("dataavailable", (event) => { if (event.data?.size) audioCapture.chunks.push(event.data); });
    recorder.addEventListener("stop", () => void finalizeAudioCapture());
    await setAudioSessionState("listening", { mode, microphone_label: track?.label || null });
    recorder.start(500);
    audioCapture.timer = window.setInterval(updateAudioTimer, 250);
    notice = { kind: "info", message: mode === "live_conversation" ? "Live conversation is listening." : mode === "voice_note" ? "Voice note recording started." : "Push-to-talk recording started." };
  } catch (error) {
    if (audioCapture.session) await setAudioSessionState("failed", {}, "capture_start_failed").catch(() => null);
    clearAudioTimer();
    stopAudioTracks();
    audioCapture.recorder = null;
    audioCapture.session = null;
    notice = { kind: "warning", message: `Microphone capture failed: ${String(error)}` };
  } finally {
    audioCapture.busy = false;
    mount(true);
  }
}

async function stopAudioCapture() {
  if (!audioActive() || audioCapture.busy) return;
  audioCapture.busy = true;
  clearAudioTimer();
  try {
    await setAudioSessionState("finalizing_transcript", { duration_ms: Date.now() - audioCapture.startedAt });
    audioCapture.recorder.stop();
    stopAudioTracks();
    notice = { kind: "info", message: "Finalizing the local recording metadata." };
  } catch (error) {
    await setAudioSessionState("failed", {}, "capture_stop_failed").catch(() => null);
    stopAudioTracks();
    audioCapture.recorder = null;
    audioCapture.session = null;
    audioCapture.busy = false;
    notice = { kind: "warning", message: `Unable to stop recording cleanly: ${String(error)}` };
    mount(true);
  }
}

async function finalizeAudioCapture() {
  const session = audioCapture.session;
  const chunks = [...audioCapture.chunks];
  const durationMs = Math.max(0, Date.now() - audioCapture.startedAt);
  try {
    const mimeType = audioCapture.recorder?.mimeType || chunks[0]?.type || "audio/webm";
    const blob = new Blob(chunks, { type: mimeType });
    const segment = await invoke("homeserver_audio_action", { request: {
      action: "finalize_segment",
      session_id: session.session_id,
      mime_type: blob.type || "application/octet-stream",
      duration_ms: durationMs,
      byte_length: blob.size,
      content_sha256: await sha256Blob(blob),
      transcript: null,
    }});
    const url = URL.createObjectURL(blob);
    audioCapture.localRecordings.unshift({ segment_id: segment.segment_id, session_id: session.session_id, url, blob });
    await setAudioSessionState("stopped", { segment_id: segment.segment_id, raw_audio_retained: false });
    audioStatus = await invoke("homeserver_audio_status");
    notice = { kind: "success", message: "Recording complete. Add a transcript to send it through the Agent conversation engine." };
  } catch (error) {
    await setAudioSessionState("failed", {}, "segment_finalize_failed").catch(() => null);
    notice = { kind: "warning", message: `Recording finalization failed: ${String(error)}` };
  } finally {
    clearAudioTimer();
    stopAudioTracks();
    audioCapture.recorder = null;
    audioCapture.chunks = [];
    audioCapture.session = null;
    audioCapture.mode = null;
    audioCapture.startedAt = 0;
    audioCapture.busy = false;
    mount(true);
  }
}

async function sendAudioTranscript(segmentId) {
  const input = document.querySelector(`#hs-audio-transcript-${CSS.escape(segmentId)}`);
  const transcript = input?.value?.trim() || "";
  if (!transcript) {
    notice = { kind: "warning", message: "Add a transcript before sending the recording to Agent Chat." };
    mount(true);
    return;
  }
  try {
    await invoke("homeserver_audio_action", { request: {
      action: "update_transcript",
      segment_id: segmentId,
      transcript,
      linked_message_id: null,
    }});
    pendingAudioSegmentId = segmentId;
    audioPanelOpen = false;
    mount(true);
    const composer = document.querySelector("#hs-chat-input");
    if (composer) {
      composer.value = transcript;
      autoSizeComposer();
      document.querySelector("#homeserver-chat-form")?.requestSubmit();
    }
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
    mount(true);
  }
}

async function deleteAudioSession(sessionId) {
  if (!sessionId || !window.confirm("Delete this completed audio session and its transcript metadata?")) return;
  try {
    await invoke("homeserver_audio_action", { request: {
      action: "delete_session",
      session_id: sessionId,
      confirmation: "DELETE AUDIO SESSION",
    }});
    audioCapture.localRecordings = audioCapture.localRecordings.filter((recording) => {
      if (recording.session_id !== sessionId) return true;
      URL.revokeObjectURL(recording.url);
      return false;
    });
    audioStatus = await invoke("homeserver_audio_status");
    notice = { kind: "success", message: "Audio session deleted." };
  } catch (error) {
    notice = { kind: "warning", message: String(error) };
  }
  mount(true);
}
'''
append_once("src/homeserver-agent-chat.js", "async function startAudioCapture(mode)", AUDIO_FUNCTIONS)

AUDIO_CSS = r'''
/* Phase 23 Agent Chat ears and conversation engine */
.hs-chat-mic{width:42px;height:42px;flex:0 0 42px;border:1px solid #d8dee8;border-radius:50%;background:#f7f8fa;color:#111827;font-size:13px;cursor:pointer;transition:.18s ease}
.hs-chat-mic:hover{background:#eef2f7;border-color:#bcc6d4}
.hs-chat-mic.active{background:#111827;color:#fff;border-color:#111827;box-shadow:0 0 0 5px rgba(17,24,39,.1)}
.hs-audio-panel{margin:10px 0 12px;padding:16px;border:1px solid #dde3eb;border-radius:18px;background:#fbfcfd;box-shadow:0 12px 30px rgba(15,23,42,.06)}
.hs-audio-panel>header{display:flex;align-items:flex-start;justify-content:space-between;gap:18px;margin-bottom:14px}
.hs-audio-panel>header div{display:grid;gap:3px}
.hs-audio-panel>header span{font-size:11px;font-weight:800;letter-spacing:.08em;text-transform:uppercase;color:#64748b}
.hs-audio-panel>header strong{font-size:16px;color:#111827}
.hs-audio-panel>header p{margin:0;max-width:680px;font-size:12px;line-height:1.55;color:#64748b}
.hs-audio-panel>header button{border:0;background:transparent;font-size:22px;color:#64748b;cursor:pointer}
.hs-audio-status{display:flex;align-items:center;gap:9px;padding:10px 12px;border:1px solid #e2e8f0;border-radius:12px;background:#fff}
.hs-audio-status-dot{width:9px;height:9px;border-radius:50%;background:#94a3b8}
.hs-audio-status-dot.active{background:#111827;box-shadow:0 0 0 5px rgba(15,23,42,.1);animation:hsAudioPulse 1.4s ease-in-out infinite}
.hs-audio-status strong{font-size:13px;text-transform:capitalize}
.hs-audio-status span:last-child{margin-left:auto;font-variant-numeric:tabular-nums;color:#64748b}
@keyframes hsAudioPulse{50%{transform:scale(.75);opacity:.65}}
.hs-audio-device{display:grid;grid-template-columns:110px 1fr;align-items:center;gap:12px;margin-top:12px;font-size:12px;font-weight:700;color:#475569}
.hs-audio-device select{min-width:0;border:1px solid #dbe2ea;border-radius:10px;background:#fff;padding:9px 10px;color:#111827}
.hs-audio-actions{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px;margin-top:12px}
.hs-audio-actions button{border:1px solid #d7dee8;border-radius:10px;background:#fff;padding:10px;font-weight:700;color:#1f2937;cursor:pointer}
.hs-audio-actions button:hover:not(:disabled){background:#f1f5f9}
.hs-audio-actions button.danger{color:#991b1b}
.hs-audio-actions button:disabled{opacity:.45;cursor:not-allowed}
.hs-audio-privacy{display:grid;gap:2px;margin-top:12px;padding:10px 12px;border-radius:12px;background:#f1f5f9;font-size:12px}
.hs-audio-privacy span{color:#64748b;line-height:1.45}
.hs-audio-recordings{display:grid;gap:9px;margin-top:14px}
.hs-audio-recordings-title{display:flex;align-items:center;justify-content:space-between;font-size:12px;color:#475569}
.hs-audio-recording{display:grid;gap:9px;padding:12px;border:1px solid #e2e8f0;border-radius:14px;background:#fff}
.hs-audio-recording-head{display:flex;align-items:flex-start;justify-content:space-between;gap:12px}
.hs-audio-recording-head>div{display:grid;gap:2px}
.hs-audio-recording-head span,.hs-audio-recording small{font-size:11px;color:#64748b}
.hs-audio-recording-head button{border:0;background:transparent;color:#991b1b;font-size:11px;cursor:pointer}
.hs-audio-recording audio{width:100%;height:34px}
.hs-audio-transcript-row{display:grid;grid-template-columns:1fr auto;gap:8px;align-items:stretch}
.hs-audio-transcript-row textarea{resize:vertical;min-height:54px;border:1px solid #dbe2ea;border-radius:10px;padding:9px 10px;font:inherit;color:#111827}
.hs-audio-transcript-row button{border:0;border-radius:10px;background:#111827;color:#fff;padding:0 14px;font-weight:700;cursor:pointer}
.hs-audio-empty{padding:18px;border:1px dashed #cbd5e1;border-radius:12px;text-align:center;font-size:12px;color:#64748b}
@media(max-width:860px){.hs-audio-actions{grid-template-columns:1fr 1fr}.hs-audio-device{grid-template-columns:1fr}.hs-audio-transcript-row{grid-template-columns:1fr}.hs-audio-transcript-row button{min-height:40px}}
'''
append_once("src/homeserver-agent-chat.css", "Phase 23 Agent Chat ears and conversation engine", AUDIO_CSS)

print("Phase 23 Agent Chat audio integration applied.")
