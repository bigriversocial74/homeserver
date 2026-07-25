from pathlib import Path

updater = Path("crates/homeserver-updater/src/main.rs")
text = updater.read_text(encoding="utf-8")

old = '''    stop_service(&plan.service_name)?;
    if install_dir.exists() {
        fs::remove_dir_all(install_dir)
            .context("unable to remove failed HomeServer installation")?;
    }
    copy_directory(rollback_dir, install_dir)?;'''
new = '''    stop_service(&plan.service_name)?;
    remove_directory_with_retry(install_dir, Duration::from_secs(30))?;
    copy_directory(rollback_dir, install_dir)?;'''
if old not in text:
    raise SystemExit("rollback removal anchor not found")
text = text.replace(old, new, 1)

old = '''fn stop_service(service_name: &str) -> Result<()> {
    let _ = Command::new("sc.exe").args(["stop", service_name]).status();
    wait_for_service_state(service_name, "STOPPED", Duration::from_secs(45))
}
'''
new = '''fn stop_service(service_name: &str) -> Result<()> {
    match service_state(service_name)? {
        None | Some(ref state) if state == "STOPPED" => return Ok(()),
        Some(_) => {}
    }

    let output = Command::new("sc.exe")
        .args(["stop", service_name])
        .output()
        .context("unable to request HomeServer service stop")?;
    if !output.status.success() && service_state(service_name)?.is_some() {
        bail!(
            "unable to stop HomeServer service: {}",
            command_output(&output)
        );
    }
    wait_for_service_stopped_or_absent(service_name, Duration::from_secs(45))
}
'''
if old not in text:
    raise SystemExit("stop_service anchor not found")
text = text.replace(old, new, 1)

old = '''    let exists = Command::new("sc.exe")
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
'''
new = '''    if service_state(service_name)?.is_none() {
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
            match run_sc(create_arguments.clone(), "create restored HomeServer service") {
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
'''
if old not in text:
    raise SystemExit("service registration anchor not found")
text = text.replace(old, new, 1)

old = '''fn run_sc(arguments: Vec<String>, action: &str) -> Result<()> {
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
'''
new = '''fn run_sc(arguments: Vec<String>, action: &str) -> Result<()> {
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
    format!("{} {}", stdout.trim(), stderr.trim()).trim().to_owned()
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
    for state in ["STOPPED", "RUNNING", "START_PENDING", "STOP_PENDING", "PAUSED"] {
        if text.contains(state) {
            return Ok(Some(state.to_owned()));
        }
    }
    Ok(Some("UNKNOWN".to_owned()))
}
'''
if old not in text:
    raise SystemExit("run_sc anchor not found")
text = text.replace(old, new, 1)

old = '''fn wait_for_service_state(service_name: &str, state: &str, timeout: Duration) -> Result<()> {
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
'''
new = '''fn wait_for_service_state(service_name: &str, state: &str, timeout: Duration) -> Result<()> {
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
            Err(error) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(500));
                if !path.exists() {
                    return Ok(());
                }
                let _ = error;
            }
            Err(error) => {
                return Err(error).context("unable to remove failed HomeServer installation")
            }
        }
    }
}
'''
if old not in text:
    raise SystemExit("wait_for_service_state anchor not found")
text = text.replace(old, new, 1)
updater.write_text(text, encoding="utf-8")

smoke = Path("scripts/smoke-test-updater.ps1")
text = smoke.read_text(encoding="utf-8")
old = '$uninstallerPath = $null\n'
new = '$uninstallerPath = $null\n$diagnosticPath = Join-Path (Get-Location) "updater-smoke-diagnostics.log"\nRemove-Item $diagnosticPath -Force -ErrorAction SilentlyContinue\n'
if old not in text:
    raise SystemExit("diagnostic path anchor not found")
text = text.replace(old, new, 1)

old = '''    $process = Start-Process -FilePath $updaterCopy -ArgumentList @("apply", $Plan.PlanPath) -PassThru -Wait
    if ($process.ExitCode -ne 0) {
        throw "HomeServer updater helper failed with exit code $($process.ExitCode)"
    }
    if (-not (Test-Path $resultPath)) {
        throw "HomeServer updater did not write an application result"
    }
    Get-Content $resultPath -Raw | ConvertFrom-Json
'''
new = '''    $process = Start-Process -FilePath $updaterCopy -ArgumentList @("apply", $Plan.PlanPath) -PassThru -Wait
    $rawResult = if (Test-Path $resultPath) { Get-Content $resultPath -Raw } else { $null }
    Add-Content -LiteralPath $diagnosticPath -Value ("Updater exit code: {0}`nResult: {1}`n" -f $process.ExitCode, $rawResult)
    if ($process.ExitCode -ne 0) {
        throw "HomeServer updater helper failed with exit code $($process.ExitCode). Result: $rawResult"
    }
    if (-not $rawResult) {
        throw "HomeServer updater did not write an application result"
    }
    $rawResult | ConvertFrom-Json
'''
if old not in text:
    raise SystemExit("Invoke-UpdatePlan anchor not found")
text = text.replace(old, new, 1)

old = '''    Write-Host "HomeServer signed update, Authenticode verification, health confirmation, automatic rollback, installer preservation, and cleanup smoke tests passed."
}
finally {
'''
new = '''    Write-Host "HomeServer signed update, Authenticode verification, health confirmation, automatic rollback, installer preservation, and cleanup smoke tests passed."
}
catch {
    Add-Content -LiteralPath $diagnosticPath -Value ("Failure: {0}`nScript stack: {1}`n" -f $_.Exception.Message, $_.ScriptStackTrace)
    if (Test-Path $resultPath) {
        Add-Content -LiteralPath $diagnosticPath -Value ("Last result: {0}`n" -f (Get-Content $resultPath -Raw))
    }
    Add-Content -LiteralPath $diagnosticPath -Value ((& "$env:SystemRoot\\System32\\sc.exe" query $serviceName 2>&1 | Out-String))
    throw
}
finally {
'''
if old not in text:
    raise SystemExit("smoke catch anchor not found")
text = text.replace(old, new, 1)
smoke.write_text(text, encoding="utf-8")

workflow = Path(".github/workflows/phase-1-foundation.yml")
text = workflow.read_text(encoding="utf-8")
old = '''          name: updater-smoke-log
          path: updater-smoke.log
          if-no-files-found: warn'''
new = '''          name: updater-smoke-log
          path: |
            updater-smoke.log
            updater-smoke-diagnostics.log
          if-no-files-found: warn'''
if old not in text:
    raise SystemExit("updater artifact anchor not found")
workflow.write_text(text.replace(old, new, 1), encoding="utf-8")
