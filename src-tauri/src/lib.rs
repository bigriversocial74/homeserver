mod agent;
mod cloud;
mod mcp;
mod model;
mod ollama_install;
mod operational;
mod review_intelligence;
mod vault;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use futures_util::StreamExt;
use microgifter_homeserver_core::{
    api_base_url, ApplyUpdateRequest, BackupActionResult, BackupCatalog, BackupReferenceRequest,
    CreateBackupRequest, HealthSnapshot, UpdateActionResult, UpdateStatus, LOCAL_CLIENT_HEADER,
    LOCAL_CLIENT_VALUE,
};
use rfd::AsyncFileDialog;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
#[cfg(desktop)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use zeroize::Zeroizing;

const MAX_RECOVERY_PACKAGE_BYTES: u64 = 320 * 1024 * 1024;
const MAX_LOCAL_JSON_BYTES: usize = 2 * 1024 * 1024;
const PASSPHRASE_HEADER: &str = "x-mg-recovery-passphrase";


#[cfg(desktop)]
struct DesktopUiState {
    quitting: AtomicBool,
}

#[cfg(desktop)]
fn show_control_center(app: &tauri::AppHandle, route: Option<&str>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(route) = route {
            let script = match route {
                "dashboard" => "window.location.hash = '#dashboard';",
                "agent" => "window.location.hash = '#agent';",
                "system" => "window.location.hash = '#system';",
                _ => "",
            };
            if !script.is_empty() {
                let _ = window.eval(script);
            }
        }
    }
}

#[cfg(desktop)]
fn run_tray_action(app: &tauri::AppHandle, action: &str) {
    match action {
        "open-dashboard" => show_control_center(app, Some("dashboard")),
        "open-agent" => show_control_center(app, Some("agent")),
        "open-status" => show_control_center(app, Some("system")),
        "check-updates" => {
            show_control_center(app, Some("system"));
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.eval(
                    "window.dispatchEvent(new CustomEvent('homeserver-tray-action', { detail: { action: 'check-updates' } }));",
                );
            }
        }
        "quit-control-center" => {
            app.state::<DesktopUiState>()
                .quitting
                .store(true, Ordering::SeqCst);
            app.exit(0);
        }
        _ => {}
    }
}

#[tauri::command]
fn control_center_autostart_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch().is_enabled().map_err(|error| error.to_string())
    }
    #[cfg(not(desktop))]
    {
        let _ = app;
        Ok(false)
    }
}

#[tauri::command]
fn control_center_set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        if enabled {
            manager.enable().map_err(|error| error.to_string())?;
        } else {
            manager.disable().map_err(|error| error.to_string())?;
        }
        manager.is_enabled().map_err(|error| error.to_string())
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, enabled);
        Ok(false)
    }
}

#[derive(Debug, Deserialize)]
struct ApiErrorPayload {
    message: String,
}

fn client() -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::HeaderName::from_static(LOCAL_CLIENT_HEADER),
        reqwest::header::HeaderValue::from_static(LOCAL_CLIENT_VALUE),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .map_err(|error| error.to_string())
}

async fn decode_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_LOCAL_JSON_BYTES as u64)
    {
        return Err("HomeServer response exceeds the local JSON size limit.".to_owned());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        let next_len = bytes
            .len()
            .checked_add(chunk.len())
            .ok_or_else(|| "HomeServer response size overflow.".to_owned())?;
        if next_len > MAX_LOCAL_JSON_BYTES {
            return Err("HomeServer response exceeds the local JSON size limit.".to_owned());
        }
        bytes.extend_from_slice(&chunk);
    }
    if status.is_success() {
        return serde_json::from_slice::<T>(&bytes).map_err(|error| error.to_string());
    }

    let message = serde_json::from_slice::<ApiErrorPayload>(&bytes)
        .map(|payload| payload.message)
        .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).chars().take(500).collect());
    Err(if message.trim().is_empty() {
        format!("HomeServer request failed with HTTP {status}")
    } else {
        message
    })
}

async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let response = client()?
        .get(format!("{}{}", api_base_url(), path))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    decode_json(response).await
}

async fn post_json<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T, String> {
    let response = client()?
        .post(format!("{}{}", api_base_url(), path))
        .json(body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    decode_json(response).await
}

fn validate_passphrase(passphrase: &str) -> Result<(), String> {
    let count = passphrase.chars().count();
    if !(12..=256).contains(&count) {
        return Err("Recovery passphrase must contain between 12 and 256 characters.".to_owned());
    }
    Ok(())
}

fn safe_file_name(value: &str) -> String {
    let name: String = value
        .chars()
        .take(180)
        .map(|character| {
            if character.is_ascii_alphanumeric() || ".-_".contains(character) {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.ends_with(".mghbackup") && name.len() > ".mghbackup".len() {
        name
    } else {
        "Microgifter-HomeServer-Recovery.mghbackup".to_owned()
    }
}

#[tauri::command]
async fn homeserver_status() -> Result<HealthSnapshot, String> {
    get_json("/v1/status").await
}

#[tauri::command]
async fn homeserver_backups() -> Result<BackupCatalog, String> {
    get_json("/v1/backups").await
}

#[tauri::command]
async fn homeserver_create_backup(
    request: CreateBackupRequest,
) -> Result<BackupActionResult, String> {
    post_json("/v1/backups/create", &request).await
}

#[tauri::command]
async fn homeserver_verify_backup(
    request: BackupReferenceRequest,
) -> Result<BackupActionResult, String> {
    post_json("/v1/backups/verify", &request).await
}

#[tauri::command]
async fn homeserver_stage_restore(
    request: BackupReferenceRequest,
) -> Result<BackupActionResult, String> {
    post_json("/v1/backups/stage-restore", &request).await
}

#[tauri::command]
async fn homeserver_updates() -> Result<UpdateStatus, String> {
    get_json("/v1/updates").await
}

#[tauri::command]
async fn homeserver_check_updates() -> Result<UpdateActionResult, String> {
    post_json("/v1/updates/check", &serde_json::json!({})).await
}

#[tauri::command]
async fn homeserver_download_update() -> Result<UpdateActionResult, String> {
    post_json("/v1/updates/download", &serde_json::json!({})).await
}

#[tauri::command]
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
}

#[tauri::command]
async fn homeserver_import_recovery_package(
    passphrase: String,
) -> Result<Option<BackupActionResult>, String> {
    let passphrase = Zeroizing::new(passphrase);
    validate_passphrase(passphrase.as_str())?;
    let Some(source) = AsyncFileDialog::new()
        .add_filter("Microgifter recovery package", &["mghbackup"])
        .pick_file()
        .await
    else {
        return Ok(None);
    };

    let metadata = tokio::fs::metadata(source.path())
        .await
        .map_err(|error| error.to_string())?;
    if metadata.len() <= 12 || metadata.len() > MAX_RECOVERY_PACKAGE_BYTES {
        return Err("Recovery package size is invalid.".to_owned());
    }

    let input = tokio::fs::File::open(source.path())
        .await
        .map_err(|error| error.to_string())?;
    let encoded_passphrase = Zeroizing::new(URL_SAFE_NO_PAD.encode(passphrase.as_bytes()));
    let response = client()?
        .post(format!("{}/v1/backups/import", api_base_url()))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/vnd.microgifter.homeserver-backup",
        )
        .header(reqwest::header::CONTENT_LENGTH, metadata.len())
        .header(PASSPHRASE_HEADER, encoded_passphrase.as_str())
        .body(reqwest::Body::wrap_stream(ReaderStream::new(input)))
        .send()
        .await
        .map_err(|error| error.to_string())?;

    decode_json(response).await.map(Some)
}

#[tauri::command]
async fn homeserver_export_recovery_package(
    backup_id: String,
    suggested_file_name: String,
) -> Result<Option<String>, String> {
    let file_name = safe_file_name(&suggested_file_name);
    let Some(destination) = AsyncFileDialog::new()
        .add_filter("Microgifter recovery package", &["mghbackup"])
        .set_file_name(&file_name)
        .save_file()
        .await
    else {
        return Ok(None);
    };

    let response = client()?
        .get(format!(
            "{}/v1/backups/{}/package",
            api_base_url(),
            backup_id
        ))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return decode_json::<serde_json::Value>(response)
            .await
            .map(|_| None);
    }

    let destination_path = destination.path().to_path_buf();
    let mut output = tokio::fs::File::create(&destination_path)
        .await
        .map_err(|error| error.to_string())?;
    let mut stream = response.bytes_stream();
    let mut total_bytes = 0_u64;
    let transfer_result = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            total_bytes = total_bytes
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| "Recovery export size overflow.".to_owned())?;
            if total_bytes > MAX_RECOVERY_PACKAGE_BYTES {
                return Err("Recovery export exceeds the package size limit.".to_owned());
            }
            output
                .write_all(&chunk)
                .await
                .map_err(|error| error.to_string())?;
        }
        if total_bytes <= 12 {
            return Err("Recovery export was empty or truncated.".to_owned());
        }
        output.sync_all().await.map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = transfer_result {
        drop(output);
        let _ = tokio::fs::remove_file(&destination_path).await;
        return Err(error);
    }

    Ok(Some(destination_path.to_string_lossy().into_owned()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri_plugin_autostart::MacosLauncher;

                app.handle().plugin(tauri_plugin_autostart::init(
                    MacosLauncher::LaunchAgent,
                    Some(vec!["--hidden"]),
                ))?;
                app.manage(DesktopUiState {
                    quitting: AtomicBool::new(false),
                });

                let dashboard = MenuItem::with_id(app, "open-dashboard", "Open Dashboard", true, None::<&str>)?;
                let agent = MenuItem::with_id(app, "open-agent", "Open Agent Chat", true, None::<&str>)?;
                let status = MenuItem::with_id(app, "open-status", "HomeServer Status", true, None::<&str>)?;
                let updates = MenuItem::with_id(app, "check-updates", "Check for Updates", true, None::<&str>)?;
                let separator = PredefinedMenuItem::separator(app)?;
                let quit = MenuItem::with_id(app, "quit-control-center", "Quit Control Center", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&dashboard, &agent, &status, &updates, &separator, &quit])?;
                let tray_icon = app
                    .default_window_icon()
                    .cloned()
                    .ok_or("HomeServer application icon is unavailable")?;

                TrayIconBuilder::with_id("homeserver-control-center")
                    .icon(tray_icon)
                    .tooltip("Microgifter HomeServer")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| run_tray_action(app, event.id.as_ref()))
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_control_center(tray.app_handle(), None);
                        }
                    })
                    .build(app)?;

                if std::env::args().any(|argument| argument == "--hidden") {
                    if let Some(window) = app.get_webview_window("main") {
                        window.hide()?;
                    }
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let WindowEvent::CloseRequested { api, .. } = event {
                let quitting = window
                    .app_handle()
                    .state::<DesktopUiState>()
                    .quitting
                    .load(Ordering::SeqCst);
                if !quitting {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            homeserver_status,
            control_center_autostart_enabled,
            control_center_set_autostart,
            agent::homeserver_agent_workspace,
            agent::homeserver_agent_prompt,
            agent::homeserver_create_agent_goal,
            agent::homeserver_archive_agent_goal,
            agent::homeserver_create_agent_plan,
            agent::homeserver_cancel_agent_plan,
            agent::homeserver_approve_agent_plan,
            agent::homeserver_reject_agent_plan,
            agent::homeserver_execute_agent_plan,
            agent::homeserver_create_world_mission,
            agent::homeserver_cancel_world_mission,
            operational::homeserver_operational_data,
            operational::homeserver_update_operational_dataset_grant,
            operational::homeserver_import_operational_data,
            operational::homeserver_query_operational_data,
            review_intelligence::homeserver_review_intelligence,
            review_intelligence::homeserver_update_review_intelligence_settings,
            review_intelligence::homeserver_sync_review_dataset,
            review_intelligence::homeserver_run_review_analysis,
            review_intelligence::homeserver_record_review_recommendation_outcome,
            cloud::homeserver_cloud_status,
            cloud::homeserver_pair_cloud,
            cloud::homeserver_disconnect_cloud,
            cloud::homeserver_cloud_vault_self_test,
            cloud::homeserver_enqueue_cloud_sync,
            cloud::homeserver_sync_cloud,
            cloud::homeserver_cloud_connections,
            cloud::homeserver_pair_cloud_connection,
            cloud::homeserver_disconnect_cloud_connection,
            cloud::homeserver_enqueue_connection_sync,
            cloud::homeserver_sync_cloud_connection,
            cloud::homeserver_sync_all_cloud_connections,
            cloud::homeserver_microgifter_status,
            cloud::homeserver_connect_microgifter,
            cloud::homeserver_refresh_microgifter_entitlement,
            cloud::homeserver_send_microgifter_heartbeat,
            cloud::homeserver_rotate_microgifter_credentials,
            cloud::homeserver_microgifter_update_preferences,
            cloud::homeserver_save_microgifter_update_preferences,
            cloud::homeserver_authorize_microgifter_update,
            cloud::homeserver_start_microgifter_device_replacement,
            cloud::homeserver_complete_microgifter_device_replacement,
            cloud::homeserver_pod_status,
            cloud::homeserver_connect_pod,
            cloud::homeserver_update_pod_runtime,
            cloud::homeserver_poll_pod,
            cloud::homeserver_disconnect_pod,
            homeserver_backups,
            homeserver_create_backup,
            homeserver_verify_backup,
            homeserver_stage_restore,
            homeserver_updates,
            homeserver_check_updates,
            homeserver_download_update,
            homeserver_apply_update,
            homeserver_import_recovery_package,
            homeserver_export_recovery_package,
            vault::homeserver_vault,
            vault::homeserver_import_vault_document,
            vault::homeserver_search_vault,
            vault::homeserver_semantic_vault,
            vault::homeserver_rebuild_semantic_vault,
            vault::homeserver_search_semantic_vault,
            vault::homeserver_reindex_vault,
            vault::homeserver_delete_vault_document,
            model::homeserver_models,
            model::homeserver_pull_model,
            model::homeserver_delete_model,
            model::homeserver_unload_model,
            model::homeserver_test_model,
            model::homeserver_update_model_settings,
            mcp::homeserver_mcp,
            mcp::homeserver_create_mcp_client,
            mcp::homeserver_revoke_mcp_client,
            mcp::homeserver_mcp_bridge_path,
            ollama_install::homeserver_open_ollama_official,
            ollama_install::homeserver_open_ollama_terminal,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Microgifter HomeServer Control Center");
}

