use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{SecondsFormat, Utc};
use rfd::AsyncFileDialog;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex as AsyncMutex,
};
use uuid::Uuid;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
use zeroize::Zeroizing;

const MANIFEST_SCHEMA: &str = "homeserver.local-whisper-model.v1";
const ENGINE_ID: &str = "whisper.cpp/whisper-rs-0.16.0";
const MANIFEST_FILE: &str = "model-manifest.json";
const MIN_MODEL_BYTES: u64 = 1 * 1024 * 1024;
const MAX_MODEL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const SAMPLE_RATE_HZ: u32 = 16_000;
const MIN_PCM_SAMPLES: usize = 1_600;
const MAX_PCM_SAMPLES: usize = 16_000 * 32;
const MAX_TRANSCRIPT_CHARS: usize = 20_000;
const MAX_IDENTIFIER_CHARS: usize = 160;
const COPY_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct WhisperRuntimeState {
    active: AsyncMutex<Option<ActiveTranscription>>,
}

struct ActiveTranscription {
    transcription_id: String,
    cancel: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhisperModelManifest {
    schema: String,
    engine: String,
    model_file: String,
    model_sha256: String,
    byte_length: u64,
    imported_at_utc: String,
    last_verified_at_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WhisperStatus {
    schema: &'static str,
    engine: &'static str,
    model_ready: bool,
    model_sha256: Option<String>,
    model_byte_length: Option<u64>,
    imported_at_utc: Option<String>,
    last_verified_at_utc: Option<String>,
    verification_state: String,
    active_transcription_id: Option<String>,
    capabilities: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WhisperTranscriptionRequest {
    segment_id: String,
    pcm16_base64: String,
    sample_rate_hz: u32,
    channels: u8,
    language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WhisperTranscriptionResult {
    schema: &'static str,
    transcription_id: String,
    segment_id: String,
    engine: &'static str,
    model_sha256: String,
    language: String,
    sample_rate_hz: u32,
    sample_count: usize,
    duration_ms: u64,
    transcript: String,
    completed_at_utc: String,
    raw_audio_retained: bool,
}

#[derive(Debug, Clone, Serialize)]
struct WhisperProgressEvent {
    schema: &'static str,
    transcription_id: String,
    segment_id: String,
    kind: &'static str,
    progress: i32,
    partial_transcript: String,
    model_sha256: String,
}

fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn validate_sha256(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "Whisper model SHA-256 must contain exactly 64 hexadecimal characters.".to_owned(),
        );
    }
    Ok(normalized)
}

fn validate_identifier(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty()
        || normalized.chars().count() > MAX_IDENTIFIER_CHARS
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(format!("{label} is invalid."));
    }
    Ok(normalized.to_owned())
}

fn validate_language(value: Option<String>) -> Result<String, String> {
    let normalized = value
        .unwrap_or_else(|| "en".to_owned())
        .trim()
        .to_ascii_lowercase();
    if normalized == "auto" {
        return Ok(normalized);
    }
    if !(2..=8).contains(&normalized.len())
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
    {
        return Err("Whisper language must be 'auto' or a lowercase language code.".to_owned());
    }
    Ok(normalized)
}

fn normalize_transcript(value: &str) -> Result<String, String> {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Err("Local Whisper returned an empty transcript.".to_owned());
    }
    if normalized.chars().count() > MAX_TRANSCRIPT_CHARS {
        return Err("Local Whisper transcript exceeds the governed text boundary.".to_owned());
    }
    Ok(normalized)
}

fn decode_pcm16(request: &WhisperTranscriptionRequest) -> Result<Zeroizing<Vec<f32>>, String> {
    if request.sample_rate_hz != SAMPLE_RATE_HZ || request.channels != 1 {
        return Err("Local Whisper accepts only 16 kHz mono PCM.".to_owned());
    }
    if request.pcm16_base64.len() > MAX_PCM_SAMPLES.saturating_mul(4) {
        return Err("Local Whisper PCM payload exceeds the encoded size boundary.".to_owned());
    }
    let bytes = Zeroizing::new(
        STANDARD
            .decode(request.pcm16_base64.as_bytes())
            .map_err(|_| "Local Whisper PCM payload is not valid base64.".to_owned())?,
    );
    if bytes.len() % 2 != 0 {
        return Err("Local Whisper PCM payload has an incomplete sample.".to_owned());
    }
    let sample_count = bytes.len() / 2;
    if !(MIN_PCM_SAMPLES..=MAX_PCM_SAMPLES).contains(&sample_count) {
        return Err(
            "Local Whisper PCM sample count is outside the governed duration boundary.".to_owned(),
        );
    }
    let mut samples = Zeroizing::new(Vec::with_capacity(sample_count));
    for chunk in bytes.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(sample as f32 / i16::MAX as f32);
    }
    Ok(samples)
}

fn speech_directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("speech"))
        .map_err(|error| error.to_string())
}

fn manifest_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(speech_directory(app)?.join(MANIFEST_FILE))
}

async fn read_manifest(app: &AppHandle) -> Result<Option<WhisperModelManifest>, String> {
    let path = manifest_path(app)?;
    let bytes = match fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if bytes.len() > 64 * 1024 {
        return Err("Whisper model manifest exceeds the local size boundary.".to_owned());
    }
    let manifest: WhisperModelManifest =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if manifest.schema != MANIFEST_SCHEMA || manifest.engine != ENGINE_ID {
        return Err("Whisper model manifest schema is unsupported.".to_owned());
    }
    validate_sha256(&manifest.model_sha256)?;
    if Path::new(&manifest.model_file).components().count() != 1 {
        return Err("Whisper model manifest contains an unsafe file name.".to_owned());
    }
    Ok(Some(manifest))
}

async fn install_temporary_file(
    temporary: &Path,
    destination: &Path,
) -> Result<Option<PathBuf>, String> {
    let backup = if fs::try_exists(destination)
        .await
        .map_err(|error| error.to_string())?
    {
        let file_name = destination
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| "Atomic replacement destination has an invalid file name.".to_owned())?;
        let backup =
            destination.with_file_name(format!(".{file_name}.{}.backup", Uuid::new_v4().simple()));
        fs::rename(destination, &backup)
            .await
            .map_err(|error| error.to_string())?;
        Some(backup)
    } else {
        None
    };

    if let Err(error) = fs::rename(temporary, destination).await {
        if let Some(backup_path) = backup.as_ref() {
            let _ = fs::rename(backup_path, destination).await;
        }
        return Err(error.to_string());
    }
    Ok(backup)
}

async fn rollback_temporary_file(destination: &Path, backup: Option<&Path>) -> Result<(), String> {
    if fs::try_exists(destination)
        .await
        .map_err(|error| error.to_string())?
    {
        fs::remove_file(destination)
            .await
            .map_err(|error| error.to_string())?;
    }
    if let Some(backup_path) = backup {
        fs::rename(backup_path, destination)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn commit_temporary_file(backup: Option<&Path>) {
    if let Some(backup_path) = backup {
        let _ = fs::remove_file(backup_path).await;
    }
}

async fn write_manifest(app: &AppHandle, manifest: &WhisperModelManifest) -> Result<(), String> {
    let directory = speech_directory(app)?;
    fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let path = directory.join(MANIFEST_FILE);
    let temporary = directory.join(format!(".{MANIFEST_FILE}.{}.tmp", Uuid::new_v4().simple()));
    let bytes = serde_json::to_vec_pretty(manifest).map_err(|error| error.to_string())?;
    let mut output = fs::File::create(&temporary)
        .await
        .map_err(|error| error.to_string())?;
    if let Err(error) = output.write_all(&bytes).await {
        drop(output);
        let _ = fs::remove_file(&temporary).await;
        return Err(error.to_string());
    }
    if let Err(error) = output.sync_all().await {
        drop(output);
        let _ = fs::remove_file(&temporary).await;
        return Err(error.to_string());
    }
    drop(output);
    let backup = match install_temporary_file(&temporary, &path).await {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_file(&temporary).await;
            return Err(error);
        }
    };
    commit_temporary_file(backup.as_deref()).await;
    Ok(())
}

async fn hash_file(path: &Path, maximum_bytes: u64) -> Result<(String, u64), String> {
    let mut input = fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    loop {
        let read = input
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| "Whisper model size overflow.".to_owned())?;
        if total > maximum_bytes {
            return Err("Whisper model exceeds the local size boundary.".to_owned());
        }
        hasher.update(&buffer[..read]);
    }
    buffer.fill(0);
    Ok((hex::encode(hasher.finalize()), total))
}

async fn verified_model(app: &AppHandle) -> Result<(PathBuf, WhisperModelManifest), String> {
    let mut manifest = read_manifest(app)
        .await?
        .ok_or_else(|| "No local Whisper model has been imported.".to_owned())?;
    let path = speech_directory(app)?.join(&manifest.model_file);
    let (sha256, byte_length) = hash_file(&path, MAX_MODEL_BYTES).await?;
    if sha256 != manifest.model_sha256 || byte_length != manifest.byte_length {
        return Err("Local Whisper model integrity verification failed.".to_owned());
    }
    manifest.last_verified_at_utc = now_utc();
    write_manifest(app, &manifest).await?;
    Ok((path, manifest))
}

async fn active_id(state: &WhisperRuntimeState) -> Option<String> {
    state
        .active
        .lock()
        .await
        .as_ref()
        .map(|active| active.transcription_id.clone())
}

#[tauri::command]
pub(crate) async fn homeserver_whisper_status(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
) -> Result<WhisperStatus, String> {
    let manifest = read_manifest(&app).await?;
    let (model_ready, verification_state) = if let Some(manifest) = manifest.as_ref() {
        let path = speech_directory(&app)?.join(&manifest.model_file);
        let ready = fs::metadata(&path)
            .await
            .map(|metadata| metadata.is_file() && metadata.len() == manifest.byte_length)
            .unwrap_or(false);
        (
            ready,
            if ready {
                "verified_on_import"
            } else {
                "missing_or_changed"
            },
        )
    } else {
        (false, "not_configured")
    };
    Ok(WhisperStatus {
        schema: "homeserver.local-whisper-status.v1",
        engine: ENGINE_ID,
        model_ready,
        model_sha256: manifest.as_ref().map(|value| value.model_sha256.clone()),
        model_byte_length: manifest.as_ref().map(|value| value.byte_length),
        imported_at_utc: manifest.as_ref().map(|value| value.imported_at_utc.clone()),
        last_verified_at_utc: manifest
            .as_ref()
            .map(|value| value.last_verified_at_utc.clone()),
        verification_state: verification_state.to_owned(),
        active_transcription_id: active_id(&state).await,
        capabilities: serde_json::json!({
            "embedded_whisper_cpp": true,
            "sample_rate_hz": SAMPLE_RATE_HZ,
            "channels": 1,
            "maximum_audio_seconds": MAX_PCM_SAMPLES / SAMPLE_RATE_HZ as usize,
            "partial_transcripts": true,
            "cancellation": true,
            "cloud_speech": false,
            "raw_audio_persistence": false,
            "model_download": false
        }),
    })
}

#[tauri::command]
pub(crate) async fn homeserver_import_whisper_model(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
    expected_sha256: String,
    confirmation: String,
) -> Result<Option<WhisperStatus>, String> {
    if active_id(&state).await.is_some() {
        return Err("A local transcription is active; the model cannot be replaced.".to_owned());
    }
    let expected_sha256 = validate_sha256(&expected_sha256)?;
    if confirmation != format!("IMPORT WHISPER MODEL {expected_sha256}") {
        return Err("Exact local Whisper model import confirmation is required.".to_owned());
    }
    let Some(source) = AsyncFileDialog::new()
        .add_filter("Whisper GGML model", &["bin"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };
    let source_path = source.path().to_path_buf();
    let metadata = fs::metadata(&source_path)
        .await
        .map_err(|error| error.to_string())?;
    if !metadata.is_file() || !(MIN_MODEL_BYTES..=MAX_MODEL_BYTES).contains(&metadata.len()) {
        return Err("Selected Whisper model is outside the supported file boundary.".to_owned());
    }

    let directory = speech_directory(&app)?;
    fs::create_dir_all(&directory)
        .await
        .map_err(|error| error.to_string())?;
    let model_file = format!("whisper-model-{expected_sha256}.bin");
    let destination = directory.join(&model_file);
    let temporary = directory.join(format!(".{model_file}.{}.tmp", Uuid::new_v4().simple()));
    let mut input = fs::File::open(&source_path)
        .await
        .map_err(|error| error.to_string())?;
    let mut output = fs::File::create(&temporary)
        .await
        .map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; COPY_BUFFER_BYTES];
    let copy_result = async {
        loop {
            let read = input
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            total = total
                .checked_add(read as u64)
                .ok_or_else(|| "Whisper model size overflow.".to_owned())?;
            if total > MAX_MODEL_BYTES {
                return Err("Whisper model exceeds the local size boundary.".to_owned());
            }
            hasher.update(&buffer[..read]);
            output
                .write_all(&buffer[..read])
                .await
                .map_err(|error| error.to_string())?;
        }
        output.sync_all().await.map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    }
    .await;
    buffer.fill(0);
    if let Err(error) = copy_result {
        drop(output);
        let _ = fs::remove_file(&temporary).await;
        return Err(error);
    }
    drop(output);
    let actual_sha256 = hex::encode(hasher.finalize());
    if actual_sha256 != expected_sha256 {
        let _ = fs::remove_file(&temporary).await;
        return Err("Whisper model SHA-256 did not match; the copied file was removed.".to_owned());
    }
    let model_backup = match install_temporary_file(&temporary, &destination).await {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_file(&temporary).await;
            return Err(error);
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(|error| error.to_string())?;
    }

    let previous = read_manifest(&app).await?;
    let imported_at_utc = now_utc();
    let manifest = WhisperModelManifest {
        schema: MANIFEST_SCHEMA.to_owned(),
        engine: ENGINE_ID.to_owned(),
        model_file,
        model_sha256: expected_sha256,
        byte_length: total,
        imported_at_utc: imported_at_utc.clone(),
        last_verified_at_utc: imported_at_utc,
    };
    if let Err(error) = write_manifest(&app, &manifest).await {
        let rollback_error = rollback_temporary_file(&destination, model_backup.as_deref())
            .await
            .err();
        return Err(match rollback_error {
            Some(rollback) => {
                format!("{error}; local Whisper model rollback also failed: {rollback}")
            }
            None => error,
        });
    }
    commit_temporary_file(model_backup.as_deref()).await;
    if let Some(previous) = previous {
        if previous.model_file != manifest.model_file {
            let old = directory.join(previous.model_file);
            let _ = fs::remove_file(old).await;
        }
    }
    homeserver_whisper_status(app, state).await.map(Some)
}

#[tauri::command]
pub(crate) async fn homeserver_remove_whisper_model(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
    confirmation: String,
) -> Result<WhisperStatus, String> {
    if active_id(&state).await.is_some() {
        return Err("A local transcription is active; the model cannot be removed.".to_owned());
    }
    if confirmation != "REMOVE LOCAL WHISPER MODEL" {
        return Err("Exact local Whisper model removal confirmation is required.".to_owned());
    }
    if let Some(manifest) = read_manifest(&app).await? {
        let model = speech_directory(&app)?.join(manifest.model_file);
        let _ = fs::remove_file(model).await;
    }
    let manifest = manifest_path(&app)?;
    let _ = fs::remove_file(manifest).await;
    homeserver_whisper_status(app, state).await
}

#[tauri::command]
pub(crate) async fn homeserver_cancel_whisper_transcription(
    state: State<'_, WhisperRuntimeState>,
    transcription_id: String,
) -> Result<bool, String> {
    let transcription_id = validate_identifier(&transcription_id, "transcription ID")?;
    let active = state.active.lock().await;
    let Some(active) = active.as_ref() else {
        return Ok(false);
    };
    if active.transcription_id != transcription_id {
        return Err("A different local transcription is active.".to_owned());
    }
    active.cancel.store(true, Ordering::SeqCst);
    Ok(true)
}

#[tauri::command]
pub(crate) async fn homeserver_whisper_transcribe(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
    request: WhisperTranscriptionRequest,
) -> Result<WhisperTranscriptionResult, String> {
    let segment_id = validate_identifier(&request.segment_id, "audio segment ID")?;
    let language = validate_language(request.language.clone())?;
    let samples = decode_pcm16(&request)?;
    let sample_count = samples.len();
    let duration_ms = (sample_count as u64 * 1000) / SAMPLE_RATE_HZ as u64;
    let transcription_id = format!("whisper_{}", Uuid::new_v4().simple());
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut active = state.active.lock().await;
        if active.is_some() {
            return Err("Only one local Whisper transcription may run at a time.".to_owned());
        }
        *active = Some(ActiveTranscription {
            transcription_id: transcription_id.clone(),
            cancel: cancel.clone(),
        });
    }

    let result = async {
        let (model_path, manifest) = verified_model(&app).await?;
        let worker_app = app.clone();
        let worker_transcription_id = transcription_id.clone();
        let worker_segment_id = segment_id.clone();
        let worker_model_sha256 = manifest.model_sha256.clone();
        let worker_language = language.clone();
        let transcript = tokio::task::spawn_blocking(move || {
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
            schema: "homeserver.local-whisper-transcript.v1",
            transcription_id: transcription_id.clone(),
            segment_id: segment_id.clone(),
            engine: ENGINE_ID,
            model_sha256: manifest.model_sha256,
            language: language.clone(),
            sample_rate_hz: SAMPLE_RATE_HZ,
            sample_count,
            duration_ms,
            transcript,
            completed_at_utc: now_utc(),
            raw_audio_retained: false,
        })
    }
    .await;

    let mut active = state.active.lock().await;
    if active
        .as_ref()
        .is_some_and(|value| value.transcription_id == transcription_id)
    {
        *active = None;
    }
    drop(active);

    result
}

#[allow(clippy::too_many_arguments)]
fn run_whisper(
    app: AppHandle,
    model_path: PathBuf,
    samples: Zeroizing<Vec<f32>>,
    transcription_id: String,
    segment_id: String,
    model_sha256: String,
    language: String,
    cancel: Arc<AtomicBool>,
) -> Result<String, String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("Local Whisper transcription was cancelled.".to_owned());
    }
    let model_path = model_path
        .to_str()
        .ok_or_else(|| "Local Whisper model path is not valid UTF-8.".to_owned())?;
    let context = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
        .map_err(|error| format!("Unable to load the local Whisper model: {error}"))?;
    let mut whisper_state = context
        .create_state()
        .map_err(|error| format!("Unable to create the local Whisper state: {error}"))?;
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_translate(false);
    params.set_no_context(true);
    params.set_no_timestamps(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_n_threads(
        std::thread::available_parallelism()
            .map(|value| value.get().min(8) as i32)
            .unwrap_or(2),
    );
    if language == "auto" {
        params.set_detect_language(true);
        params.set_language(None);
    } else {
        params.set_language(Some(&language));
    }

    let partials = Arc::new(Mutex::new(BTreeMap::<i32, String>::new()));
    let segment_partials = partials.clone();
    let segment_app = app.clone();
    let segment_transcription_id = transcription_id.clone();
    let segment_segment_id = segment_id.clone();
    let segment_model_sha256 = model_sha256.clone();
    params.set_segment_callback_safe_lossy(Some(move |data| {
        if let Ok(mut values) = segment_partials.lock() {
            values.insert(data.segment, data.text);
            let combined = values.values().cloned().collect::<Vec<_>>().join(" ");
            let partial_transcript = combined
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(MAX_TRANSCRIPT_CHARS)
                .collect::<String>();
            let _ = segment_app.emit(
                "homeserver-whisper-progress",
                WhisperProgressEvent {
                    schema: "homeserver.local-whisper-progress.v1",
                    transcription_id: segment_transcription_id.clone(),
                    segment_id: segment_segment_id.clone(),
                    kind: "partial",
                    progress: -1,
                    partial_transcript,
                    model_sha256: segment_model_sha256.clone(),
                },
            );
        }
    }));

    let progress_app = app.clone();
    let progress_transcription_id = transcription_id.clone();
    let progress_segment_id = segment_id.clone();
    let progress_model_sha256 = model_sha256.clone();
    params.set_progress_callback_safe(Some(move |progress| {
        let _ = progress_app.emit(
            "homeserver-whisper-progress",
            WhisperProgressEvent {
                schema: "homeserver.local-whisper-progress.v1",
                transcription_id: progress_transcription_id.clone(),
                segment_id: progress_segment_id.clone(),
                kind: "progress",
                progress: progress.clamp(0, 100),
                partial_transcript: String::new(),
                model_sha256: progress_model_sha256.clone(),
            },
        );
    }));
    let abort = cancel.clone();
    params.set_abort_callback_safe(Some(move || abort.load(Ordering::SeqCst)));

    let run_result = whisper_state.full(params, samples.as_slice());
    if cancel.load(Ordering::SeqCst) {
        return Err("Local Whisper transcription was cancelled.".to_owned());
    }
    run_result.map_err(|error| format!("Local Whisper transcription failed: {error}"))?;

    let mut text = String::new();
    for segment in whisper_state.as_iter() {
        let value = segment
            .to_str_lossy()
            .map_err(|error| format!("Unable to read a local Whisper segment: {error}"))?;
        text.push_str(value.as_ref());
        text.push(' ');
    }
    let transcript = normalize_transcript(&text)?;
    let _ = app.emit(
        "homeserver-whisper-progress",
        WhisperProgressEvent {
            schema: "homeserver.local-whisper-progress.v1",
            transcription_id,
            segment_id,
            kind: "final",
            progress: 100,
            partial_transcript: transcript.clone(),
            model_sha256,
        },
    );
    Ok(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(samples: &[i16]) -> WhisperTranscriptionRequest {
        let bytes = samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect::<Vec<_>>();
        WhisperTranscriptionRequest {
            segment_id: "audseg_test".to_owned(),
            pcm16_base64: STANDARD.encode(bytes),
            sample_rate_hz: SAMPLE_RATE_HZ,
            channels: 1,
            language: Some("en".to_owned()),
        }
    }

    #[test]
    fn validates_hash_and_identifiers() {
        assert_eq!(validate_sha256(&"a".repeat(64)).unwrap(), "a".repeat(64));
        assert!(validate_sha256("nope").is_err());
        assert!(validate_identifier("audseg_test-1", "id").is_ok());
        assert!(validate_identifier("../unsafe", "id").is_err());
    }

    #[test]
    fn decodes_bounded_pcm16() {
        let samples = vec![123_i16; MIN_PCM_SAMPLES];
        let decoded = decode_pcm16(&request(&samples)).unwrap();
        assert_eq!(decoded.len(), MIN_PCM_SAMPLES);
        assert!(decoded.iter().all(|value| value.is_finite()));
        assert!(decode_pcm16(&request(&[1_i16; 100])).is_err());
    }

    #[test]
    fn normalizes_and_bounds_transcripts() {
        assert_eq!(
            normalize_transcript(" hello\n  local   world ").unwrap(),
            "hello local world"
        );
        assert!(normalize_transcript("   ").is_err());
        assert!(normalize_transcript(&"x".repeat(MAX_TRANSCRIPT_CHARS + 1)).is_err());
    }

    #[test]
    fn validates_language_boundary() {
        assert_eq!(validate_language(Some("AUTO".to_owned())).unwrap(), "auto");
        assert_eq!(validate_language(None).unwrap(), "en");
        assert!(validate_language(Some("en<script>".to_owned())).is_err());
    }

    #[tokio::test]
    async fn atomic_replacement_commits_and_rolls_back() {
        let root = std::env::temp_dir().join(format!(
            "homeserver-whisper-atomic-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).await.unwrap();
        let destination = root.join("model.bin");
        let temporary = root.join("model.tmp");
        fs::write(&destination, b"old").await.unwrap();
        fs::write(&temporary, b"new").await.unwrap();

        let backup = install_temporary_file(&temporary, &destination)
            .await
            .unwrap();
        assert_eq!(fs::read(&destination).await.unwrap(), b"new");
        assert!(backup.as_ref().is_some_and(|path| path.exists()));

        rollback_temporary_file(&destination, backup.as_deref())
            .await
            .unwrap();
        assert_eq!(fs::read(&destination).await.unwrap(), b"old");

        let second = root.join("second.tmp");
        fs::write(&second, b"committed").await.unwrap();
        let backup = install_temporary_file(&second, &destination).await.unwrap();
        commit_temporary_file(backup.as_deref()).await;
        assert_eq!(fs::read(&destination).await.unwrap(), b"committed");
        assert!(backup.as_ref().is_none_or(|path| !path.exists()));
        fs::remove_dir_all(root).await.unwrap();
    }
}
