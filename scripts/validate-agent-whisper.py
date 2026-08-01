from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NATIVE = (ROOT / "src-tauri/src/whisper.rs").read_text(encoding="utf-8")
CONTROLLER = (ROOT / "src/homeserver-agent-whisper.js").read_text(encoding="utf-8")
CODEC = (ROOT / "src/homeserver-whisper-codec.js").read_text(encoding="utf-8")
QUEUE = (ROOT / "src/homeserver-whisper-queue.js").read_text(encoding="utf-8")
VAD = (ROOT / "src/homeserver-agent-vad.js").read_text(encoding="utf-8")
SERVICE = (ROOT / "crates/homeserver-service/src/audio_runtime.rs").read_text(
    encoding="utf-8"
)
LIB = (ROOT / "src-tauri/src/lib.rs").read_text(encoding="utf-8")
INDEX = (ROOT / "index.html").read_text(encoding="utf-8")
PACKAGE = (ROOT / "package.json").read_text(encoding="utf-8")
ROOT_CARGO = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
TAURI_CARGO = (ROOT / "src-tauri/Cargo.toml").read_text(encoding="utf-8")
WORKFLOW = (ROOT / ".github/workflows/phase23c-local-whisper.yml").read_text(
    encoding="utf-8"
)


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
    ("data: SegmentCallbackData", "explicit whisper-rs callback type"),
    ("set_progress_callback_safe", "bounded progress callback"),
    ("progress: i32", "explicit whisper-rs progress callback type"),
    ("set_abort_callback_safe", "native cancellation callback"),
    ("WhisperContext::new_with_params", "embedded whisper.cpp model runtime"),
    ("whisper_state.full", "embedded local inference"),
    ("raw_audio_retained: false", "raw-audio non-retention receipt"),
    ("let transcript = tokio::task::spawn_blocking", "direct worker transcript return"),
    ("Ok(transcript)", "native final transcript return"),
    ("fs::remove_file(&temporary)", "failed-import cleanup"),
    ("install_temporary_file", "atomic file replacement"),
    ("rollback_temporary_file", "replacement rollback"),
    ("commit_temporary_file", "post-commit backup cleanup"),
    ("model_backup.as_deref()", "model replacement receipt"),
    ("partial_transcript: transcript.clone()", "non-consuming final transcript event"),
    ("atomic_replacement_commits_and_rolls_back", "native rollback test"),
    ("model_operation: Arc<AsyncMutex<()>>", "shared model operation gate"),
    ("state.model_operation.clone().lock_owned().await", "serialized model operations"),
    ("fs::set_permissions(&temporary", "pre-install model permissions"),
    ("let previous = match read_manifest(&app).await", "pre-install manifest validation"),
    ("model_operation_lock_serializes_mutation_and_inference", "native operation lock test"),
    ("cleanup_staged_speech_directories", "interrupted removal recovery"),
    ("stage_speech_directory_removal", "atomic whole-directory removal"),
    ("rollback_staged_speech_directory", "removal rollback"),
    ("commit_staged_speech_directory", "removal commit"),
    ("speech_directory_removal_commits_and_rolls_back", "native removal transaction test"),
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
    ("LOCAL_WHISPER_ENGINE", "pinned browser receipt engine"),
    ("validateFinalReceipt", "strict native receipt validation"),
    ("state.busy = true;", "pre-await controller reservation"),
    ("receipt?.raw_audio_retained !== false", "raw-audio receipt validation"),
    ("WhisperSegmentQueue", "bounded utterance queue integration"),
    ("queueSegment(detail)", "non-dropping active transcription handoff"),
    ("drainQueuedSegment()", "FIFO queue drain"),
    ("state.queue.clear()", "ephemeral queue cleanup"),
):
    require(CONTROLLER, token, label)

require(VAD, "homeserver:vad-segment-finalized", "VAD-to-Whisper local blob handoff")
for token, label in (
    ("DEFAULT_MAX_SEGMENTS = 6", "bounded queue segment count"),
    ("DEFAULT_MAX_BYTES = 64 * 1024 * 1024", "bounded queue byte count"),
    ("this.segmentIds", "duplicate segment rejection"),
    ("this.bytes + blob.size > this.maxBytes", "aggregate byte boundary"),
    ("this.items.shift()", "FIFO queue order"),
):
    require(QUEUE, token, label)

for token, label in (
    ("transcription_id", "governed transcription identity"),
    ("transcription_engine", "governed engine identity"),
    ("transcription_model_sha256", "governed model identity"),
    ("local_whisper_transcription_completed", "durable transcription receipt event"),
    ("LOCAL_WHISPER_ENGINE", "pinned service receipt engine"),
    ("valid_local_whisper_transcription_id", "service transcription ID boundary"),
    ("valid_local_whisper_language", "service language boundary"),
    ("local_whisper_receipt_boundaries_are_closed", "service receipt tests"),
    ("matches!(byte, b'a'..=b'f')", "lowercase transcription ID boundary"),
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
    ("node --check src/homeserver-whisper-queue.js", "Whisper queue syntax gate"),
    ("node --check src/homeserver-agent-whisper.js", "Whisper controller syntax gate"),
    ("node scripts/test-agent-whisper-codec.mjs", "Whisper codec behavior gate"),
    ("node scripts/test-agent-whisper-queue.mjs", "Whisper queue behavior gate"),
    ("validate-agent-whisper.py", "permanent Phase 23C validator gate"),
):
    require(PACKAGE, token, label)

for token, label in (
    ("ubuntu-24.04", "Linux certification job"),
    ("windows-2025", "Windows certification job"),
    ("build-essential", "Linux C++ build tools"),
    ("libclang-dev", "Linux whisper.cpp binding prerequisites"),
    ("npm run check:frontend", "retained frontend validation"),
    ("npm run build", "production frontend build"),
    ("npm run prepare:icons", "generated Tauri desktop resources"),
    ("cargo build", "packaged HomeServer binary build"),
    ("-p microgifter-homeserver-service", "service package staging"),
    ("-p microgifter-homeserver-updater", "updater package staging"),
    ("-p microgifter-homeserver-mcp", "MCP package staging"),
    ("src-tauri/resources/microgifter-homeserver-service.exe", "service bundle resource"),
    ("src-tauri/resources/microgifter-homeserver-updater.exe", "updater bundle resource"),
    ("src-tauri/resources/microgifter-homeserver-mcp.exe", "MCP bundle resource"),
    ("cargo fmt --all -- --check", "Rust formatting gate"),
    (
        "cargo test -p microgifter-homeserver-control-center whisper::tests",
        "native Whisper boundary tests",
    ),
    (
        "cargo test -p microgifter-homeserver-service audio_runtime::tests",
        "governed transcript receipt tests",
    ),
    (
        "cargo clippy -p microgifter-homeserver-control-center --all-targets -- -D warnings",
        "strict Control Center lint",
    ),
    (
        "cargo clippy -p microgifter-homeserver-service --all-targets -- -D warnings",
        "strict service lint",
    ),
):
    require(WORKFLOW, token, label)

for text, token, label in (
    (CONTROLLER, "SpeechRecognition", "browser/cloud speech recognition"),
    (CONTROLLER, "webkitSpeechRecognition", "browser/cloud speech recognition"),
    (CONTROLLER, "fetch(", "browser network egress"),
    (CONTROLLER, "XMLHttpRequest", "browser network egress"),
    (CONTROLLER, "WebSocket", "browser network egress"),
    (CONTROLLER, "localStorage", "browser persistence"),
    (CONTROLLER, "sessionStorage", "browser persistence"),
    (CONTROLLER, "indexedDB", "browser persistence"),
    (CONTROLLER, "|| state.active) return;", "silent active-transcription segment drop"),
    (NATIVE, "reqwest", "native cloud speech or model download"),
    (NATIVE, "Command::new", "external executable invocation"),
    (NATIVE, "std::process", "external executable invocation"),
    (NATIVE, "FINAL_TRANSCRIPTS", "global transcript transfer state"),
    (NATIVE, "worker_transcript_placeholder", "placeholder transcript receipt"),
    (NATIVE, "fs::remove_file(&destination)", "delete-before-replace model update"),
    (NATIVE, "fs::remove_file(&path)", "delete-before-replace manifest update"),
    (NATIVE, "fs::set_permissions(&destination", "post-install permission mutation"),
):
    forbid(text, token, label)

for temporary_path in (
    ROOT / "scripts/apply-phase23c-integration.py",
    ROOT / ".github/workflows/phase23c-integration.yml",
    ROOT / ".github/workflows/phase23c-format.yml",
    ROOT / "scripts/apply-phase23c-worker-hardening.py",
    ROOT / ".github/workflows/phase23c-worker-hardening.yml",
    ROOT / ".github/workflows/phase23c-native-diagnostics.yml",
    ROOT / "phase23c-native-errors.txt",
    ROOT / "phase23c-native-exit-code.txt",
    ROOT / ".github/workflows/phase23c-whisper-errors.yml",
    ROOT / "phase23c-whisper-errors.txt",
    ROOT / "phase23c-whisper-exit.txt",
    ROOT / "scripts/apply-phase23c-atomic-hardening.py",
    ROOT / ".github/workflows/phase23c-atomic-hardening.yml",
    ROOT / "scripts/apply-phase23c-operation-hardening.py",
    ROOT / ".github/workflows/phase23c-operation-hardening.yml",
    ROOT / "scripts/apply-phase23c-queue-hardening.py",
    ROOT / ".github/workflows/phase23c-queue-hardening.yml",
    ROOT / "scripts/apply-phase23c-final-contract-hardening.py",
    ROOT / ".github/workflows/phase23c-final-contract-hardening.yml",
    ROOT / "scripts/apply-phase23c-removal-hardening.py",
    ROOT / ".github/workflows/phase23c-removal-hardening.yml",
    ROOT / "scripts/apply-phase23c-callback-type-repair.py",
    ROOT / ".github/workflows/phase23c-callback-type-repair.yml",
    ROOT / "scripts/apply-phase23c-progress-type-repair.py",
    ROOT / ".github/workflows/phase23c-progress-type-repair.yml",
):
    if temporary_path.exists():
        raise SystemExit(
            "Temporary Phase 23C certification asset remains: "
            f"{temporary_path.relative_to(ROOT)}"
        )

print(
    "Phase 23C validates embedded whisper.cpp inference, verified local model import, "
    "bounded and zeroized PCM, partial/final transcript events, cancellation, durable "
    "model/engine receipts, editable Agent Chat transcripts, generated desktop resources, "
    "real packaged binary staging, cross-platform native certification, permanent cleanup "
    "hygiene, and zero cloud speech egress."
)
