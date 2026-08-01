from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one {label} replacement in {path}, found {count}")
    path.write_text(text.replace(old, new), encoding="utf-8")


lib = ROOT / "src-tauri/src/lib.rs"
replace_exact(
    lib,
    """mod vault;
mod vp3_authority;
""",
    """mod vault;
mod vp3_authority;
mod whisper;
""",
    "Whisper module registration",
)
replace_exact(
    lib,
    """        .setup(|app| {
            #[cfg(desktop)]
""",
    """        .setup(|app| {
            app.manage(whisper::WhisperRuntimeState::default());
            #[cfg(desktop)]
""",
    "Whisper runtime state management",
)
replace_exact(
    lib,
    """            agent::homeserver_open_agent_authorization,
            operational::homeserver_operational_data,
""",
    """            agent::homeserver_open_agent_authorization,
            whisper::homeserver_whisper_status,
            whisper::homeserver_import_whisper_model,
            whisper::homeserver_remove_whisper_model,
            whisper::homeserver_whisper_transcribe,
            whisper::homeserver_cancel_whisper_transcription,
            operational::homeserver_operational_data,
""",
    "Whisper command registration",
)

vad = ROOT / "src/homeserver-agent-vad.js"
replace_exact(
    vad,
    """      rememberPlayback(segment.segment_id, session.session_id, blob);
      await setSessionState("stopped", {
""",
    """      rememberPlayback(segment.segment_id, session.session_id, blob);
      window.dispatchEvent(new CustomEvent("homeserver:vad-segment-finalized", {
        detail: {
          segment_id: segment.segment_id,
          session_id: session.session_id,
          blob,
          duration_ms: durationMs,
          content_sha256: segment.content_sha256,
        },
      }));
      await setSessionState("stopped", {
""",
    "local VAD-to-Whisper blob handoff",
)

service = ROOT / "crates/homeserver-service/src/audio_runtime.rs"
replace_exact(
    service,
    """pub struct UpdateAudioTranscriptRequest {
    pub segment_id: String,
    pub transcript: String,
    pub linked_message_id: Option<String>,
}
""",
    """pub struct UpdateAudioTranscriptRequest {
    pub segment_id: String,
    pub transcript: String,
    pub linked_message_id: Option<String>,
    #[serde(default)]
    pub transcription_id: Option<String>,
    #[serde(default)]
    pub transcription_engine: Option<String>,
    #[serde(default)]
    pub transcription_model_sha256: Option<String>,
    #[serde(default)]
    pub transcription_language: Option<String>,
    #[serde(default)]
    pub transcription_final: bool,
    #[serde(default)]
    pub raw_audio_retained: bool,
}
""",
    "transcription receipt request fields",
)
replace_exact(
    service,
    """    let linked_message_id =
        normalized_optional_identifier(request.linked_message_id, "linked message ID")?;

    let now = now_utc();
""",
    """    let linked_message_id =
        normalized_optional_identifier(request.linked_message_id, "linked message ID")?;
    let transcription_id = normalized_optional_identifier(
        request.transcription_id,
        "transcription ID",
    )?;
    let transcription_engine = normalized_optional_value(
        request.transcription_engine,
        160,
        "transcription engine",
    )?;
    let transcription_model_sha256 = request
        .transcription_model_sha256
        .map(|value| normalized_sha256(&value))
        .transpose()?;
    let transcription_language = normalized_optional_value(
        request.transcription_language,
        32,
        "transcription language",
    )?;
    let has_transcription_receipt = transcription_id.is_some()
        || transcription_engine.is_some()
        || transcription_model_sha256.is_some()
        || transcription_language.is_some();
    if has_transcription_receipt {
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
        ensure!(
            linked_message_id.is_none(),
            "local transcription receipt must precede Agent message linkage"
        );
    }

    let now = now_utc();
""",
    "local transcription receipt validation",
)
replace_exact(
    service,
    """        if linked_message_id.is_some() {
            "transcript_committed"
        } else {
            "transcript_updated"
        },
        json!({
            "linked_message_id": linked_message_id,
            "thread_id": resolved_thread_id
        }),
""",
    """        if linked_message_id.is_some() {
            "transcript_committed"
        } else if has_transcription_receipt {
            "local_whisper_transcription_completed"
        } else {
            "transcript_updated"
        },
        json!({
            "linked_message_id": linked_message_id,
            "thread_id": resolved_thread_id,
            "transcription_id": transcription_id,
            "transcription_engine": transcription_engine,
            "transcription_model_sha256": transcription_model_sha256,
            "transcription_language": transcription_language,
            "transcription_final": has_transcription_receipt.then_some(true),
            "raw_audio_retained": has_transcription_receipt.then_some(false)
        }),
""",
    "durable local transcription receipt event",
)

index = ROOT / "index.html"
replace_exact(
    index,
    """    <script type="module" src="/src/homeserver-agent-vad.js"></script>
    <script type="module" src="/src/shared-sidebar.js"></script>
""",
    """    <script type="module" src="/src/homeserver-agent-vad.js"></script>
    <script type="module" src="/src/homeserver-agent-whisper.js"></script>
    <script type="module" src="/src/shared-sidebar.js"></script>
""",
    "Whisper browser module load",
)

package = ROOT / "package.json"
text = package.read_text(encoding="utf-8")
text = text.replace(
    "node --check src/homeserver-agent-vad.js && node scripts/test-agent-audio-vad.mjs &&",
    "node --check src/homeserver-agent-vad.js && node scripts/test-agent-audio-vad.mjs && node --check src/homeserver-whisper-codec.js && node --check src/homeserver-agent-whisper.js && node scripts/test-agent-whisper-codec.mjs &&",
)
text = text.replace(
    "validate-agent-audio-vad.py validate-notification-menu.py",
    "validate-agent-audio-vad.py validate-agent-whisper.py validate-notification-menu.py",
)
package.write_text(text, encoding="utf-8")

print("Applied Phase 23C native, service, VAD, and Agent Chat integration.")
