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
    if matches!(command.as_str(), "version" | "--version" | "-V") {
        ensure!(arguments.next().is_none(), "unexpected updater arguments");
        println!("MicrogifterHomeServerUpdater {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let plan_path = arguments
        .next()
        .map(PathBuf::from)
        .context("an update plan path is required")?;
    ensure!(arguments.next().is_none(), "unexpected updater arguments");
    ensure!(command == "apply", "expected updater command 'apply'");

    let plan = read_plan(&plan_path)?;
    let result = match apply_update(&plan).await {
        Ok(result) => result,
        Err(error) => {
            eprintln!("HomeServer updater internal failure: {error:#}");
            UpdateApplicationResult {
                schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
                update_id: plan.update_id.clone(),
                target_version: plan.target_version.clone(),
                state: UpdateState::Failed,
                message: "HomeServer update failed before a verified installation was available."
                    .to_owned(),
                failure_code: Some(public_failure_code(&error)),
                completed_at_utc: Utc::now(),
            }
        }
    };
    write_result(Path::new(&plan.result_path), &result)?;

    match result.state {
        UpdateState::Succeeded | UpdateState::RolledBack => Ok(()),
        _ => bail!(
            "{}",
            result.failure_code.as_deref().unwrap_or("update_failed")
        ),
    }
}

fn read_plan(path: &Path) -> Result<UpdateApplicationPlan> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("unable to read update plan metadata at {}", path.display()))?;
    ensure!(metadata.is_file(), "update plan is not a regular file");
    ensure!(
        metadata.len() > 2 && metadata.len() <= MAX_PLAN_BYTES,
        "update plan size is invalid"
    );
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
    ensure!(
        valid_identifier(&plan.update_id),
        "update identity is invalid"
    );
    ensure!(
        !plan.current_version.trim().is_empty(),
        "current version is missing"
    );
    ensure!(
        !plan.target_version.trim().is_empty(),
        "target version is missing"
    );
    ensure!(
        plan.service_name == "MicrogifterHomeServer",
        "service identity is invalid"
    );
    ensure!(
        plan.health_url == "http://127.0.0.1:47831",
        "health URL must remain loopback-only"
    );
    ensure!(
        valid_sha256(&plan.installer_sha256),
        "installer SHA-256 is invalid"
    );
    ensure!(
        (1_000_000..=MAX_INSTALLER_BYTES).contains(&plan.installer_size_bytes),
        "installer size is outside the supported range"
    );
    ensure!(
        valid_thumbprint(&plan.authenticode_thumbprint),
        "Authenticode thumbprint is invalid"
    );

    let data_dir = absolute(Path::new(&plan.data_dir))?;
    let installer_path = absolute(Path::new(&plan.installer_path))?;
    let rollback_dir = absolute(Path::new(&plan.rollback_dir))?;
    let archived_installer = absolute(Path::new(&plan.archived_installer_path))?;
    let result_path = absolute(Path::new(&plan.result_path))?;
    let install_dir = absolute(Path::new(&plan.install_dir))?;

    ensure!(
        installer_path.starts_with(&data_dir),
        "installer must be staged inside HomeServer data"
    );
    ensure!(
        rollback_dir.starts_with(&data_dir),
        "rollback directory must be inside HomeServer data"
    );
    ensure!(
        archived_installer.starts_with(&data_dir),
        "installer archive must be inside HomeServer data"
    );
    ensure!(
        result_path.starts_with(&data_dir),
        "update result must be inside HomeServer data"
    );
    ensure!(
        !install_dir.starts_with(&data_dir),
        "install and data directories must be separate"
    );
    ensure!(
        install_dir.is_dir(),
        "HomeServer install directory is unavailable"
    );

    let canonical_data = data_dir
        .canonicalize()
        .context("HomeServer data directory is unavailable")?;
    let canonical_updates = canonical_data
        .join("updates")
        .canonicalize()
        .context("HomeServer update directory is unavailable")?;
    let canonical_staging = canonical_updates
        .join("staging")
        .canonicalize()
        .context("HomeServer update staging directory is unavailable")?;
    let canonical_rollback = canonical_updates
        .join("rollback")
        .canonicalize()
        .context("HomeServer update rollback directory is unavailable")?;
    let canonical_installed = canonical_updates
        .join("installed")
        .canonicalize()
        .context("HomeServer installed-update archive is unavailable")?;
    let canonical_installer = installer_path
        .canonicalize()
        .context("staged installer is unavailable")?;
    let canonical_install = install_dir
        .canonicalize()
        .context("HomeServer install directory is unavailable")?;
    let rollback_parent = rollback_dir
        .parent()
        .context("rollback directory parent is unavailable")?
        .canonicalize()
        .context("HomeServer rollback root is unavailable")?;
    let archive_parent = archived_installer
        .parent()
        .context("installer archive parent is unavailable")?
        .canonicalize()
        .context("HomeServer installed-update archive is unavailable")?;
    let result_parent = result_path
        .parent()
        .context("update result parent is unavailable")?
        .canonicalize()
        .context("HomeServer update result directory is unavailable")?;

    ensure!(
        canonical_installer.starts_with(&canonical_staging),
        "staged installer resolves outside managed update staging"
    );
    ensure!(
        rollback_parent == canonical_rollback
            && rollback_dir.file_name().and_then(|value| value.to_str())
                == Some(plan.update_id.as_str()),
        "rollback directory is outside managed update rollback storage"
    );
    ensure!(
        archive_parent == canonical_installed,
        "installer archive is outside managed installed-update storage"
    );
    ensure!(
        result_parent == canonical_updates
            && result_path.file_name().and_then(|value| value.to_str())
                == Some("last-update-result.json"),
        "update result path is outside the managed update contract"
    );
    ensure!(
        !canonical_install.starts_with(&canonical_data),
        "install and data directories resolve to overlapping storage"
    );
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
    snapshot_directory(&install_dir, &rollback_dir)?;
    stop_service(&plan.service_name)?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let install_result = match Command::new(&installer_path).arg("/S").status() {
        Ok(result) => result,
        Err(error) => {
            let restarted = start_service(&plan.service_name).is_ok()
                && wait_for_health(
                    &plan.health_url,
                    &plan.current_version,
                    Duration::from_secs(90),
                )
                .await;
            ensure!(
                restarted,
                "the staged installer could not start and the previous HomeServer service could not be recovered"
            );
            return Err(error).context("unable to start the staged HomeServer installer");
        }
    };
    let installation_healthy = install_result.success()
        && wait_for_health(
            &plan.health_url,
            &plan.target_version,
            Duration::from_secs(120),
        )
        .await;

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
            message: format!(
                "Microgifter HomeServer {} installed and passed health verification.",
                plan.target_version
            ),
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
    remove_directory_with_retry(install_dir, Duration::from_secs(30))?;
    copy_directory(rollback_dir, install_dir)?;
    ensure_service_registration(&plan.service_name, install_dir)?;
    start_service(&plan.service_name)?;
    ensure!(
        wait_for_health(
            &plan.health_url,
            &plan.current_version,
            Duration::from_secs(90)
        )
        .await,
        "binary rollback completed but the previous HomeServer service did not become healthy"
    );
    Ok(())
}

fn verify_file(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<()> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("unable to inspect staged installer at {}", path.display()))?;
    ensure!(metadata.is_file(), "staged installer is not a regular file");
    ensure!(
        metadata.len() == expected_size,
        "staged installer size does not match the signed manifest"
    );

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
    let script = r#"$exists = Test-Path -LiteralPath $env:MG_UPDATE_FILE
$signature = Get-AuthenticodeSignature -LiteralPath $env:MG_UPDATE_FILE
$status = [string]$signature.Status
$message = [string]$signature.StatusMessage
$thumbprint = if ($signature.SignerCertificate) { [string]$signature.SignerCertificate.Thumbprint } else { '' }
if ($status -ne 'Valid' -or -not $signature.SignerCertificate) {
    [Console]::Error.Write("path=$env:MG_UPDATE_FILE; exists=$exists; status=$status; message=$message; thumbprint=$thumbprint")
    exit 20
}
[Console]::Out.Write($thumbprint)"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("MG_UPDATE_FILE", path)
        .output()
        .context("unable to execute Windows Authenticode verification")?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    ensure!(
        output.status.success(),
        "staged installer does not have a valid trusted Authenticode signature: {}",
        if stderr.is_empty() {
            "Windows PowerShell returned no signature details"
        } else {
            &stderr
        }
    );
    let actual = String::from_utf8(output.stdout)?.trim().replace(' ', "");
    ensure!(
        actual.eq_ignore_ascii_case(expected_thumbprint),
        "staged installer Authenticode signer does not match the signed manifest: expected {}, received {}",
        expected_thumbprint,
        actual
    );
    Ok(())
}

#[cfg(not(windows))]
fn verify_authenticode(_path: &Path, _expected_thumbprint: &str) -> Result<()> {
    bail!("Authenticode verification is only supported on Windows")
}

fn snapshot_directory(source: &Path, destination: &Path) -> Result<()> {
    let temporary = destination.with_extension("snapshot-tmp");
    let previous = destination.with_extension("snapshot-previous");
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    if previous.exists() {
        fs::remove_dir_all(&previous)?;
    }
    if let Err(error) = copy_directory(source, &temporary) {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    if destination.exists() {
        fs::rename(destination, &previous)?;
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        if previous.exists() {
            let _ = fs::rename(&previous, destination);
        }
        let _ = fs::remove_dir_all(&temporary);
        return Err(error.into());
    }
    if previous.exists() {
        fs::remove_dir_all(previous)?;
    }
    Ok(())
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
        ensure!(
            !metadata.file_type().is_symlink(),
            "symbolic links are not allowed in update rollback trees"
        );
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            copy_directory_inner(&entry.path(), &target, file_count, byte_count)?;
        } else if metadata.is_file() {
            *file_count = file_count
                .checked_add(1)
                .context("rollback file count overflow")?;
            *byte_count = byte_count
                .checked_add(metadata.len())
                .context("rollback byte count overflow")?;
            ensure!(
                *file_count <= MAX_ROLLBACK_FILES,
                "rollback snapshot contains too many files"
            );
            ensure!(
                *byte_count <= MAX_ROLLBACK_BYTES,
                "rollback snapshot exceeds the size limit"
            );
            fs::copy(entry.path(), target)?;
        } else {
            bail!("unsupported file type in HomeServer installation directory");
        }
    }
    Ok(())
}

fn atomic_copy(source: &Path, destination: &Path) -> Result<()> {
    let temporary = destination.with_extension("tmp");
    let previous = destination.with_extension("replace-backup");
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    if previous.exists() {
        fs::remove_file(&previous)?;
    }
    fs::copy(source, &temporary)?;
    fs::File::options()
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    if destination.exists() {
        fs::rename(destination, &previous)?;
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        if previous.exists() {
            let _ = fs::rename(&previous, destination);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if previous.exists() {
        fs::remove_file(previous)?;
    }
    Ok(())
}

fn stop_service(service_name: &str) -> Result<()> {
    match service_state(service_name)? {
        None => return Ok(()),
        Some(state) if state == "STOPPED" => return Ok(()),
        Some(_) => {}
    }

    let output = Command::new("sc.exe")
        .args(["stop", service_name])
        .output()
        .context("unable to request HomeServer service stop")?;
    if !output.status.success() {
        match service_state(service_name)? {
            None => return Ok(()),
            Some(state) if state == "STOPPED" => return Ok(()),
            Some(_) => {
                bail!(
                    "unable to stop HomeServer service: {}",
                    command_output(&output)
                );
            }
        }
    }
    wait_for_service_stopped_or_absent(service_name, Duration::from_secs(45))
}

fn ensure_service_registration(service_name: &str, install_dir: &Path) -> Result<()> {
    let service_binary = install_dir
        .join("resources")
        .join("microgifter-homeserver-service.exe");
    ensure!(
        service_binary.is_file(),
        "restored HomeServer service binary is unavailable"
    );
    let binary_command = format!("\"{}\" service", service_binary.display());
    if service_state(service_name)?.is_none() {
        let create_arguments = vec![
            "create".to_owned(),
            service_name.to_owned(),
            "binPath=".to_owned(),
            binary_command.clone(),
            "start=".to_owned(),
            "auto".to_owned(),
            "DisplayName=".to_owned(),
            "Microgifter HomeServer".to_owned(),
        ];
        let started = std::time::Instant::now();
        loop {
            match run_sc(
                create_arguments.clone(),
                "create restored HomeServer service",
            ) {
                Ok(()) => break,
                Err(error) if started.elapsed() < Duration::from_secs(20) => {
                    let message = error.to_string();
                    if !message.contains("1072") && !message.to_lowercase().contains("deletion") {
                        return Err(error);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(error) => return Err(error),
            }
        }
    }
    run_sc(
        vec![
            "config".to_owned(),
            service_name.to_owned(),
            "start=".to_owned(),
            "delayed-auto".to_owned(),
        ],
        "configure restored HomeServer service startup",
    )?;
    run_sc(
        vec![
            "failure".to_owned(),
            service_name.to_owned(),
            "reset=".to_owned(),
            "86400".to_owned(),
            "actions=".to_owned(),
            "restart/5000/restart/15000/none/0".to_owned(),
        ],
        "restore HomeServer service recovery policy",
    )?;
    run_sc(
        vec![
            "failureflag".to_owned(),
            service_name.to_owned(),
            "1".to_owned(),
        ],
        "restore HomeServer service failure actions",
    )?;
    run_sc(
        vec![
            "sidtype".to_owned(),
            service_name.to_owned(),
            "unrestricted".to_owned(),
        ],
        "restore HomeServer service identity",
    )?;
    Ok(())
}

fn run_sc(arguments: Vec<String>, action: &str) -> Result<()> {
    let output = Command::new("sc.exe")
        .args(&arguments)
        .output()
        .with_context(|| format!("unable to {action}"))?;
    ensure!(
        output.status.success(),
        "unable to {action}: {}",
        command_output(&output)
    );
    Ok(())
}

fn command_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{} {}", stdout.trim(), stderr.trim())
        .trim()
        .to_owned()
}

fn service_state(service_name: &str) -> Result<Option<String>> {
    let output = Command::new("sc.exe")
        .args(["query", service_name])
        .output()
        .context("unable to query HomeServer service")?;
    if !output.status.success() {
        let message = command_output(&output);
        if message.contains("1060") || message.to_lowercase().contains("does not exist") {
            return Ok(None);
        }
        bail!("unable to query HomeServer service: {message}");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for state in [
        "STOPPED",
        "RUNNING",
        "START_PENDING",
        "STOP_PENDING",
        "PAUSED",
    ] {
        if text.contains(state) {
            return Ok(Some(state.to_owned()));
        }
    }
    Ok(Some("UNKNOWN".to_owned()))
}

fn start_service(service_name: &str) -> Result<()> {
    if service_state(service_name)?.as_deref() == Some("RUNNING") {
        return Ok(());
    }

    let started = std::time::Instant::now();
    let mut last_output = "no service start attempt was made".to_owned();
    while started.elapsed() < Duration::from_secs(45) {
        let output = Command::new("sc.exe")
            .args(["start", service_name])
            .output()
            .context("unable to request HomeServer service start")?;
        last_output = command_output(&output);

        if wait_for_service_state(service_name, "RUNNING", Duration::from_secs(8)).is_ok() {
            return Ok(());
        }

        match service_state(service_name)? {
            Some(state) if state == "RUNNING" => return Ok(()),
            None => {
                bail!("unable to start HomeServer service: service registration is unavailable")
            }
            Some(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }

    let final_state = service_state(service_name)?.unwrap_or_else(|| "ABSENT".to_owned());
    bail!(
        "unable to start HomeServer service after 45 seconds (final state: {final_state}; last sc.exe output: {last_output})"
    )
}

fn wait_for_service_state(service_name: &str, state: &str, timeout: Duration) -> Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if service_state(service_name)?.as_deref() == Some(state) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("HomeServer service did not reach {state}")
}

fn wait_for_service_stopped_or_absent(service_name: &str, timeout: Duration) -> Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        match service_state(service_name)? {
            None => return Ok(()),
            Some(state) if state == "STOPPED" => return Ok(()),
            Some(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    bail!("HomeServer service did not stop or leave the service registry")
}

fn remove_directory_with_retry(path: &Path, timeout: Duration) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let started = std::time::Instant::now();
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(500));
                if !path.exists() {
                    return Ok(());
                }
            }
            Err(error) => {
                return Err(error).context("unable to remove failed HomeServer installation");
            }
        }
    }
}

async fn wait_for_health(base_url: &str, expected_version: &str, timeout: Duration) -> bool {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::HeaderName::from_static(microgifter_homeserver_core::LOCAL_CLIENT_HEADER),
        reqwest::header::HeaderValue::from_static(microgifter_homeserver_core::LOCAL_CLIENT_VALUE),
    );
    let client = match reqwest::Client::builder()
        .default_headers(headers)
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
    let backup = path.with_extension("replace-backup");
    fs::write(&temporary, serde_json::to_vec_pretty(result)?)?;
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

fn absolute(path: &Path) -> Result<PathBuf> {
    use std::path::Component;

    ensure!(path.is_absolute(), "update path must be absolute");
    ensure!(
        !path
            .components()
            .any(|component| { matches!(component, Component::CurDir | Component::ParentDir) }),
        "update path cannot contain relative traversal components"
    );
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
    #[cfg(windows)]
    #[test]
    fn update_paths_reject_relative_traversal() {
        assert!(absolute(Path::new("C:\\ProgramData\\Microgifter\\HomeServer")).is_ok());
        assert!(absolute(Path::new("C:\\ProgramData\\Microgifter\\..\\Windows")).is_err());
    }
}
