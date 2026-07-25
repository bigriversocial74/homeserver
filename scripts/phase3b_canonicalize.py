from pathlib import Path


def patch_backup_fixture() -> None:
    path = Path("crates/homeserver-service/src/backup.rs")
    text = path.read_text(encoding="utf-8")
    old = '''            imports_dir: data_dir.join("imports"),
            restore_dir: data_dir.join("restore"),
            staging_dir: data_dir.join("staging"),
            data_dir,
            server_name: "Test HomeServer".to_owned(),'''
    new = '''            imports_dir: data_dir.join("imports"),
            restore_dir: data_dir.join("restore"),
            staging_dir: data_dir.join("staging"),
            updates_dir: data_dir.join("updates"),
            update_staging_dir: data_dir.join("updates/staging"),
            update_rollback_dir: data_dir.join("updates/rollback"),
            update_installed_dir: data_dir.join("updates/installed"),
            update_manifest_url: "https://updates.microgifter.com/homeserver/stable/manifest.json".to_owned(),
            data_dir,
            server_name: "Test HomeServer".to_owned(),'''
    if old in text:
        text = text.replace(old, new, 1)
    old_dirs = '''            &config.restore_dir,
            &config.staging_dir,
        ] {'''
    new_dirs = '''            &config.restore_dir,
            &config.staging_dir,
            &config.updates_dir,
            &config.update_staging_dir,
            &config.update_rollback_dir,
            &config.update_installed_dir,
        ] {'''
    if old_dirs in text:
        text = text.replace(old_dirs, new_dirs, 1)
    if "update_manifest_url" not in text[text.index("fn config(directory"):]:
        raise RuntimeError("backup AppConfig fixture was not updated")
    path.write_text(text, encoding="utf-8")


def patch_update_result_binding() -> None:
    path = Path("crates/homeserver-service/src/update_store.rs")
    text = path.read_text(encoding="utf-8")
    text = text.replace(
        "use chrono::{DateTime, Utc};", "use chrono::{DateTime, Duration, Utc};"
    )
    start = text.index("pub fn record_application_result(")
    end = text.index("\npub fn latest_update", start)
    replacement = '''pub fn record_application_result(
    connection: &Connection,
    result: &UpdateApplicationResult,
) -> Result<()> {
    ensure!(
        matches!(
            result.state,
            UpdateState::Succeeded | UpdateState::RolledBack | UpdateState::Failed
        ),
        "update application result has an unsupported state"
    );
    ensure!(
        result.completed_at_utc <= Utc::now() + Duration::minutes(10),
        "update application result completion time is in the future"
    );
    let stored = update_by_id(connection, &result.update_id)?;
    ensure!(
        stored.record.state == UpdateState::Applying,
        "update application result does not belong to an applying release"
    );
    ensure!(
        stored.record.version == result.target_version,
        "update application result target version does not match the applying release"
    );
    let state = result.state.as_str();
    let changed = connection.execute(
        "UPDATE update_records SET state=?1,applied_at_utc=CASE WHEN ?1='succeeded' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE applied_at_utc END,failure_code=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE update_id=?3 AND state='applying'",
        params![state, result.failure_code.as_deref(), &result.update_id],
    )?;
    ensure!(changed == 1, "applying update state changed before its result was recorded");
    set_runtime(
        connection,
        result.state.clone(),
        result.failure_code.as_deref(),
        false,
    )?;
    record_event(
        connection,
        Some(&result.update_id),
        match &result.state {
            UpdateState::Succeeded => "update.succeeded",
            UpdateState::RolledBack => "update.rolled_back",
            _ => "update.failed",
        },
        &result.message,
        &serde_json::json!({
            "target_version": &result.target_version,
            "failure_code": result.failure_code.as_deref(),
        })
        .to_string(),
    )
}
'''
    path.write_text(text[:start] + replacement + text[end:], encoding="utf-8")


def patch_tauri_exit() -> None:
    path = Path("src-tauri/src/lib.rs")
    text = path.read_text(encoding="utf-8")
    old = '''#[tauri::command]
async fn homeserver_apply_update(
    request: ApplyUpdateRequest,
) -> Result<UpdateActionResult, String> {
    post_json("/v1/updates/apply", &request).await
}'''
    new = '''#[tauri::command]
async fn homeserver_apply_update(
    app: tauri::AppHandle,
    request: ApplyUpdateRequest,
) -> Result<UpdateActionResult, String> {
    let result = post_json("/v1/updates/apply", &request).await?;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        app.exit(0);
    });
    Ok(result)
}'''
    if old in text:
        text = text.replace(old, new, 1)
    if "app.exit(0)" not in text:
        raise RuntimeError("Tauri update shutdown was not applied")
    path.write_text(text, encoding="utf-8")


def patch_updater_rollback() -> None:
    path = Path("crates/homeserver-updater/src/main.rs")
    text = path.read_text(encoding="utf-8")
    text = text.replace(
        '''    stop_service(&plan.service_name)?;
    snapshot_directory(&install_dir, &rollback_dir)?;

    let install_result''',
        '''    stop_service(&plan.service_name)?;
    snapshot_directory(&install_dir, &rollback_dir)?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let install_result''',
        1,
    )
    text = text.replace(
        '''    copy_directory(rollback_dir, install_dir)?;
    start_service(&plan.service_name)?;''',
        '''    copy_directory(rollback_dir, install_dir)?;
    ensure_service_registration(&plan.service_name, install_dir)?;
    start_service(&plan.service_name)?;''',
        1,
    )
    anchor = "fn start_service(service_name: &str) -> Result<()> {"
    if "fn ensure_service_registration(" not in text:
        service_registration = '''fn ensure_service_registration(service_name: &str, install_dir: &Path) -> Result<()> {
    let service_binary = install_dir
        .join("resources")
        .join("microgifter-homeserver-service.exe");
    ensure!(service_binary.is_file(), "restored HomeServer service binary is unavailable");
    let binary_command = format!("\\\"{}\\\" service", service_binary.display());
    let exists = Command::new("sc.exe")
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
    run_sc(
        vec![
            "config".to_owned(),
            service_name.to_owned(),
            "binPath=".to_owned(),
            binary_command,
            "start=".to_owned(),
            "delayed-auto".to_owned(),
        ],
        "configure restored HomeServer service",
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
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

'''
        if anchor not in text:
            raise RuntimeError("updater start-service anchor was not found")
        text = text.replace(anchor, service_registration + anchor, 1)
    if "ensure_service_registration(&plan.service_name" not in text:
        raise RuntimeError("updater rollback registration was not applied")
    path.write_text(text, encoding="utf-8")


def patch_cfg_imports() -> None:
    for file_name in (
        "crates/homeserver-service/src/update.rs",
        "crates/homeserver-service/src/update_apply.rs",
    ):
        path = Path(file_name)
        text = path.read_text(encoding="utf-8")
        text = text.replace(
            "use anyhow::{bail, ensure, Context, Result};",
            '#[cfg(not(windows))]\nuse anyhow::bail;\nuse anyhow::{ensure, Context, Result};',
        )
        path.write_text(text, encoding="utf-8")


patch_backup_fixture()
patch_update_result_binding()
patch_tauri_exit()
patch_updater_rollback()
patch_cfg_imports()
