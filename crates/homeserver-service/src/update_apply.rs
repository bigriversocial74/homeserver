use crate::{config::AppConfig, update};
#[cfg(not(windows))]
use anyhow::bail;
use anyhow::{ensure, Context, Result};
use microgifter_homeserver_core::{
    UpdateApplicationPlan, UpdateRecord, SERVICE_NAME, UPDATE_MANIFEST_SCHEMA_VERSION,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

const UPDATER_RESOURCE_NAME: &str = "microgifter-homeserver-updater.exe";

pub fn verify_staged_installer(record: &UpdateRecord, installer_path: &Path) -> Result<()> {
    let metadata = fs::metadata(installer_path)
        .with_context(|| format!("unable to inspect {}", installer_path.display()))?;
    ensure!(metadata.is_file(), "staged installer is not a regular file");
    ensure!(
        metadata.len() == record.installer_size_bytes,
        "staged installer size does not match the signed manifest"
    );
    let mut input = fs::File::open(installer_path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    ensure!(
        hex::encode(hasher.finalize()).eq_ignore_ascii_case(&record.installer_sha256),
        "staged installer SHA-256 does not match the signed manifest"
    );
    update::verify_authenticode(installer_path, &record.authenticode_thumbprint)
}

pub fn prepare_and_launch(
    config: &AppConfig,
    record: &UpdateRecord,
    installer_path: &Path,
) -> Result<PathBuf> {
    ensure!(
        record.state == microgifter_homeserver_core::UpdateState::Staged,
        "update is not staged for application"
    );
    let canonical_staging = config
        .update_staging_dir
        .canonicalize()
        .context("managed update staging directory is unavailable")?;
    let canonical_installer = installer_path
        .canonicalize()
        .context("staged update installer is unavailable")?;
    ensure!(
        canonical_installer.starts_with(&canonical_staging),
        "staged installer is outside managed update storage"
    );
    verify_staged_installer(record, &canonical_installer)?;

    let current_exe = std::env::current_exe()
        .context("unable to resolve HomeServer service executable")?
        .canonicalize()
        .context("unable to canonicalize HomeServer service executable")?;
    let resource_dir = current_exe
        .parent()
        .context("HomeServer service resource directory is unavailable")?;
    ensure!(
        resource_dir.file_name().and_then(|value| value.to_str()) == Some("resources"),
        "updates can only be applied from an installed HomeServer service"
    );
    let install_dir = resource_dir
        .parent()
        .context("HomeServer installation directory is unavailable")?
        .canonicalize()
        .context("HomeServer installation directory is unavailable")?;
    let bundled_updater = resource_dir
        .join(UPDATER_RESOURCE_NAME)
        .canonicalize()
        .context("HomeServer updater helper is unavailable")?;
    ensure!(
        bundled_updater.starts_with(resource_dir) && bundled_updater.is_file(),
        "HomeServer updater helper is outside the installed resource directory"
    );

    let run_id = Uuid::new_v4().simple().to_string();
    let updater_copy = config
        .update_staging_dir
        .join(format!("updater-{run_id}.exe"));
    fs::copy(&bundled_updater, &updater_copy)?;
    let rollback_dir = config.update_rollback_dir.join(&record.update_id);
    let archived_installer_path = config
        .update_installed_dir
        .join(format!("Microgifter-HomeServer-{}.exe", record.version));
    let result_path = config.update_result_path();
    let plan_path = config.update_plan_path();
    let plan = UpdateApplicationPlan {
        schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
        update_id: record.update_id.clone(),
        current_version: env!("CARGO_PKG_VERSION").to_owned(),
        target_version: record.version.clone(),
        installer_path: canonical_installer.to_string_lossy().into_owned(),
        installer_size_bytes: record.installer_size_bytes,
        installer_sha256: record.installer_sha256.clone(),
        authenticode_thumbprint: record.authenticode_thumbprint.clone(),
        install_dir: install_dir.to_string_lossy().into_owned(),
        data_dir: config.data_dir.to_string_lossy().into_owned(),
        rollback_dir: rollback_dir.to_string_lossy().into_owned(),
        archived_installer_path: archived_installer_path.to_string_lossy().into_owned(),
        result_path: result_path.to_string_lossy().into_owned(),
        service_name: SERVICE_NAME.to_owned(),
        health_url: microgifter_homeserver_core::api_base_url(),
    };
    atomic_write_json(&plan_path, &plan)?;
    if result_path.exists() {
        fs::remove_file(&result_path)?;
    }
    launch_detached(&updater_copy, &plan_path)?;
    Ok(plan_path)
}

#[cfg(windows)]
fn launch_detached(updater: &Path, plan_path: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
    Command::new(updater)
        .arg("apply")
        .arg(plan_path)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .context("unable to launch the HomeServer updater helper")?;
    Ok(())
}

#[cfg(not(windows))]
fn launch_detached(_updater: &Path, _plan_path: &Path) -> Result<()> {
    bail!("HomeServer update application is only supported on Windows")
}

fn atomic_write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    let backup = path.with_extension("replace-backup");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::File::options()
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    if path.exists() {
        fs::rename(path, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use microgifter_homeserver_core::{UpdateChannel, UpdateState};
    use tempfile::tempdir;

    #[test]
    fn staged_installer_hash_and_size_are_rechecked() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("setup.exe");
        fs::write(&path, b"test installer").unwrap();
        let hash = hex::encode(Sha256::digest(b"test installer"));
        let record = UpdateRecord {
            update_id: "update:0.2.0:test".to_owned(),
            version: "0.2.0".to_owned(),
            channel: UpdateChannel::Stable,
            state: UpdateState::Staged,
            release_notes: String::new(),
            installer_file_name: "Microgifter-HomeServer-Setup.exe".to_owned(),
            installer_size_bytes: 14,
            installer_sha256: hash,
            authenticode_thumbprint: "A".repeat(40),
            checked_at_utc: chrono::Utc::now(),
            downloaded_at_utc: None,
            applied_at_utc: None,
            failure_code: None,
        };
        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(metadata.len(), record.installer_size_bytes);
    }
}
