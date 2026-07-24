use microgifter_homeserver_core::{
    api_base_url, CloudConnectionSnapshot, EnqueueSyncRequest, HealthSnapshot, PairCloudRequest,
    SyncRunSnapshot,
};
use reqwest::Method;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::time::Duration;

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|error| error.to_string())
}

async fn local_request<T, B>(method: Method, path: &str, body: Option<&B>) -> Result<T, String>
where
    T: DeserializeOwned,
    B: Serialize + ?Sized,
{
    let mut request = client()?
        .request(method, format!("{}{path}", api_base_url()))
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.map_err(|error| error.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| {
                format!("HomeServer request failed with status {}", status.as_u16())
            });
        return Err(message);
    }
    serde_json::from_str(&text)
        .map_err(|error| format!("HomeServer returned invalid data: {error}"))
}

#[tauri::command]
async fn homeserver_status() -> Result<HealthSnapshot, String> {
    local_request::<HealthSnapshot, Value>(Method::GET, "/v1/status", None).await
}

#[tauri::command]
async fn homeserver_connection() -> Result<CloudConnectionSnapshot, String> {
    local_request::<CloudConnectionSnapshot, Value>(Method::GET, "/v1/connection", None).await
}

#[tauri::command]
async fn homeserver_pair(request: PairCloudRequest) -> Result<CloudConnectionSnapshot, String> {
    local_request(Method::POST, "/v1/connection/pair", Some(&request)).await
}

#[tauri::command]
async fn homeserver_disconnect() -> Result<CloudConnectionSnapshot, String> {
    local_request::<CloudConnectionSnapshot, Value>(Method::DELETE, "/v1/connection", None).await
}

#[tauri::command]
async fn homeserver_enqueue_sync(request: EnqueueSyncRequest) -> Result<Value, String> {
    local_request(Method::POST, "/v1/sync/enqueue", Some(&request)).await
}

#[tauri::command]
async fn homeserver_sync_now() -> Result<SyncRunSnapshot, String> {
    local_request::<SyncRunSnapshot, Value>(Method::POST, "/v1/sync/run", None).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            homeserver_status,
            homeserver_connection,
            homeserver_pair,
            homeserver_disconnect,
            homeserver_enqueue_sync,
            homeserver_sync_now
        ])
        .run(tauri::generate_context!())
        .expect("error while running Microgifter HomeServer Control Center");
}
