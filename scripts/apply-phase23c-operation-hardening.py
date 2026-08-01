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
    """#[derive(Default)]
pub(crate) struct WhisperRuntimeState {
    active: AsyncMutex<Option<ActiveTranscription>>,
}
""",
    """#[derive(Default)]
pub(crate) struct WhisperRuntimeState {
    active: AsyncMutex<Option<ActiveTranscription>>,
    model_operation: Arc<AsyncMutex<()>>,
}
""",
    "model operation gate",
)

replace_exact(
    native,
    """    if active_id(&state).await.is_some() {
        return Err("A local transcription is active; the model cannot be replaced.".to_owned());
    }
    let expected_sha256 = validate_sha256(&expected_sha256)?;
""",
    """    let expected_sha256 = validate_sha256(&expected_sha256)?;
""",
    "deferred import activity check",
)

replace_exact(
    native,
    """    let source_path = source.path().to_path_buf();
    let metadata = fs::metadata(&source_path)
""",
    """    let source_path = source.path().to_path_buf();
    let _model_operation = state.model_operation.clone().lock_owned().await;
    if active_id(&state).await.is_some() {
        return Err("A local transcription is active; the model cannot be replaced.".to_owned());
    }
    let metadata = fs::metadata(&source_path)
""",
    "serialized model import",
)

replace_exact(
    native,
    """    let model_backup = match install_temporary_file(&temporary, &destination).await {
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
""",
    """    let previous = match read_manifest(&app).await {
        Ok(previous) => previous,
        Err(error) => {
            let _ = fs::remove_file(&temporary).await;
            return Err(error);
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) =
            fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600)).await
        {
            let _ = fs::remove_file(&temporary).await;
            return Err(error.to_string());
        }
    }

    let model_backup = match install_temporary_file(&temporary, &destination).await {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_file(&temporary).await;
            return Err(error);
        }
    };
""",
    "pre-commit import validation",
)

replace_exact(
    native,
    """    if active_id(&state).await.is_some() {
        return Err("A local transcription is active; the model cannot be removed.".to_owned());
    }
    if confirmation != "REMOVE LOCAL WHISPER MODEL" {
""",
    """    let _model_operation = state.model_operation.clone().lock_owned().await;
    if active_id(&state).await.is_some() {
        return Err("A local transcription is active; the model cannot be removed.".to_owned());
    }
    if confirmation != "REMOVE LOCAL WHISPER MODEL" {
""",
    "serialized model removal",
)

replace_exact(
    native,
    """        *active = Some(ActiveTranscription {
            transcription_id: transcription_id.clone(),
            cancel: cancel.clone(),
        });
    }

    let result = async {
""",
    """        *active = Some(ActiveTranscription {
            transcription_id: transcription_id.clone(),
            cancel: cancel.clone(),
        });
    }
    let _model_operation = state.model_operation.clone().lock_owned().await;

    let result = async {
""",
    "serialized transcription model access",
)

replace_exact(
    native,
    """    #[tokio::test]
    async fn atomic_replacement_commits_and_rolls_back() {
""",
    """    #[tokio::test]
    async fn model_operation_lock_serializes_mutation_and_inference() {
        let runtime = WhisperRuntimeState::default();
        let first = runtime.model_operation.clone().lock_owned().await;
        assert!(runtime.model_operation.clone().try_lock_owned().is_err());
        drop(first);
        assert!(runtime.model_operation.clone().try_lock_owned().is_ok());
    }

    #[tokio::test]
    async fn atomic_replacement_commits_and_rolls_back() {
""",
    "model operation serialization test",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    """    ("atomic_replacement_commits_and_rolls_back", "native rollback test"),
):
""",
    """    ("atomic_replacement_commits_and_rolls_back", "native rollback test"),
    ("model_operation: Arc<AsyncMutex<()>>", "shared model operation gate"),
    ("state.model_operation.clone().lock_owned().await", "serialized model operations"),
    ("fs::set_permissions(&temporary", "pre-install model permissions"),
    ("let previous = match read_manifest(&app).await", "pre-install manifest validation"),
    ("model_operation_lock_serializes_mutation_and_inference", "native operation lock test"),
):
""",
    "operation hardening validator",
)
replace_exact(
    validator,
    """    (NATIVE, "fs::remove_file(&path)", "delete-before-replace manifest update"),
):
""",
    """    (NATIVE, "fs::remove_file(&path)", "delete-before-replace manifest update"),
    (NATIVE, "fs::set_permissions(&destination", "post-install permission mutation"),
):
""",
    "post-install permission prohibition",
)
replace_exact(
    validator,
    """    ROOT / "phase23c-whisper-exit.txt",
):
""",
    """    ROOT / "phase23c-whisper-exit.txt",
    ROOT / "scripts/apply-phase23c-atomic-hardening.py",
    ROOT / ".github/workflows/phase23c-atomic-hardening.yml",
    ROOT / "scripts/apply-phase23c-operation-hardening.py",
    ROOT / ".github/workflows/phase23c-operation-hardening.yml",
):
""",
    "operation hardening cleanup denylist",
)

print("Applied Phase 23C model-operation serialization and pre-commit import hardening.")
