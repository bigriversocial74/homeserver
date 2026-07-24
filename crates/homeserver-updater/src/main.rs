use anyhow::{bail, ensure, Context, Result};
use chrono::Utc;
use microgifter_homeserver_core::{
    HealthSnapshot, UpdateApplicationPlan, UpdateApplicationResult, UpdateState,
    UPDATE_MANIFEST_SCHEMA_VERSION,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

const MAX_PLAN_BYTES: u64 = 64 * 1024;
const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ROLLBACK_FILES: usize = 20_000;
const MAX_ROLLBACK_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    let command = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .unwrap_or_default();
    let plan_path = arguments
        .next()
        .map(PathBuf::from)
        .context("an update plan path is required")?;
    ensure!(arguments.next().is_none(), "unexpected updater arguments");
    ensure!(command == "apply", "expected updater command 'apply'");

    let plan = read_plan(&plan_path)?;
    let result = match apply_update(&plan).await {
        Ok(result) => result,
        Err(error) => UpdateApplicationResult {
            schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
            update_id: plan.update_id.clone(),
            target_version: plan.target_version.clone(),
            state: UpdateState::Failed,
            message: "HomeServer update failed before a verified installation was available."
                .to_owned(),
            failure_code: Some(public_failure_code(&error)),
            completed_at_utc: Utc::now(),
        },
    };
    write_result(Path::new(&plan.result_path), &result)?;

    match result.state {
        UpdateState::Succeeded | UpdateState::RolledBack => Ok(()),
        _ => bail!("{}", result.failure_code.as_deref().unwrap_or("update_failed")),
    }
}

fn read_plan(path: &Path) -> Result<UpdateApplicationPlan> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("unable to read update plan metadata at {}", path.display()))?;
    ensure!(metadata.is_file(), "update plan is not a regular file");
    ensure!(metadata.len() > 2 && metadata.len() <= MAX_PLAN_BYTES, "update plan size is invalid");
    let bytes = fs::read(path).context("unable to read update plan")?;
    let plan: UpdateApplicationPlan =
        serde_json::from_slice(&bytes).context("update plan JSON is invalid")?;
    validate_plan(&plan)?;
    Ok(plan)
}

fn validate_plan(plan: &UpdateApplicationPlan) -> Result<()> {
    ensure!(
        plan.schema_version == UPDATE_MANIFEST_SCHEMA_VERSION,
        "unsupported update plan schema"
    );
    ensure!(valid_identifier(&plan.update_id), "update identity is invalid");
    ensure!(!plan.current_version.trim().is_empty(), "current version is missing");
    ensure!(!plan.target_version.trim().is_empty(), "target version is missing");
    ensure!(plan.service_name == "MicrogifterHomeServer", "service identity is invalid");
    ensure!(plan.health_url == "http://127.0.0.1:47831", "health URL must remain loopback-only");
    ensure!(valid_sha256(&plan.installer_sha256), "installer SHA-256 is invalid");
    ensure!(
        (1_000_000..=MAX_INSTALLER_BYTES).contains(&plan.installer_size_bytes),
        "installer size is outside the supported range"
    );
    ensure!(valid_thumbprint(&plan.authenticode_thumbprint), "Authenticode thumbprint is invalid");

    let data_dir = absolute(Path::new(&plan.data_dir))?;
    let installer_path = absolute(Path::new(&plan.installer_path))?;
    let rollback_dir = absolute(Path::new(&plan.rollback_dir))?;
    let archived_installer = absolute(Path::new(&plan.archived_installer_path))?;
    let result_path = absolute(Path::new(&plan.result_path))?;
    let install_dir = absolute(Path::new(&plan.install_dir))?;

    ensure!(installer_path.starts_with(&data_dir), "installer must be staged inside HomeServer data");
    ensure!(rollback_dir.starts_with(&data_dir), "rollback directory must be inside HomeServer data");
    ensure!(archived_installer.starts_with(&data_dir), "installer archive must be inside HomeServer data");
    ensure!(result_path.starts_with(&data_dir), "update result must be inside HomeServer data");
    ensure!(!install_dir.starts_with(&data_dir), "install and data directories must be separate");
    ensure!(install_dir.is_dir(), "HomeServer install directory is unavailable");
    Ok(())
}

async fn apply_update(plan: &UpdateApplicationPlan) -> Result<UpdateApplicationResult> {
    let installer_path = PathBuf::from(&plan.installer_path);
    verify_file(
        &installer_path,
        plan.installer_size_bytes,
        &plan.installer_sha256,
    )?;
    verify_authenticode(&installer_path, &plan.authenticode_thumbprint)?;

    let install_dir = PathBuf::from(&plan.install_dir);
    let rollback_dir = PathBuf::from(&plan.rollback_dir);
    stop_service(&plan.service_name)?;
    snapshot_directory(&install_dir, &rollback_dir)?;

    let install_result = Command::new(&installer_path)
        .arg("/S")
        .status()
        .context("unable to start the staged HomeServer installer")?;
    let installation_healthy = install_result.success()
        && wait_for_health(&plan.health_url, &plan.target_version, Duration::from_secs(120)).await;

    if installation_healthy {
        let archived_installer = PathBuf::from(&plan.archived_installer_path);
        if let Some(parent) = archived_installer.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_copy(&installer_path, &archived_installer)?;
        return Ok(UpdateApplicationResult {
            schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
            update_id: plan.update_id.clone(),
            target_version: plan.target_version.clone(),
            state: UpdateState::Succeeded,
            message: format!("Microgifter HomeServer {} installed and passed health verification.", plan.target_version),
            failure_code: None,
            completed_at_utc: Utc::now(),
        });
    }

    let failure_code = if install_result.success() {
        "post_update_health_failed"
    } else {
        "installer_failed"
    };
    rollback_installation(plan, &install_dir, &rollback_dir).await?;
    Ok(UpdateApplicationResult {
        schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
        update_id: plan.update_id.clone(),
        target_version: plan.target_version.clone(),
        state: UpdateState::RolledBack,
        message: "The update did not pass health verification. The previous HomeServer binaries were restored."
            .to_owned(),
        failure_code: Some(failure_code.to_owned()),
        completed_at_utc: Utc::now(),
    })
}

async fn rollback_installation(
    plan: &UpdateApplicationPlan,
    install_dir: &Path,
    rollback_dir: &Path,
) -> Result<()> {
    stop_service(&plan.service_name)?;
    if install_dir.exists() {
        fs::remove_dir_all(install_dir).context("unable to remove failed HomeServer installation")?;
    }
    copy_directory(rollback_dir, install_dir)?;
    start_service(&plan.service_name)?;
    ensure!(
        wait_for_health(&plan.health_url, &plan.current_version, Duration::from_secs(90)).await,
        "binary rollback completed but the previous HomeServer service did not become healthy"
    );
    Ok(())
}

fn verify_file(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("unable to inspect staged installer at {}", path.display()))?;
    ensure!(metadata.is_file(), "staged installer is not a regular file");
    ensure!(metadata.len() == expected_size, "staged installer size does not match the signed manifest");

    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    ensure!(
        hex::encode(hasher.finalize()).eq_ignore_ascii_case(expected_sha256),
        "staged installer SHA-256 does not match the signed manifest"
    );
    Ok(())
}

#[cfg(windows)]
fn verify_authenticode(path: &Path, expected_thumbprint: &str) -> Result<()> {
    let script = r#"$signature = Get-AuthenticodeSignature -LiteralPath $env:MG_UPDATE_FILE
if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate) { exit 20 }
[Console]::Out.Write($signature.SignerCertificate.Thumbprint)"#;
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", script])
        .env("MG_UPDATE_FILE", path)
        .output()
        .context("unable to execute Windows Authenticode verification")?;
    ensure!(output.status.success(), "staged installer does not have a valid trusted Authenticode signature");
    let actual = String::from_utf8(output.stdout)?.trim().replace(' ', "");
    ensure!(
        actual.eq_ignore_ascii_case(expected_thumbprint),
        "staged installer Authenticode signer does not match the signed manifest"
    );
    Ok(())
}

#[cfg(not(windows))]
fn verify_authenticode(_path: &Path, _expected_thumbprint: &str) -> Result<()> {
    bail!("Authenticode verification is only supported on Windows")
}

fn snapshot_directory(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    copy_directory(source, destination)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    ensure!(source.is_dir(), "source directory is unavailable");
    let mut file_count = 0_usize;
    let mut byte_count = 0_u64;
    copy_directory_inner(source, destination, &mut file_count, &mut byte_count)
}

fn copy_directory_inner(
    source: &Path,
    destination: &Path,
    file_count: &mut usize,
    byte_count: &mut u64,
) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(!metadata.file_type().is_symlink(), "symbolic links are not allowed in update rollback trees");
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            copy_directory_inner(&entry.path(), &target, file_count, byte_count)?;
        } else if metadata.is_file() {
            *file_count = file_count.checked_add(1).context("rollback file count overflow")?;
            *byte_count = byte_count
                .checked_add(metadata.len())
                .context("rollback byte count overflow")?;
            ensure!(*file_count <= MAX_ROLLBACK_FILES, "rollback snapshot contains too many files");
            ensure!(*byte_count <= MAX_ROLLBACK_BYTES, "rollback snapshot exceeds the size limit");
            fs::copy(entry.path(), target)?;
        } else {
            bail!("unsupported file type in HomeServer installation directory");
        }
    }
    Ok(())
}

fn atomic_copy(source: &Path, destination: &Path) -> Result<()> {
    let temporary = destination.with_extension("tmp");
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    fs::copy(source, &temporary)?;
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(temporary, destination)?;
    Ok(())
}

fn stop_service(service_name: &str) -> Result<()> {
    let _ = Command::new("sc.exe").args(["stop", service_name]).status();
    wait_for_service_state(service_name, "STOPPED", Duration::from_secs(45))
}

fn start_service(service_name: &str) -> Result<()> {
    let output = Command::new("sc.exe")
        .args(["start", service_name])
        .output()
        .context("unable to start HomeServer service")?;
    ensure!(output.status.success() || wait_for_service_state(service_name, "RUNNING", Duration::from_secs(5)).is_ok(), "unable to start HomeServer service");
    Ok(())
}

fn wait_for_service_state(service_name: &str, state: &str, timeout: Duration) -> Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        let output = Command::new("sc.exe").args(["query", service_name]).output()?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains(state) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("HomeServer service did not reach {state}")
}

async fn wait_for_health(base_url: &str, expected_version: &str, timeout: Duration) -> bool {
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        let healthy = match client.get(format!("{base_url}/healthz")).send().await {
            Ok(response) if response.status().as_u16() == 204 => client
                .get(format!("{base_url}/v1/status"))
                .send()
                .await
                .ok()
                .and_then(|response| response.error_for_status().ok()),
            _ => None,
        };
        if let Some(response) = healthy {
            if let Ok(snapshot) = response.json::<HealthSnapshot>().await {
                if snapshot.version == expected_version && snapshot.api_available {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    false
}

fn write_result(path: &Path, result: &UpdateApplicationResult) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(result)?)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn absolute(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "update path must be absolute");
    Ok(path.to_path_buf())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.:".contains(character))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn valid_thumbprint(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn public_failure_code(error: &anyhow::Error) -> String {
    let text = error.to_string().to_lowercase();
    if text.contains("authenticode") {
        "authenticode_verification_failed"
    } else if text.contains("sha-256") || text.contains("size") {
        "installer_integrity_failed"
    } else if text.contains("rollback") {
        "rollback_failed"
    } else if text.contains("service") {
        "service_transition_failed"
    } else {
        "update_application_failed"
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn update_identifiers_are_bounded() {
        assert!(valid_identifier("update:0.2.0-abc"));
        assert!(!valid_identifier("invalid update"));
        assert!(!valid_identifier(&"x".repeat(101)));
    }

    #[test]
    fn rollback_copy_rejects_symbolic_links_and_preserves_files() {
        let directory = tempdir().expect("temporary directory");
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        fs::create_dir_all(source.join("resources")).expect("source tree");
        fs::write(source.join("app.exe"), b"binary").expect("binary fixture");
        fs::write(source.join("resources/service.exe"), b"service").expect("service fixture");
        copy_directory(&source, &destination).expect("copy directory");
        assert_eq!(fs::read(destination.join("app.exe")).unwrap(), b"binary");
        assert_eq!(
            fs::read(destination.join("resources/service.exe")).unwrap(),
            b"service"
        );
    }
}
