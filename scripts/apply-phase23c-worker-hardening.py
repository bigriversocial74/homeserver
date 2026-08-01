from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one {label} replacement in {path}, found {count}")
    path.write_text(text.replace(old, new), encoding="utf-8")


native = ROOT / "src-tauri/src/whisper.rs"
replace_exact(
    native,
    """    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
""",
    """    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
""",
    "native imports",
)
replace_exact(
    native,
    """        tokio::task::spawn_blocking(move || {
            run_whisper(
                worker_app,
                model_path,
                samples,
                worker_transcription_id,
                worker_segment_id,
                worker_model_sha256,
                worker_language,
                cancel,
            )
        })
        .await
        .map_err(|error| format!("Local Whisper worker failed: {error}"))??;
        Ok::<WhisperTranscriptionResult, String>(WhisperTranscriptionResult {
""",
    """        let transcript = tokio::task::spawn_blocking(move || {
            run_whisper(
                worker_app,
                model_path,
                samples,
                worker_transcription_id,
                worker_segment_id,
                worker_model_sha256,
                worker_language,
                cancel,
            )
        })
        .await
        .map_err(|error| format!("Local Whisper worker failed: {error}"))??;
        Ok::<WhisperTranscriptionResult, String>(WhisperTranscriptionResult {
""",
    "native worker result",
)
replace_exact(
    native,
    """            transcript: worker_transcript_placeholder(),
""",
    """            transcript,
""",
    "direct final transcript receipt",
)
replace_exact(
    native,
    """    match result {
        Ok(mut receipt) => {
            let final_event = FINAL_TRANSCRIPTS
                .lock()
                .map_err(|_| "Local Whisper transcript lock was poisoned.".to_owned())?
                .remove(&transcription_id)
                .ok_or_else(|| "Local Whisper final transcript was unavailable.".to_owned())?;
            receipt.transcript = final_event;
            Ok(receipt)
        }
        Err(error) => {
            if let Ok(mut transcripts) = FINAL_TRANSCRIPTS.lock() {
                transcripts.remove(&transcription_id);
            }
            Err(error)
        }
    }
}

static FINAL_TRANSCRIPTS: std::sync::LazyLock<Mutex<BTreeMap<String, String>>> =
    std::sync::LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn worker_transcript_placeholder() -> String {
    String::new()
}
""",
    """    result
}
""",
    "remove global transcript handoff",
)
replace_exact(
    native,
    """    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
""",
    """    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
""",
    "worker transcript return type",
)
replace_exact(
    native,
    """    FINAL_TRANSCRIPTS
        .lock()
        .map_err(|_| "Local Whisper transcript lock was poisoned.".to_owned())?
        .insert(transcription_id.clone(), transcript.clone());
    let _ = app.emit(
""",
    """    let _ = app.emit(
""",
    "remove global transcript storage",
)
replace_exact(
    native,
    """    );
    Ok(())
}
""",
    """    );
    Ok(transcript)
}
""",
    "return native transcript",
)

controller = ROOT / "src/homeserver-agent-whisper.js"
replace_exact(
    controller,
    """async function cancelActive() {
  const transcriptionId = state.active?.transcriptionId
    || state.status?.active_transcription_id;
  if (!transcriptionId) {
    notify("Local Whisper has not entered the cancellable decode stage yet.");
    return;
  }
""",
    """async function cancelActive() {
  let transcriptionId = state.active?.transcriptionId
    || state.status?.active_transcription_id;
  if (!transcriptionId && state.active) {
    const status = await refreshStatus();
    transcriptionId = status?.active_transcription_id || null;
  }
  if (!transcriptionId) {
    notify("Local Whisper has not entered the cancellable decode stage yet.");
    return;
  }
""",
    "pre-progress cancellation refresh",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    """    ("raw_audio_retained: false", "raw-audio non-retention receipt"),
    ("fs::remove_file(&temporary)", "failed-import cleanup"),
""",
    """    ("raw_audio_retained: false", "raw-audio non-retention receipt"),
    ("let transcript = tokio::task::spawn_blocking", "direct worker transcript return"),
    ("Ok(transcript)", "native final transcript return"),
    ("fs::remove_file(&temporary)", "failed-import cleanup"),
""",
    "direct transcript validator",
)
replace_exact(
    validator,
    """    ('invoke("homeserver_cancel_whisper_transcription"', "native cancellation command"),
""",
    """    ('invoke("homeserver_cancel_whisper_transcription"', "native cancellation command"),
    ("const status = await refreshStatus()", "pre-progress cancellation discovery"),
""",
    "cancellation validator",
)
replace_exact(
    validator,
    """    (NATIVE, "std::process", "external executable invocation"),
):
""",
    """    (NATIVE, "std::process", "external executable invocation"),
    (NATIVE, "FINAL_TRANSCRIPTS", "global transcript transfer state"),
    (NATIVE, "worker_transcript_placeholder", "placeholder transcript receipt"),
):
""",
    "global-state prohibition",
)

print("Applied Phase 23C direct worker result and cancellation hardening.")
