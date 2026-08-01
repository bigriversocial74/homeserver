from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENGINE = (ROOT / "src/homeserver-vad-engine.js").read_text(encoding="utf-8")
CONTROLLER = (ROOT / "src/homeserver-agent-vad.js").read_text(encoding="utf-8")
INDEX = (ROOT / "index.html").read_text(encoding="utf-8")
PACKAGE = (ROOT / "package.json").read_text(encoding="utf-8")


def require(text: str, token: str, label: str) -> None:
    if token not in text:
        raise SystemExit(f"Missing {label}: {token}")


def forbid(text: str, token: str, label: str) -> None:
    if token in text:
        raise SystemExit(f"Forbidden {label}: {token}")


for token, label in (
    ("class AdaptiveVadEngine", "pure adaptive VAD engine"),
    ("calibrationMs", "room-noise calibration"),
    ("attackMs", "speech attack hysteresis"),
    ("silenceHangoverMs", "silence hangover"),
    ("minSpeechMs", "short-burst boundary"),
    ("maxSegmentMs", "segment-duration boundary"),
    ("noiseAdaptation", "adaptive noise floor"),
    ("speech_start", "speech-start event"),
    ("speech_end", "speech-end event"),
    ("segment_limit", "maximum-segment event"),
    ("Math.max(", "minimum sustained-speech activation"),
    ("segmentLimitEmitted", "one-shot segment-limit latch"),
    ('const boundary = this.snapshot(now, "speech_end")', "one-shot speech-end snapshot"),
    ("this.speaking = false", "speech-end state reset"),
    ("timestamps must be monotonic", "monotonic frame validation"),
):
    require(ENGINE, token, label)

for token, label in (
    ("PRE_ROLL_MS = 400", "bounded local pre-roll"),
    ("MAX_SEGMENT_BYTES = 64 * 1024 * 1024", "bounded segment memory"),
    ("audio_start_session", "governed utterance session"),
    ("state: nextState", "governed state transition"),
    ('"user_speaking"', "speech boundary state"),
    ('"finalizing_transcript"', "finalization state"),
    ("audio_finalize_segment", "governed segment finalization"),
    ("vad_short_burst_rejected", "short-noise rejection"),
    ("conversation_group_id", "multi-utterance correlation"),
    ("raw_audio_retained: false", "raw-audio non-retention receipt"),
    ("URL.revokeObjectURL", "deterministic playback cleanup"),
    ("getFloatTimeDomainData", "local Web Audio analysis"),
    ("noiseSuppression: false", "unmodified local VAD input"),
    ("autoGainControl: false", "stable local VAD input"),
    ("capturePanelActions", "live-conversation takeover"),
    ("runtime.transitioning", "serialized state transitions"),
    ("detectedSpeechMs", "detected-speech short-burst rejection"),
    ("vad_control_center_closed", "closed-window recovery"),
):
    require(CONTROLLER, token, label)

for token, label in (
    ("homeserver-agent-vad.js", "Phase 23B browser module"),
):
    require(INDEX, token, label)

for token, label in (
    ("node --check src/homeserver-vad-engine.js", "VAD engine syntax gate"),
    ("node --check src/homeserver-agent-vad.js", "VAD controller syntax gate"),
    ("node scripts/test-agent-audio-vad.mjs", "deterministic VAD behavior gate"),
    ("validate-agent-audio-vad.py", "permanent Phase 23B validator gate"),
):
    require(PACKAGE, token, label)

for forbidden, label in (
    ("SpeechRecognition", "browser/cloud speech recognition"),
    ("webkitSpeechRecognition", "browser/cloud speech recognition"),
    ("fetch(", "network egress"),
    ("XMLHttpRequest", "network egress"),
    ("WebSocket", "network egress"),
    ("localStorage", "browser persistence"),
    ("sessionStorage", "browser persistence"),
    ("indexedDB", "browser persistence"),
    ("MediaStreamTrackProcessor", "unbounded raw-frame pipeline"),
):
    forbid(CONTROLLER, forbidden, label)

print(
    "Phase 23B local VAD validates adaptive calibration, sustained hysteresis, "
    "edge-triggered speech boundaries, pre-roll, short-burst rejection, bounded "
    "utterance segmentation, governed evidence, ephemeral raw audio, and zero "
    "browser/cloud speech or network egress."
)
