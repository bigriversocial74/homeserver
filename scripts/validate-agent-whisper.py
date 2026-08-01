from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NATIVE = (ROOT / "src-tauri/src/whisper.rs").read_text(encoding="utf-8")
CONTROLLER = (ROOT / "src/homeserver-agent-whisper.js").read_text(encoding="utf-8")
CODEC = (ROOT / "src/homeserver-whisper-codec.js").read_text(encoding="utf-8")
VAD = (ROOT / "src/homeserver-agent-vad.js").read_text(encoding="utf-8")
SERVICE = (ROOT / "crates/homeserver-service/src/audio_runtime.rs").read_text(
    encoding="utf-8"
)
LIB = (ROOT / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
INDEX = (ROOT / "index.html").read_text(encoding="utf-8")
PACKAGE = (ROOT / "package.json").read_text(encoding="utf-8")
ROOT_CARGO = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
TAURI_CARGO = (ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8")


def require(text: str, token: str, label: str) -> None:
    if token not in text:
        raise SystemExit(f"Missing {label}: {token}")


def forbid(text: str, token: str, label: str) -> None:
    if token in text:
        raise SystemExit(f"Forbidden {label}: {token}")


for token, label in (
    ('const ENGINE_ID: &str = "whisper.cpp/whisper-rs-0.16.0"', "pinned engine identity"),
    ("MAX_MODEL_BYTES", "bounded model import"),
    ("MAX_PCM_SAMPLES", "bounded PCM duration"),
    ("Zeroizing<Vec<f32>>", "ephemeral PCM zeroization"),
    ("validate_sha256", "model hash validation"),
    ("hash_file(&path, MAX_MODEL_BYTES)", "pre-inference model verification"),
    ("IMPORT WHISPER MODEL", "explicit model import confirmation"),
    ("REMOVE LOCAL WHISPER MODEL", "explicit model removal confirmation"),
    ("Only one local Whisper transcription", "single active transcription"),
    ("set_segment_callback_safe_lossy", "partial transcript callback"),
    ("set_progress_callback_safe", "bounded progress callback"),
    ("set_abort_callback_safe", "native cancellation callback"),
    ("WhisperContext::new_with_params", "embedded whisper.cpp model runtime"),
    ("whisper_state.full", "embedded local inference"),
    ("raw_audio_retained: false", "raw-audio non-retention receipt"),
    ("let transcript = tokio::task::spawn_blocking", "direct worker transcript return"),
    ("Ok(transcript)", "native final transcript return"),
    ("fs::remove_file(&temporary)", "failed-import cleanup"),
):
    require(NATIVE, token, label)

for token, label in (
    ("OfflineAudioContext", "browser-local 16 kHz resampling"),
    ("floatToPcm16", "signed PCM conversion"),
    ("WHISPER_MAX_SECONDS = 32", "browser PCM duration boundary"),
    ("decodeContext.close", "decoder context cleanup"),
):
    require(CODEC, token, label)

for token, label in (
    ('listen("homeserver-whisper-progress"', "native partial event listener"),
    ("audioBlobToWhisperPcm", "local PCM handoff"),
    ('invoke("homeserver_whisper_transcribe"', "native transcription command"),
    ('invoke("homeserver_cancel_whisper_transcription"', "native cancellation command"),
    ("const status = await refreshStatus()", "pre-progress cancellation discovery"),
    ('invoke("homeserver_import_whisper_model"', "local model import command"),
    ("audio_update_transcript", "governed final transcript persistence"),
    ("transcription_model_sha256", "model evidence persistence"),
    ("escapeHtml(state.notice.message)", "escaped transcription notices"),
    ("Final local transcript is ready", "editable final transcript UX"),
):
    require(CONTROLLER, token, label)

require(VAD, "homeserver:vad-segment-finalized", "VAD-to-Whisper local blob handoff")

for token, label in (
    ("transcription_id", "governed transcription identity"),
    ("transcription_engine", "governed engine identity"),
    ("transcription_model_sha256", "governed model identity"),
    ("local_whisper_transcription_completed", "durable transcription receipt event"),
):
    require(SERVICE, token, label)

for token, label in (
    ("mod whisper;", "native Whisper module registration"),
    ("WhisperRuntimeState::default", "native Whisper state management"),
    ("whisper::homeserver_whisper_status", "status command registration"),
    ("whisper::homeserver_whisper_transcribe", "transcription command registration"),
    ("whisper::homeserver_cancel_whisper_transcription", "cancellation command registration"),
):
    require(LIB, token, label)

require(INDEX, "homeserver-agent-whisper.js", "Phase 23C browser module")
require(ROOT_CARGO, 'whisper-rs = { version = "=0.16.0"', "pinned workspace binding")
require(TAURI_CARGO, "whisper-rs.workspace = true", "desktop Whisper dependency")

for token, label in (
    ("node --check src/homeserver-whisper-codec.js", "Whisper codec syntax gate"),
    ("node --check src/homeserver-agent-whisper.js", "Whisper controller syntax gate"),
    ("node scripts/test-agent-whisper-codec.mjs", "Whisper codec behavior gate"),
    ("validate-agent-whisper.py", "permanent Phase 23C validator gate"),
):
    require(PACKAGE, token, label)

for text, token, label in (
    (CONTROLLER, "SpeechRecognition", "browser/cloud speech recognition"),
    (CONTROLLER, "webkitSpeechRecognition", "browser/cloud speech recognition"),
    (CONTROLLER, "fetch(", "browser network egress"),
    (CONTROLLER, "XMLHttpRequest", "browser network egress"),
    (CONTROLLER, "WebSocket", "browser network egress"),
    (CONTROLLER, "localStorage", "browser persistence"),
    (CONTROLLER, "sessionStorage", "browser persistence"),
    (CONTROLLER, "indexedDB", "browser persistence"),
    (NATIVE, "reqwest", "native cloud speech or model download"),
    (NATIVE, "Command::new", "external executable invocation"),
    (NATIVE, "std::process", "external executable invocation"),
    (NATIVE, "FINAL_TRANSCRIPTS", "global transcript transfer state"),
    (NATIVE, "worker_transcript_placeholder", "placeholder transcript receipt"),
):
    forbid(text, token, label)

print(
    "Phase 23C validates embedded whisper.cpp inference, verified local model import, "
    "bounded and zeroized PCM, partial/final transcript events, cancellation, durable "
    "model/engine receipts, editable Agent Chat transcripts, and zero cloud speech egress."
)
