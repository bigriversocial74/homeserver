from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one {label} replacement in {path}, found {count}")
    path.write_text(text.replace(old, new), encoding="utf-8")


engine = ROOT / "src/homeserver-vad-engine.js"
replace_exact(
    engine,
    """      if (this.candidateSpeechAtMs === null) this.candidateSpeechAtMs = now;
      if (now - this.candidateSpeechAtMs >= this.options.attackMs) {
""",
    """      if (this.candidateSpeechAtMs === null) this.candidateSpeechAtMs = now;
      const requiredSpeechMs = Math.max(
        this.options.attackMs,
        this.options.minSpeechMs,
      );
      if (now - this.candidateSpeechAtMs >= requiredSpeechMs) {
""",
    "minimum sustained-speech activation",
)

controller = ROOT / "src/homeserver-agent-vad.js"
replace_exact(
    controller,
    """  stopping: false,
  finalizing: false,
  stream: null,
""",
    """  stopping: false,
  finalizing: false,
  transitioning: false,
  stream: null,
""",
    "transition lock state",
)
replace_exact(
    controller,
    """    || runtime.finalizing
    || runtime.segmentStartedAt
""",
    """    || runtime.finalizing
    || runtime.transitioning
    || runtime.segmentStartedAt
""",
    "speech-start transition guard",
)
replace_exact(
    controller,
    """  runtime.currentSnapshot = snapshot;
  await setSessionState(\"user_speaking\", vadDetail(snapshot, \"speech_start\"));
  notify(\"Voice detected. Capturing this utterance locally.\", \"success\");
}
""",
    """  runtime.currentSnapshot = snapshot;
  runtime.transitioning = true;
  try {
    await setSessionState(\"user_speaking\", vadDetail(snapshot, \"speech_start\"));
    notify(\"Voice detected. Capturing this utterance locally.\", \"success\");
  } catch (error) {
    await failRuntime(
      \"vad_speech_transition_failed\",
      `Unable to enter the governed speech state: ${String(error)}`,
    );
  } finally {
    runtime.transitioning = false;
  }
}
""",
    "speech-start transition lock",
)
replace_exact(
    controller,
    """  if (
    runtime.finalizing
    || !runtime.segmentStartedAt
""",
    """  if (
    runtime.finalizing
    || runtime.transitioning
    || !runtime.segmentStartedAt
""",
    "speech-end transition guard",
)
replace_exact(
    controller,
    """    const minimumSpeechMs = runtime.engine?.options.minSpeechMs || VAD_DEFAULTS.minSpeechMs;

    if (durationMs < minimumSpeechMs || blob.size < MIN_SEGMENT_BYTES) {
""",
    """    const minimumSpeechMs = runtime.engine?.options.minSpeechMs || VAD_DEFAULTS.minSpeechMs;
    const detectedSpeechMs = Math.max(0, snapshot?.speechMs || durationMs - PRE_ROLL_MS);

    if (detectedSpeechMs < minimumSpeechMs || blob.size < MIN_SEGMENT_BYTES) {
""",
    "detected-speech short-burst boundary",
)
replace_exact(
    controller,
    """    || runtime.stopping
    || runtime.finalizing
    || !runtime.analyser
""",
    """    || runtime.stopping
    || runtime.finalizing
    || runtime.transitioning
    || !runtime.analyser
""",
    "sampling transition guard",
)
replace_exact(
    controller,
    """  runtime.finalizing = false;
  runtime.session = null;
""",
    """  runtime.finalizing = false;
  runtime.transitioning = false;
  runtime.session = null;
""",
    "transition cleanup",
)

tests = ROOT / "scripts/test-agent-audio-vad.mjs"
marker = """{
  const engine = new AdaptiveVadEngine();
  feed(engine, 0, 1_200, -60);
  const events = [];
  feed(engine, 1_230, 1_290, -24, events);
  feed(engine, 1_320, 1_650, -60, events);
  assert.equal(
    events.some((event) => event.event === \"speech_start\"),
    false,
    \"a sub-attack transient must be rejected\",
  );
}

"""
addition = marker + """{
  const engine = new AdaptiveVadEngine();
  feed(engine, 0, 1_200, -60);
  const events = [];
  feed(engine, 1_230, 1_380, -24, events);
  feed(engine, 1_410, 2_400, -65, events);
  assert.equal(
    events.some((event) => event.event === \"speech_start\"),
    false,
    \"speech shorter than the minimum sustained boundary must be rejected\",
  );
}

"""
replace_exact(tests, marker, addition, "sustained-speech regression test")

validator = ROOT / "scripts/validate-agent-audio-vad.py"
replace_exact(
    validator,
    """    (\"segment_limit\", \"maximum-segment event\"),
    (\"timestamps must be monotonic\", \"monotonic frame validation\"),
""",
    """    (\"segment_limit\", \"maximum-segment event\"),
    (\"Math.max(\", \"minimum sustained-speech activation\"),
    (\"timestamps must be monotonic\", \"monotonic frame validation\"),
""",
    "engine sustained-speech validator",
)
replace_exact(
    validator,
    """    (\"capturePanelActions\", \"live-conversation takeover\"),
    (\"vad_control_center_closed\", \"closed-window recovery\"),
""",
    """    (\"capturePanelActions\", \"live-conversation takeover\"),
    (\"runtime.transitioning\", \"serialized state transitions\"),
    (\"detectedSpeechMs\", \"detected-speech short-burst rejection\"),
    (\"vad_control_center_closed\", \"closed-window recovery\"),
""",
    "controller transition validator",
)

print("Applied Phase 23B transition serialization and sustained-speech hardening.")
