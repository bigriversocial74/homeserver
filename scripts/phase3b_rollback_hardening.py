from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        raise SystemExit(f"{label} anchor not found")
    return text.replace(old, new, 1)


updater = Path("crates/homeserver-updater/src/main.rs")
text = updater.read_text(encoding="utf-8")

text = replace_once(
    text,
    '''        Err(error) => UpdateApplicationResult {
            schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
            update_id: plan.update_id.clone(),
            target_version: plan.target_version.clone(),
            state: UpdateState::Failed,
            message: "HomeServer update failed before a verified installation was available."
                .to_owned(),
            failure_code: Some(public_failure_code(&error)),
            completed_at_utc: Utc::now(),
        },''',
    '''        Err(error) => {
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
        },''',
    "updater error diagnostic",
)

text = replace_once(
    text,
    '''    stop_service(&plan.service_name)?;
    if install_dir.exists() {
        fs::remove_dir_all(install_dir)
            .context("unable to remove failed HomeServer installation")?;
    }
    copy_directory(rollback_dir, install_dir)?;''',
    '''    stop_service(&plan.service_name)?;
    remove_directory_with_retry(install_dir, Duration::from_secs(30))?;
    copy_directory(rollback_dir, install_dir)?;''',
    "rollback removal",
)

text = replace_once(
    text,
    '''fn stop_service(service_name: &str) -> Result<()> {
    let _ = Command::new("sc.exe").args(["stop", service_name]).status();
    wait_for_service_state(service_name, "STOPPED", Duration::from_secs(45))
}
''',
    '''fn stop_service(service_name: &str) -> Result<()> {
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
''',
    "stop service",
)

text = replace_once(
    text,
    '''    let exists = Command::new("sc.exe")
        .args(["query", service_name])
        .status()
        .context("unable to query HomeServer service registration")?
        .success();
    if !exists {
        run_sc(
            vec![
                "create".to_owned(),
                service_name.to_owned(),
                "binPath=".to_owned(),
                binary_command.clone(),
                "start=".to_owned(),
                "auto".to_owned(),
                "DisplayName=".to_owned(),
                "Microgifter HomeServer".to_owned(),
            ],
            "create restored HomeServer service",
        )?;
    }
''',
    '''    if service_state(service_name)?.is_none() {
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
                    if !message.contains("1072")
                        && !message.to_lowercase().contains("deletion")
                    {
                        return Err(error);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(error) => return Err(error),
            }
        }
    }
''',
    "service registration",
)

text = replace_once(
    text,
    '''fn run_sc(arguments: Vec<String>, action: &str) -> Result<()> {
    let output = Command::new("sc.exe")
        .args(&arguments)
        .output()
        .with_context(|| format!("unable to {action}"))?;
    ensure!(
        output.status.success(),
        "unable to {action}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}
''',
    '''fn run_sc(arguments: Vec<String>, action: &str) -> Result<()> {
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
''',
    "service command diagnostics",
)

text = replace_once(
    text,
    '''fn wait_for_service_state(service_name: &str, state: &str, timeout: Duration) -> Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        let output = Command::new("sc.exe")
            .args(["query", service_name])
            .output()?;
        let text = String::from_utf8_lossy(&output.stdout);
        if text.contains(state) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("HomeServer service did not reach {state}")
}
''',
    '''fn wait_for_service_state(service_name: &str, state: &str, timeout: Duration) -> Result<()> {
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
''',
    "service wait and rollback removal",
)

updater.write_text(text, encoding="utf-8")
