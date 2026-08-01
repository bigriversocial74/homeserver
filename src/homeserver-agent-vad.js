import { invoke } from "@tauri-apps/api/core";
import { AdaptiveVadEngine, VAD_DEFAULTS, rmsToDb } from "./homeserver-vad-engine.js";
import "./homeserver-agent-vad.css";

const FRAME_MS = VAD_DEFAULTS.frameMs;
const RECORDER_SLICE_MS = 100;
const PRE_ROLL_MS = 400;
const PRE_ROLL_CHUNKS = Math.ceil(PRE_ROLL_MS / RECORDER_SLICE_MS);
const MAX_SEGMENT_BYTES = 64 * 1024 * 1024;
const MIN_SEGMENT_BYTES = 128;
const MAX_LOCAL_PLAYBACKS = 12;

const runtime = {
  active: false,
  starting: false,
  stopping: false,
  finalizing: false,
  stream: null,
  audioContext: null,
  analyser: null,
  samples: null,
  sampleTimer: null,
  engine: null,
  session: null,
  recorder: null,
  discardRecorder: false,
  preRoll: [],
  preRollBytes: 0,
  segmentChunks: [],
  segmentBytes: 0,
  segmentStartedAt: 0,
  currentSnapshot: null,
  groupId: null,
  utteranceNumber: 0,
  selectedDeviceId: "",
  localPlayback: new Map(),
  notice: null,
};

let mutationObserver = null;
let uiScheduled = false;

function activeThreadId() {
  return document.querySelector(".hs-chat-thread.active")?.dataset.chatThread || null;
}

function selectedDeviceId() {
  return document.querySelector("[data-agent-audio-device]")?.value || "";
}

function recordingSupported() {
  return Boolean(
    navigator.mediaDevices?.getUserMedia
      && window.MediaRecorder
      && window.AudioContext
      && window.crypto?.subtle,
  );
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

function audioAction(action, fields = {}) {
  return invoke("homeserver_agent_integration_action", {
    request: { action, ...fields },
  });
}

function notify(message, kind = "info") {
  runtime.notice = { message: String(message), kind };
  window.dispatchEvent(
    new CustomEvent("homeserver:agent-audio-notice", {
      detail: runtime.notice,
    }),
  );
  scheduleUi();
}

function stopTracks() {
  runtime.stream?.getTracks().forEach((track) => track.stop());
  runtime.stream = null;
}

async function closeAudioContext() {
  const context = runtime.audioContext;
  runtime.audioContext = null;
  runtime.analyser = null;
  runtime.samples = null;
  if (context && context.state !== "closed") {
    await context.close().catch(() => null);
  }
}

function clearSampleTimer() {
  if (runtime.sampleTimer) window.clearInterval(runtime.sampleTimer);
  runtime.sampleTimer = null;
}

function revokePlayback(segmentId) {
  const playback = runtime.localPlayback.get(segmentId);
  if (playback?.url) URL.revokeObjectURL(playback.url);
  runtime.localPlayback.delete(segmentId);
}

function rememberPlayback(segmentId, sessionId, blob) {
  runtime.localPlayback.set(segmentId, {
    sessionId,
    blob,
    url: URL.createObjectURL(blob),
  });
  while (runtime.localPlayback.size > MAX_LOCAL_PLAYBACKS) {
    const oldest = runtime.localPlayback.keys().next().value;
    revokePlayback(oldest);
  }
}

function releaseAllPlayback() {
  [...runtime.localPlayback.keys()].forEach(revokePlayback);
}

function sha256(blob) {
  return blob.arrayBuffer().then((buffer) => crypto.subtle.digest("SHA-256", buffer))
    .then((digest) => [...new Uint8Array(digest)]
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join(""));
}

function rmsFromSamples(samples) {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / Math.max(1, samples.length));
}

function roundedMetric(value) {
  return Number.isFinite(value) ? Number(value.toFixed(2)) : null;
}

function vadDetail(snapshot, reason = null) {
  return {
    capture_host: "control_center_webview",
    segmentation: "local_adaptive_vad",
    conversation_group_id: runtime.groupId,
    utterance_number: runtime.utteranceNumber,
    boundary_reason: reason,
    algorithm_version: snapshot?.algorithmVersion || VAD_DEFAULTS.algorithmVersion,
    frame_ms: FRAME_MS,
    pre_roll_ms: PRE_ROLL_MS,
    rms_db: roundedMetric(snapshot?.rmsDb),
    noise_floor_db: roundedMetric(snapshot?.noiseFloorDb),
    start_threshold_db: roundedMetric(snapshot?.startThresholdDb),
    stop_threshold_db: roundedMetric(snapshot?.stopThresholdDb),
    speech_ms: Math.round(snapshot?.speechMs || 0),
  };
}

async function setSessionState(nextState, detail = {}, failureCode = null) {
  if (!runtime.session?.session_id) return null;
  runtime.session = await audioAction("audio_set_state", {
    session_id: runtime.session.session_id,
    state: nextState,
    failure_code: failureCode,
    detail,
  });
  return runtime.session;
}

function resetSegmentBuffers() {
  runtime.preRoll = [];
  runtime.preRollBytes = 0;
  runtime.segmentChunks = [];
  runtime.segmentBytes = 0;
  runtime.segmentStartedAt = 0;
}

function appendPreRoll(chunk) {
  runtime.preRoll.push(chunk);
  runtime.preRollBytes += chunk.size;
  while (runtime.preRoll.length > PRE_ROLL_CHUNKS) {
    runtime.preRollBytes -= runtime.preRoll.shift()?.size || 0;
  }
}

function appendSegment(chunk) {
  runtime.segmentChunks.push(chunk);
  runtime.segmentBytes += chunk.size;
  if (runtime.segmentBytes >= MAX_SEGMENT_BYTES && !runtime.finalizing) {
    void finishSpeech("segment_size_limit");
  }
}

function createRecorder() {
  const mimeType = recorderMimeType();
  const recorder = mimeType
    ? new MediaRecorder(runtime.stream, { mimeType })
    : new MediaRecorder(runtime.stream);
  runtime.recorder = recorder;
  runtime.discardRecorder = false;
  resetSegmentBuffers();

  recorder.addEventListener("dataavailable", (event) => {
    if (!event.data?.size || runtime.discardRecorder) return;
    if (runtime.segmentStartedAt) appendSegment(event.data);
    else appendPreRoll(event.data);
  });
  recorder.addEventListener("stop", () => {
    if (runtime.discardRecorder) return;
    void finalizeUtterance();
  }, { once: true });
  recorder.addEventListener("error", (event) => {
    const detail = event.error?.message || "MediaRecorder reported an error.";
    void failRuntime("vad_media_recorder_error", detail);
  }, { once: true });
  recorder.start(RECORDER_SLICE_MS);
}

async function startGovernedListeningSession() {
  if (!runtime.active || runtime.stopping) return;
  const track = runtime.stream?.getAudioTracks?.()[0];
  if (!track) throw new Error("The microphone stream is unavailable.");
  const settings = track.getSettings?.() || {};

  runtime.utteranceNumber += 1;
  runtime.session = await audioAction("audio_start_session", {
    thread_id: activeThreadId(),
    mode: "live_conversation",
    retention_mode: "transcript",
    input_device_id: settings.deviceId || runtime.selectedDeviceId || null,
    input_device_label: track.label || null,
    microphone_authorized: true,
    recording_authorized: true,
  });
  await setSessionState("listening", vadDetail(runtime.engine?.snapshot(Date.now()), "vad_listening"));
  createRecorder();
  runtime.engine.resetSpeech(performance.now());
  runtime.finalizing = false;
  await window.__HOMESERVER_AGENT_AUDIO_V1__?.refresh?.().catch(() => null);
  scheduleUi();
}

async function beginSpeech(snapshot) {
  if (
    !runtime.active
    || runtime.stopping
    || runtime.finalizing
    || runtime.segmentStartedAt
    || !runtime.session?.session_id
  ) {
    return;
  }
  runtime.segmentChunks = [...runtime.preRoll];
  runtime.segmentBytes = runtime.preRollBytes;
  runtime.preRoll = [];
  runtime.preRollBytes = 0;
  runtime.segmentStartedAt = Date.now() - PRE_ROLL_MS;
  runtime.currentSnapshot = snapshot;
  await setSessionState("user_speaking", vadDetail(snapshot, "speech_start"));
  notify("Voice detected. Capturing this utterance locally.", "success");
}

async function finishSpeech(reason) {
  if (
    runtime.finalizing
    || !runtime.segmentStartedAt
    || !runtime.session?.session_id
    || !runtime.recorder
  ) {
    return;
  }
  runtime.finalizing = true;
  runtime.currentSnapshot = runtime.engine?.snapshot(performance.now()) || runtime.currentSnapshot;
  await setSessionState(
    "finalizing_transcript",
    vadDetail(runtime.currentSnapshot, reason),
  );
  runtime.recorder.requestData?.();
  runtime.recorder.stop();
  notify("Speech boundary detected. Finalizing the local utterance.");
}

async function finalizeUtterance() {
  const session = runtime.session;
  const chunks = [...runtime.segmentChunks];
  const durationMs = Math.max(0, Date.now() - runtime.segmentStartedAt);
  const snapshot = runtime.currentSnapshot;

  try {
    if (!session?.session_id) throw new Error("The governed audio session is unavailable.");
    const mimeType = runtime.recorder?.mimeType || chunks[0]?.type || "audio/webm";
    const blob = new Blob(chunks, { type: mimeType });
    const minimumSpeechMs = runtime.engine?.options.minSpeechMs || VAD_DEFAULTS.minSpeechMs;

    if (durationMs < minimumSpeechMs || blob.size < MIN_SEGMENT_BYTES) {
      await setSessionState(
        "failed",
        vadDetail(snapshot, "short_burst_rejected"),
        "vad_short_burst_rejected",
      );
      notify("A short noise burst was rejected without creating a recording.", "info");
    } else {
      const segment = await audioAction("audio_finalize_segment", {
        session_id: session.session_id,
        mime_type: blob.type || "application/octet-stream",
        duration_ms: Math.min(durationMs, VAD_DEFAULTS.maxSegmentMs + PRE_ROLL_MS + 2_000),
        byte_length: blob.size,
        content_sha256: await sha256(blob),
        transcript: null,
      });
      rememberPlayback(segment.segment_id, session.session_id, blob);
      await setSessionState("stopped", {
        ...vadDetail(snapshot, "utterance_finalized"),
        segment_id: segment.segment_id,
        raw_audio_retained: false,
      });
      notify("Utterance segmented locally. It is ready for local transcription.", "success");
    }
  } catch (error) {
    await setSessionState(
      "failed",
      vadDetail(snapshot, "segment_finalize_failed"),
      "vad_segment_finalize_failed",
    ).catch(() => null);
    notify(`Automatic segmentation failed: ${String(error)}`, "warning");
  } finally {
    runtime.session = null;
    runtime.recorder = null;
    runtime.currentSnapshot = null;
    resetSegmentBuffers();
    runtime.finalizing = false;
    runtime.engine?.resetSpeech(performance.now());
    await window.__HOMESERVER_AGENT_AUDIO_V1__?.refresh?.().catch(() => null);
    scheduleUi();

    if (runtime.active && !runtime.stopping) {
      await startGovernedListeningSession().catch((error) => {
        void failRuntime("vad_session_restart_failed", String(error));
      });
    } else {
      await cleanupRuntime();
    }
  }
}

function sampleVad() {
  if (
    !runtime.active
    || runtime.stopping
    || runtime.finalizing
    || !runtime.analyser
    || !runtime.samples
    || !runtime.engine
  ) {
    return;
  }
  runtime.analyser.getFloatTimeDomainData(runtime.samples);
  const snapshot = runtime.engine.update(
    rmsToDb(rmsFromSamples(runtime.samples)),
    performance.now(),
  );
  runtime.currentSnapshot = snapshot;
  updateMeter(snapshot);
  if (snapshot.event === "speech_start") void beginSpeech(snapshot);
  if (snapshot.event === "speech_end") void finishSpeech("silence_hangover");
  if (snapshot.event === "segment_limit") void finishSpeech("segment_duration_limit");
}

async function startLiveConversation() {
  if (runtime.active || runtime.starting || runtime.stopping) return;
  if (!recordingSupported()) {
    notify("This Control Center runtime cannot run local voice activity detection.", "warning");
    return;
  }

  runtime.starting = true;
  runtime.notice = null;
  runtime.groupId = `vadgrp_${crypto.randomUUID().replaceAll("-", "")}`;
  runtime.utteranceNumber = 0;
  runtime.selectedDeviceId = selectedDeviceId();
  scheduleUi();

  try {
    const audioConstraint = runtime.selectedDeviceId
      ? { deviceId: { exact: runtime.selectedDeviceId } }
      : true;
    runtime.stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        ...(audioConstraint === true ? {} : audioConstraint),
        echoCancellation: true,
        noiseSuppression: false,
        autoGainControl: false,
        channelCount: 1,
      },
      video: false,
    });
    const track = runtime.stream.getAudioTracks()[0];
    if (!track) throw new Error("No microphone track was returned.");
    track.addEventListener("ended", () => {
      if (runtime.active) {
        void failRuntime(
          "vad_input_device_ended",
          "The selected microphone disconnected during local conversation monitoring.",
        );
      }
    }, { once: true });

    runtime.audioContext = new AudioContext({ latencyHint: "interactive" });
    await runtime.audioContext.resume();
    const source = runtime.audioContext.createMediaStreamSource(runtime.stream);
    runtime.analyser = runtime.audioContext.createAnalyser();
    runtime.analyser.fftSize = 1024;
    runtime.analyser.smoothingTimeConstant = 0;
    runtime.samples = new Float32Array(runtime.analyser.fftSize);
    source.connect(runtime.analyser);

    runtime.engine = new AdaptiveVadEngine();
    runtime.engine.reset(performance.now(), true);
    runtime.active = true;
    runtime.stopping = false;
    runtime.sampleTimer = window.setInterval(sampleVad, FRAME_MS);
    await startGovernedListeningSession();
    notify("Local VAD is calibrating to the room noise floor.", "success");
  } catch (error) {
    await failRuntime("vad_start_failed", String(error));
  } finally {
    runtime.starting = false;
    scheduleUi();
  }
}

async function closeSilentSession(reason) {
  if (!runtime.session?.session_id) return;
  runtime.discardRecorder = true;
  if (runtime.recorder?.state && runtime.recorder.state !== "inactive") {
    runtime.recorder.stop();
  }
  await setSessionState(
    "finalizing_transcript",
    vadDetail(runtime.engine?.snapshot(performance.now()), reason),
  );
  await setSessionState("stopped", {
    ...vadDetail(runtime.engine?.snapshot(performance.now()), reason),
    raw_audio_retained: false,
    segment_id: null,
  });
  runtime.session = null;
  runtime.recorder = null;
  resetSegmentBuffers();
}

async function stopLiveConversation(reason = "user_stop") {
  if ((!runtime.active && !runtime.starting) || runtime.stopping) return;
  runtime.stopping = true;
  runtime.active = false;
  clearSampleTimer();
  scheduleUi();

  if (runtime.segmentStartedAt && runtime.recorder?.state !== "inactive") {
    await finishSpeech(reason);
    return;
  }

  try {
    await closeSilentSession(reason);
  } catch (error) {
    await setSessionState(
      "failed",
      vadDetail(runtime.engine?.snapshot(performance.now()), "monitor_stop_failed"),
      "vad_monitor_stop_failed",
    ).catch(() => null);
    notify(`Unable to close local listening cleanly: ${String(error)}`, "warning");
  }
  await cleanupRuntime();
  await window.__HOMESERVER_AGENT_AUDIO_V1__?.refresh?.().catch(() => null);
  notify("Local voice activity monitoring stopped.", "info");
}

async function failRuntime(failureCode, message) {
  runtime.active = false;
  runtime.stopping = true;
  clearSampleTimer();
  runtime.discardRecorder = true;
  if (runtime.recorder?.state && runtime.recorder.state !== "inactive") {
    try {
      runtime.recorder.stop();
    } catch {
      // Continue with governed failure cleanup.
    }
  }
  await setSessionState(
    "failed",
    vadDetail(runtime.engine?.snapshot(performance.now()), "runtime_failed"),
    failureCode,
  ).catch(() => null);
  notify(message, "warning");
  await cleanupRuntime();
  await window.__HOMESERVER_AGENT_AUDIO_V1__?.refresh?.().catch(() => null);
}

async function cleanupRuntime() {
  clearSampleTimer();
  stopTracks();
  await closeAudioContext();
  runtime.active = false;
  runtime.starting = false;
  runtime.stopping = false;
  runtime.finalizing = false;
  runtime.session = null;
  runtime.recorder = null;
  runtime.discardRecorder = false;
  runtime.engine = null;
  runtime.groupId = null;
  runtime.currentSnapshot = null;
  resetSegmentBuffers();
  scheduleUi();
}

function meterPercent(snapshot) {
  if (!snapshot) return 0;
  const floor = Math.min(snapshot.stopThresholdDb - 15, -70);
  const ceiling = Math.max(snapshot.startThresholdDb + 18, -12);
  return Math.max(0, Math.min(100, ((snapshot.rmsDb - floor) / (ceiling - floor)) * 100));
}

function updateMeter(snapshot) {
  const meter = document.querySelector("[data-agent-vad-meter-fill]");
  const level = document.querySelector("[data-agent-vad-level]");
  const status = document.querySelector("[data-agent-vad-status]");
  if (meter) meter.style.width = `${meterPercent(snapshot).toFixed(1)}%`;
  if (level) level.textContent = `${Math.round(snapshot.rmsDb)} dB`;
  if (status) {
    status.textContent = snapshot.calibrated
      ? snapshot.speaking ? "speech" : "listening"
      : "calibrating";
  }
}

function enhancePanel() {
  const panel = document.querySelector("[data-agent-audio-panel]");
  if (!panel) return;
  const stateRow = panel.querySelector(".hs-agent-audio-state");
  if (!panel.querySelector("[data-agent-vad-ui]") && stateRow) {
    const shell = document.createElement("div");
    shell.className = "hs-agent-vad";
    shell.dataset.agentVadUi = "true";
    shell.innerHTML = `
      <div class="hs-agent-vad-head">
        <strong>Local voice activity</strong>
        <span data-agent-vad-status>${runtime.starting ? "starting" : runtime.active ? "listening" : "ready"}</span>
      </div>
      <div class="hs-agent-vad-meter" aria-hidden="true"><i data-agent-vad-meter-fill></i></div>
      <div class="hs-agent-vad-meta">
        <span data-agent-vad-level>${runtime.currentSnapshot ? `${Math.round(runtime.currentSnapshot.rmsDb)} dB` : "—"}</span>
        <span>adaptive noise floor</span>
        <span>${PRE_ROLL_MS} ms pre-roll</span>
      </div>
      ${runtime.notice ? `<p data-kind="${runtime.notice.kind}">${runtime.notice.message}</p>` : ""}
    `;
    stateRow.insertAdjacentElement("afterend", shell);
  }
  panel.querySelectorAll('[data-agent-audio-mode="live_conversation"]').forEach((button) => {
    button.textContent = runtime.active || runtime.starting ? "Local conversation active" : "Live conversation";
    button.classList.toggle("active", runtime.active || runtime.starting);
  });
  const stopButton = panel.querySelector("[data-agent-audio-stop]");
  if (stopButton && (runtime.active || runtime.starting || runtime.finalizing)) {
    stopButton.disabled = false;
  }
  hydrateLocalPlayback();
  updateMeter(runtime.currentSnapshot);
}

function hydrateLocalPlayback() {
  for (const [segmentId, playback] of runtime.localPlayback) {
    const textarea = document.querySelector(
      `[data-agent-audio-transcript="${CSS.escape(segmentId)}"]`,
    );
    const article = textarea?.closest(".hs-agent-audio-recording");
    if (!article || article.querySelector("[data-agent-vad-playback]")) continue;
    const audio = document.createElement("audio");
    audio.controls = true;
    audio.preload = "metadata";
    audio.src = playback.url;
    audio.dataset.agentVadPlayback = "true";
    const fallback = article.querySelector("small");
    if (fallback) fallback.replaceWith(audio);
    else article.querySelector(".hs-agent-audio-transcript")?.before(audio);
  }
}

function scheduleUi() {
  if (uiScheduled) return;
  uiScheduled = true;
  window.requestAnimationFrame(() => {
    uiScheduled = false;
    enhancePanel();
  });
}

function capturePanelActions(event) {
  const button = event.target.closest?.("button");
  if (!button) return;
  const mode = button.dataset.agentAudioMode;
  if (mode === "live_conversation") {
    event.preventDefault();
    event.stopImmediatePropagation();
    if (runtime.active || runtime.starting) void stopLiveConversation("live_button_stop");
    else void startLiveConversation();
    return;
  }
  if (button.hasAttribute("data-agent-audio-stop") && (
    runtime.active || runtime.starting || runtime.finalizing
  )) {
    event.preventDefault();
    event.stopImmediatePropagation();
    void stopLiveConversation();
    return;
  }
  const sessionId = button.dataset.agentAudioDelete;
  if (sessionId) {
    for (const [segmentId, playback] of runtime.localPlayback) {
      if (playback.sessionId === sessionId) revokePlayback(segmentId);
    }
  }
}

function abandonVadRuntime() {
  if (runtime.session?.session_id) {
    void audioAction("audio_set_state", {
      session_id: runtime.session.session_id,
      state: "failed",
      failure_code: "vad_control_center_closed",
      detail: vadDetail(runtime.currentSnapshot, "control_center_closed"),
    }).catch(() => null);
  }
  runtime.discardRecorder = true;
  if (runtime.recorder?.state && runtime.recorder.state !== "inactive") {
    try {
      runtime.recorder.stop();
    } catch {
      // The browser is already tearing down the recorder.
    }
  }
  clearSampleTimer();
  stopTracks();
  releaseAllPlayback();
  mutationObserver?.disconnect();
}

function initialize() {
  document.addEventListener("click", capturePanelActions, true);
  const app = document.querySelector("#app");
  if (app) {
    mutationObserver = new MutationObserver(scheduleUi);
    mutationObserver.observe(app, { childList: true, subtree: true });
  }
  window.addEventListener("homeserver:rendered", scheduleUi);
  window.addEventListener("homeserver-agent-route", scheduleUi);
  window.addEventListener("hashchange", scheduleUi);
  window.addEventListener("pagehide", abandonVadRuntime);
  window.addEventListener("beforeunload", abandonVadRuntime);
  scheduleUi();
}

initialize();

window.__HOMESERVER_AGENT_VAD_V1__ = {
  start: startLiveConversation,
  stop: stopLiveConversation,
  snapshot: () => runtime.currentSnapshot,
  active: () => runtime.active,
};
