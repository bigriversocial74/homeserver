from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one {label} replacement in {path}, found {count}")
    path.write_text(text.replace(old, new), encoding="utf-8")


controller = ROOT / "src/homeserver-agent-whisper.js"
replace_exact(
    controller,
    """import "./homeserver-agent-whisper.css";

const state = {
""",
    """import "./homeserver-agent-whisper.css";

const LOCAL_WHISPER_ENGINE = "whisper.cpp/whisper-rs-0.16.0";

const state = {
""",
    "pinned browser engine identity",
)
replace_exact(
    controller,
    """function validSha256(value) {
  return /^[a-f0-9]{64}$/i.test(String(value || "").trim());
}

function notify(message, kind = "info") {
""",
    """function validSha256(value) {
  return /^[a-f0-9]{64}$/i.test(String(value || "").trim());
}

function validateFinalReceipt(receipt, segmentId, status, language) {
  const transcript = String(receipt?.transcript || "").trim();
  if (
    receipt?.segment_id !== segmentId
    || !/^whisper_[a-f0-9]{32}$/.test(String(receipt?.transcription_id || ""))
    || receipt?.engine !== LOCAL_WHISPER_ENGINE
    || !validSha256(receipt?.model_sha256)
    || receipt.model_sha256 !== status?.model_sha256
    || receipt?.language !== language
    || receipt?.sample_rate_hz !== 16_000
    || !Number.isSafeInteger(receipt?.sample_count)
    || receipt.sample_count < 1_600
    || receipt.sample_count > 16_000 * 32
    || !Number.isSafeInteger(receipt?.duration_ms)
    || receipt.duration_ms < 100
    || receipt.duration_ms > 32_000
    || receipt?.raw_audio_retained !== false
    || !transcript
    || transcript.length > 20_000
  ) {
    throw new Error("Local Whisper returned an invalid final transcript receipt.");
  }
  return transcript;
}

function notify(message, kind = "info") {
""",
    "strict final receipt validation",
)
replace_exact(
    controller,
    """  const status = await refreshStatus();
  if (!status?.model_ready) {
    notify("The utterance is ready, but a verified local Whisper model has not been imported.", "warning");
    return;
  }

  state.busy = true;
  state.active = {
""",
    """  state.busy = true;
  scheduleRender();
  const status = await refreshStatus();
  if (!status?.model_ready) {
    state.queue.clear();
    state.busy = false;
    notify("The utterance is ready, but a verified local Whisper model has not been imported.", "warning");
    scheduleRender();
    return;
  }

  state.active = {
""",
    "pre-await controller reservation",
)
replace_exact(
    controller,
    """    if (receipt.segment_id !== detail.segment_id || !receipt.transcript?.trim()) {
      throw new Error("Local Whisper returned an invalid final transcript receipt.");
    }
    await updateGovernedTranscript(detail.segment_id, receipt.transcript.trim(), receipt);
    updateTranscriptElement(detail.segment_id, receipt.transcript.trim(), true);
""",
    """    const transcript = validateFinalReceipt(
      receipt,
      detail.segment_id,
      status,
      state.language,
    );
    await updateGovernedTranscript(detail.segment_id, transcript, receipt);
    updateTranscriptElement(detail.segment_id, transcript, true);
""",
    "validated native receipt persistence",
)

service = ROOT / "crates/homeserver-service/src/audio_runtime.rs"
replace_exact(
    service,
    """const LOCAL_ACTOR_ID: &str = "local_control_center";
const MAX_BODY_BYTES: usize = 256 * 1024;
""",
    """const LOCAL_ACTOR_ID: &str = "local_control_center";
const LOCAL_WHISPER_ENGINE: &str = "whisper.cpp/whisper-rs-0.16.0";
const LOCAL_WHISPER_ID_PREFIX: &str = "whisper_";
const MAX_BODY_BYTES: usize = 256 * 1024;
""",
    "service Whisper identity constants",
)
replace_exact(
    service,
    """    if has_transcription_receipt {
        ensure!(
            transcription_id.is_some()
                && transcription_engine.is_some()
                && transcription_model_sha256.is_some()
                && transcription_language.is_some()
                && request.transcription_final,
            "complete final local transcription metadata is required"
        );
        ensure!(
            !request.raw_audio_retained,
            "local transcription cannot retain raw audio"
        );
""",
    """    if has_transcription_receipt {
        ensure!(
            transcription_id.is_some()
                && transcription_engine.is_some()
                && transcription_model_sha256.is_some()
                && transcription_language.is_some()
                && request.transcription_final,
            "complete final local transcription metadata is required"
        );
        ensure!(
            transcription_id
                .as_deref()
                .is_some_and(valid_local_whisper_transcription_id),
            "local transcription ID is invalid"
        );
        ensure!(
            transcription_engine.as_deref() == Some(LOCAL_WHISPER_ENGINE),
            "local transcription engine is unsupported"
        );
        ensure!(
            transcription_language
                .as_deref()
                .is_some_and(valid_local_whisper_language),
            "local transcription language is invalid"
        );
        ensure!(
            !request.raw_audio_retained,
            "local transcription cannot retain raw audio"
        );
""",
    "service receipt boundary",
)
replace_exact(
    service,
    """fn normalized_sha256(value: &str) -> Result<String> {
""",
    """fn valid_local_whisper_transcription_id(value: &str) -> bool {
    value.len() == LOCAL_WHISPER_ID_PREFIX.len() + 32
        && value.starts_with(LOCAL_WHISPER_ID_PREFIX)
        && value[LOCAL_WHISPER_ID_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn valid_local_whisper_language(value: &str) -> bool {
    value == "auto"
        || ((2..=8).contains(&value.len())
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'-'))
}

fn normalized_sha256(value: &str) -> Result<String> {
""",
    "service receipt validators",
)
replace_exact(
    service,
    """    #[test]
    fn mime_and_hash_validation_are_closed() {
""",
    """    #[test]
    fn local_whisper_receipt_boundaries_are_closed() {
        assert!(valid_local_whisper_transcription_id(
            "whisper_0123456789abcdef0123456789abcdef"
        ));
        assert!(!valid_local_whisper_transcription_id("whisper_short"));
        assert!(!valid_local_whisper_transcription_id(
            "other_0123456789abcdef0123456789abcdef"
        ));
        assert!(valid_local_whisper_language("en"));
        assert!(valid_local_whisper_language("auto"));
        assert!(valid_local_whisper_language("pt-br"));
        assert!(!valid_local_whisper_language("EN"));
        assert!(!valid_local_whisper_language("en<script>"));
    }

    #[test]
    fn mime_and_hash_validation_are_closed() {
""",
    "service receipt tests",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    """    ("Final local transcript is ready", "editable final transcript UX"),
""",
    """    ("Final local transcript is ready", "editable final transcript UX"),
    ("LOCAL_WHISPER_ENGINE", "pinned browser receipt engine"),
    ("validateFinalReceipt", "strict native receipt validation"),
    ("state.busy = true;", "pre-await controller reservation"),
    ("receipt?.raw_audio_retained !== false", "raw-audio receipt validation"),
""",
    "final controller validator requirements",
)
replace_exact(
    validator,
    """    ("local_whisper_transcription_completed", "durable transcription receipt event"),
):
""",
    """    ("local_whisper_transcription_completed", "durable transcription receipt event"),
    ("LOCAL_WHISPER_ENGINE", "pinned service receipt engine"),
    ("valid_local_whisper_transcription_id", "service transcription ID boundary"),
    ("valid_local_whisper_language", "service language boundary"),
    ("local_whisper_receipt_boundaries_are_closed", "service receipt tests"),
):
""",
    "service receipt validator requirements",
)
replace_exact(
    validator,
    """    ROOT / ".github/workflows/phase23c-queue-hardening.yml",
):
""",
    """    ROOT / ".github/workflows/phase23c-queue-hardening.yml",
    ROOT / "scripts/apply-phase23c-final-contract-hardening.py",
    ROOT / ".github/workflows/phase23c-final-contract-hardening.yml",
):
""",
    "final hardening cleanup denylist",
)

print("Applied Phase 23C final controller and durable receipt contract hardening.")
