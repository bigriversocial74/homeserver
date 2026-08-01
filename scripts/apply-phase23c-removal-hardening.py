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
    """async fn commit_temporary_file(backup: Option<&Path>) {
    if let Some(backup_path) = backup {
        let _ = fs::remove_file(backup_path).await;
    }
}

async fn write_manifest(app: &AppHandle, manifest: &WhisperModelManifest) -> Result<(), String> {
""",
    """async fn commit_temporary_file(backup: Option<&Path>) {
    if let Some(backup_path) = backup {
        let _ = fs::remove_file(backup_path).await;
    }
}

async fn cleanup_staged_speech_directories(directory: &Path) -> Result<(), String> {
    let Some(parent) = directory.parent() else {
        return Err("Local Whisper speech directory has no parent.".to_owned());
    };
    let Some(directory_name) = directory.file_name().and_then(|value| value.to_str()) else {
        return Err("Local Whisper speech directory has an invalid name.".to_owned());
    };
    let prefix = format!(".{directory_name}.");
    let mut entries = match fs::read_dir(parent).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".removing") {
            let file_type = entry.file_type().await.map_err(|error| error.to_string())?;
            if file_type.is_dir() {
                fs::remove_dir_all(entry.path())
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

async fn stage_speech_directory_removal(directory: &Path) -> Result<Option<PathBuf>, String> {
    if !fs::try_exists(directory)
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    let parent = directory
        .parent()
        .ok_or_else(|| "Local Whisper speech directory has no parent.".to_owned())?;
    let directory_name = directory
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Local Whisper speech directory has an invalid name.".to_owned())?;
    let staged = parent.join(format!(
        ".{directory_name}.{}.removing",
        Uuid::new_v4().simple()
    ));
    fs::rename(directory, &staged)
        .await
        .map_err(|error| error.to_string())?;
    if let Err(error) = fs::create_dir_all(directory).await {
        let _ = fs::rename(&staged, directory).await;
        return Err(error.to_string());
    }
    Ok(Some(staged))
}

async fn rollback_staged_speech_directory(
    directory: &Path,
    staged: Option<&Path>,
) -> Result<(), String> {
    let Some(staged) = staged else {
        return Ok(());
    };
    if fs::try_exists(directory)
        .await
        .map_err(|error| error.to_string())?
    {
        fs::remove_dir_all(directory)
            .await
            .map_err(|error| error.to_string())?;
    }
    fs::rename(staged, directory)
        .await
        .map_err(|error| error.to_string())
}

async fn commit_staged_speech_directory(staged: Option<&Path>) -> Result<(), String> {
    if let Some(staged) = staged {
        fs::remove_dir_all(staged)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn write_manifest(app: &AppHandle, manifest: &WhisperModelManifest) -> Result<(), String> {
""",
    "staged speech-directory helpers",
)

replace_exact(
    native,
    """pub(crate) async fn homeserver_whisper_status(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
) -> Result<WhisperStatus, String> {
    let manifest = read_manifest(&app).await?;
""",
    """pub(crate) async fn homeserver_whisper_status(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
) -> Result<WhisperStatus, String> {
    if let Ok(_model_operation) = state.model_operation.clone().try_lock_owned() {
        cleanup_staged_speech_directories(&speech_directory(&app)?).await?;
    }
    let manifest = read_manifest(&app).await?;
""",
    "nonblocking removal recovery",
)

replace_exact(
    native,
    """    let source_path = source.path().to_path_buf();
    let _model_operation = state.model_operation.clone().lock_owned().await;
    if active_id(&state).await.is_some() {
""",
    """    let source_path = source.path().to_path_buf();
    let _model_operation = state.model_operation.clone().lock_owned().await;
    cleanup_staged_speech_directories(&speech_directory(&app)?).await?;
    if active_id(&state).await.is_some() {
""",
    "import removal recovery",
)

replace_exact(
    native,
    """pub(crate) async fn homeserver_remove_whisper_model(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
    confirmation: String,
) -> Result<WhisperStatus, String> {
    let _model_operation = state.model_operation.clone().lock_owned().await;
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
""",
    """pub(crate) async fn homeserver_remove_whisper_model(
    app: AppHandle,
    state: State<'_, WhisperRuntimeState>,
    confirmation: String,
) -> Result<WhisperStatus, String> {
    let _model_operation = state.model_operation.clone().lock_owned().await;
    if active_id(&state).await.is_some() {
        return Err("A local transcription is active; the model cannot be removed.".to_owned());
    }
    if confirmation != "REMOVE LOCAL WHISPER MODEL" {
        return Err("Exact local Whisper model removal confirmation is required.".to_owned());
    }
    let directory = speech_directory(&app)?;
    cleanup_staged_speech_directories(&directory).await?;
    let staged = stage_speech_directory_removal(&directory).await?;
    if let Err(error) = commit_staged_speech_directory(staged.as_deref()).await {
        let rollback_error = rollback_staged_speech_directory(&directory, staged.as_deref())
            .await
            .err();
        return Err(match rollback_error {
            Some(rollback) => format!(
                "{error}; local Whisper model removal rollback also failed: {rollback}"
            ),
            None => error,
        });
    }
    homeserver_whisper_status(app, state).await
}
""",
    "atomic whole-directory removal",
)

replace_exact(
    native,
    """    #[tokio::test]
    async fn atomic_replacement_commits_and_rolls_back() {
""",
    """    #[tokio::test]
    async fn speech_directory_removal_commits_and_rolls_back() {
        let root = std::env::temp_dir().join(format!(
            "homeserver-whisper-remove-{}",
            Uuid::new_v4().simple()
        ));
        let directory = root.join("speech");
        fs::create_dir_all(&directory).await.unwrap();
        fs::write(directory.join(MANIFEST_FILE), b"manifest")
            .await
            .unwrap();
        fs::write(directory.join("model.ggml"), b"model")
            .await
            .unwrap();

        let staged = stage_speech_directory_removal(&directory).await.unwrap();
        assert!(directory.exists());
        assert!(!directory.join(MANIFEST_FILE).exists());
        assert!(staged.as_ref().is_some_and(|path| path.exists()));
        rollback_staged_speech_directory(&directory, staged.as_deref())
            .await
            .unwrap();
        assert!(directory.join(MANIFEST_FILE).exists());
        assert!(directory.join("model.ggml").exists());

        let staged = stage_speech_directory_removal(&directory).await.unwrap();
        commit_staged_speech_directory(staged.as_deref())
            .await
            .unwrap();
        assert!(directory.exists());
        assert!(!directory.join(MANIFEST_FILE).exists());
        assert!(staged.as_ref().is_none_or(|path| !path.exists()));
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn atomic_replacement_commits_and_rolls_back() {
""",
    "atomic removal native test",
)

service = ROOT / "crates/homeserver-service/src/audio_runtime.rs"
replace_exact(
    service,
    """        && value[LOCAL_WHISPER_ID_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
""",
    """        && value[LOCAL_WHISPER_ID_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
""",
    "lowercase Whisper ID boundary",
)
replace_exact(
    service,
    """        assert!(!valid_local_whisper_transcription_id(
            "other_0123456789abcdef0123456789abcdef"
        ));
""",
    """        assert!(!valid_local_whisper_transcription_id(
            "other_0123456789abcdef0123456789abcdef"
        ));
        assert!(!valid_local_whisper_transcription_id(
            "whisper_0123456789ABCDEF0123456789ABCDEF"
        ));
""",
    "uppercase Whisper ID rejection test",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    """    ("model_operation_lock_serializes_mutation_and_inference", "native operation lock test"),
):
""",
    """    ("model_operation_lock_serializes_mutation_and_inference", "native operation lock test"),
    ("cleanup_staged_speech_directories", "interrupted removal recovery"),
    ("stage_speech_directory_removal", "atomic whole-directory removal"),
    ("rollback_staged_speech_directory", "removal rollback"),
    ("commit_staged_speech_directory", "removal commit"),
    ("speech_directory_removal_commits_and_rolls_back", "native removal transaction test"),
):
""",
    "removal validator requirements",
)
replace_exact(
    validator,
    """    ("local_whisper_receipt_boundaries_are_closed", "service receipt tests"),
):
""",
    """    ("local_whisper_receipt_boundaries_are_closed", "service receipt tests"),
    ("matches!(byte, b'a'..=b'f')", "lowercase transcription ID boundary"),
):
""",
    "lowercase receipt validator requirement",
)
replace_exact(
    validator,
    """    ROOT / ".github/workflows/phase23c-final-contract-hardening.yml",
):
""",
    """    ROOT / ".github/workflows/phase23c-final-contract-hardening.yml",
    ROOT / "scripts/apply-phase23c-removal-hardening.py",
    ROOT / ".github/workflows/phase23c-removal-hardening.yml",
):
""",
    "removal cleanup denylist",
)

print("Applied Phase 23C atomic speech-directory removal and lowercase receipt hardening.")
