use microgifter_homeserver_core::{
    api_base_url, BackupActionResult, BackupCatalog, BackupReferenceRequest, CreateBackupRequest,
    HealthSnapshot,
};
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| error.to_string())
}

async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    client()?
        .get(format!("{}{}", api_base_url(), path))
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<T>()
        .await
        .map_err(|error| error.to_string())
}

async fn post_json<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> Result<T, String> {
    client()?
        .post(format!("{}{}", api_base_url(), path))
        .json(body)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<T>()
        .await
        .map_err(|error| error.to_string())
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            homeserver_status,
            homeserver_backups,
            homeserver_create_backup,
            homeserver_verify_backup,
            homeserver_stage_restore
        ])
        .run(tauri::generate_context!())
        .expect("error while running Microgifter HomeServer Control Center");
}
