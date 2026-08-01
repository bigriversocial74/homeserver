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
    """async fn write_manifest(app: &AppHandle, manifest: &WhisperModelManifest) -> Result<(), String> {
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
    output
        .write_all(&bytes)
        .await
        .map_err(|error| error.to_string())?;
    output.sync_all().await.map_err(|error| error.to_string())?;
    drop(output);
    if fs::try_exists(&path).await.map_err(|error| error.to_string())? {
        fs::remove_file(&path)
            .await
            .map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, &path)
        .await
        .map_err(|error| error.to_string())
}
""",
    """async fn install_temporary_file(
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
        let backup = destination.with_file_name(format!(
            ".{file_name}.{}.backup",
            Uuid::new_v4().simple()
        ));
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

async fn rollback_temporary_file(
    destination: &Path,
    backup: Option<&Path>,
) -> Result<(), String> {
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
""",
    "atomic manifest replacement",
)

replace_exact(
    native,
    """    if fs::try_exists(&destination)
        .await
        .map_err(|error| error.to_string())?
    {
        fs::remove_file(&destination)
            .await
            .map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, &destination)
        .await
        .map_err(|error| error.to_string())?;

    #[cfg(unix)]
""",
    """    let model_backup = match install_temporary_file(&temporary, &destination).await {
        Ok(backup) => backup,
        Err(error) => {
            let _ = fs::remove_file(&temporary).await;
            return Err(error);
        }
    };

    #[cfg(unix)]
""",
    "atomic model replacement",
)

replace_exact(
    native,
    """    write_manifest(&app, &manifest).await?;
    if let Some(previous) = previous {
        if previous.model_file != manifest.model_file {
            let old = directory.join(previous.model_file);
            let _ = fs::remove_file(old).await;
        }
    }
""",
    """    if let Err(error) = write_manifest(&app, &manifest).await {
        let rollback_error = rollback_temporary_file(
            &destination,
            model_backup.as_deref(),
        )
        .await
        .err();
        return Err(match rollback_error {
            Some(rollback) => format!(
                "{error}; local Whisper model rollback also failed: {rollback}"
            ),
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
""",
    "manifest-failure model rollback",
)

replace_exact(
    native,
    """            partial_transcript: transcript,
            model_sha256,
        },
    );
    Ok(transcript)
""",
    """            partial_transcript: transcript.clone(),
            model_sha256,
        },
    );
    Ok(transcript)
""",
    "final transcript event clone",
)

replace_exact(
    native,
    """    #[test]
    fn validates_language_boundary() {
        assert_eq!(
            validate_language(Some("AUTO".to_owned())).unwrap(),
            "auto"
        );
        assert_eq!(validate_language(None).unwrap(), "en");
        assert!(validate_language(Some("en<script>".to_owned())).is_err());
    }
}
""",
    """    #[test]
    fn validates_language_boundary() {
        assert_eq!(
            validate_language(Some("AUTO".to_owned())).unwrap(),
            "auto"
        );
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
        let backup = install_temporary_file(&second, &destination)
            .await
            .unwrap();
        commit_temporary_file(backup.as_deref()).await;
        assert_eq!(fs::read(&destination).await.unwrap(), b"committed");
        assert!(backup.as_ref().is_none_or(|path| !path.exists()));
        fs::remove_dir_all(root).await.unwrap();
    }
}
""",
    "atomic replacement native test",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    """    ("fs::remove_file(&temporary)", "failed-import cleanup"),
):
""",
    """    ("fs::remove_file(&temporary)", "failed-import cleanup"),
    ("install_temporary_file", "atomic file replacement"),
    ("rollback_temporary_file", "replacement rollback"),
    ("commit_temporary_file", "post-commit backup cleanup"),
    ("model_backup.as_deref()", "model replacement receipt"),
    ("partial_transcript: transcript.clone()", "non-consuming final transcript event"),
    ("atomic_replacement_commits_and_rolls_back", "native rollback test"),
):
""",
    "atomic hardening validator",
)
replace_exact(
    validator,
    """    (NATIVE, "worker_transcript_placeholder", "placeholder transcript receipt"),
):
""",
    """    (NATIVE, "worker_transcript_placeholder", "placeholder transcript receipt"),
    (NATIVE, "fs::remove_file(&destination)", "delete-before-replace model update"),
    (NATIVE, "fs::remove_file(&path)", "delete-before-replace manifest update"),
):
""",
    "delete-before-replace prohibition",
)

print("Applied Phase 23C atomic replacement and final transcript hardening.")
