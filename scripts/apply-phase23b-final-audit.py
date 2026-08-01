from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one {label} replacement in {path}, found {count}")
    path.write_text(text.replace(old, new), encoding="utf-8")


controller = ROOT / "src/homeserver-agent-vad.js"
replace_exact(
    controller,
    """function selectedDeviceId() {
  return document.querySelector("[data-agent-audio-device]")?.value || "";
}

function recordingSupported() {
""",
    """function selectedDeviceId() {
  return document.querySelector("[data-agent-audio-device]")?.value || "";
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function recordingSupported() {
""",
    "HTML escaping helper",
)
replace_exact(
    controller,
    """    } else {
      const segment = await audioAction("audio_finalize_segment", {
        session_id: session.session_id,
        mime_type: blob.type || "application/octet-stream",
        duration_ms: Math.min(durationMs, VAD_DEFAULTS.maxSegmentMs + PRE_ROLL_MS + 2_000),
        byte_length: blob.size,
""",
    """    } else {
      const maxDurationMs = VAD_DEFAULTS.maxSegmentMs + PRE_ROLL_MS + 2_000;
      if (durationMs > maxDurationMs) {
        throw new Error("The utterance exceeded the governed VAD duration boundary.");
      }
      const segment = await audioAction("audio_finalize_segment", {
        session_id: session.session_id,
        mime_type: blob.type || "application/octet-stream",
        duration_ms: durationMs,
        byte_length: blob.size,
""",
    "exact duration evidence",
)
replace_exact(
    controller,
    """      ${runtime.notice ? `<p data-kind="${runtime.notice.kind}">${runtime.notice.message}</p>` : ""}
""",
    """      ${runtime.notice ? `<p data-kind="${escapeHtml(runtime.notice.kind)}">${escapeHtml(runtime.notice.message)}</p>` : ""}
""",
    "escaped VAD notice",
)

validator = ROOT / "scripts/validate-agent-audio-vad.py"
replace_exact(
    validator,
    """    ("detectedSpeechMs", "detected-speech short-burst rejection"),
    ("vad_control_center_closed", "closed-window recovery"),
""",
    """    ("detectedSpeechMs", "detected-speech short-burst rejection"),
    ("escapeHtml(runtime.notice.message)", "escaped VAD notice text"),
    ("duration_ms: durationMs", "exact captured-duration evidence"),
    ("durationMs > maxDurationMs", "explicit duration-boundary rejection"),
    ("vad_control_center_closed", "closed-window recovery"),
""",
    "final controller validator",
)
replace_exact(
    validator,
    """    ("MediaStreamTrackProcessor", "unbounded raw-frame pipeline"),
):
""",
    """    ("MediaStreamTrackProcessor", "unbounded raw-frame pipeline"),
    ("Math.min(durationMs", "silently clamped duration evidence"),
    ('>${runtime.notice.message}</p>', "unescaped VAD notice text"),
):
""",
    "final forbidden patterns",
)

print("Applied final Phase 23B evidence and UI safety hardening.")
