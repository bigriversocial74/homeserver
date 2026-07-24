use microgifter_homeserver_core::{api_base_url, HealthSnapshot};
use std::time::Duration;

#[tauri::command]
async fn homeserver_status() -> Result<HealthSnapshot, String> {
    let endpoint = format!("{}/v1/status", api_base_url());
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|error| error.to_string())?;

    client
        .get(endpoint)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<HealthSnapshot>()
        .await
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![homeserver_status])
        .run(tauri::generate_context!())
        .expect("error while running Microgifter HomeServer Control Center");
}
