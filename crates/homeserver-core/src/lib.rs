use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PRODUCT_NAME: &str = "Microgifter HomeServer";
pub const SERVICE_NAME: &str = "MicrogifterHomeServer";
pub const API_HOST: &str = "127.0.0.1";
pub const API_PORT: u16 = 47_831;

pub fn api_base_url() -> String {
    format!("http://{API_HOST}:{API_PORT}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Starting,
    Running,
    Offline,
    NeedsAttention,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloudConnectionState {
    NotPaired,
    Pairing,
    Connected,
    Degraded,
    Revoked,
}

impl CloudConnectionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotPaired => "not_paired",
            Self::Pairing => "pairing",
            Self::Connected => "connected",
            Self::Degraded => "degraded",
            Self::Revoked => "revoked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudConnectionSnapshot {
    pub state: CloudConnectionState,
    pub cloud_base_url: Option<String>,
    pub device_id: Option<String>,
    pub scopes: Vec<String>,
    pub paired_at_utc: Option<String>,
    pub last_success_utc: Option<String>,
    pub last_error: Option<String>,
}

impl Default for CloudConnectionSnapshot {
    fn default() -> Self {
        Self {
            state: CloudConnectionState::NotPaired,
            cloud_base_url: None,
            device_id: None,
            scopes: Vec::new(),
            paired_at_utc: None,
            last_success_utc: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairCloudRequest {
    pub cloud_base_url: String,
    pub pairing_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnqueueSyncRequest {
    pub operation_type: String,
    #[serde(default)]
    pub payload: Value,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncRunSnapshot {
    pub processed: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub review: u64,
    pub pending: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub version: String,
    pub server_name: String,
    pub state: ServiceState,
    pub api_available: bool,
    pub api_url: String,
    pub database: String,
    pub cloud: String,
    pub pending_sync: u64,
    pub last_backup: Option<String>,
    pub model: Option<String>,
    pub last_updated_utc: DateTime<Utc>,
}

impl HealthSnapshot {
    pub fn running(server_name: impl Into<String>, database: impl Into<String>) -> Self {
        Self::new(server_name, ServiceState::Running, true, database)
    }

    pub fn needs_attention(server_name: impl Into<String>, database: impl Into<String>) -> Self {
        Self::new(server_name, ServiceState::NeedsAttention, true, database)
    }

    pub fn offline(reason: impl Into<String>) -> Self {
        Self::new(PRODUCT_NAME, ServiceState::Offline, false, reason)
    }

    pub fn with_cloud(mut self, cloud: &CloudConnectionSnapshot) -> Self {
        self.cloud = cloud.state.as_str().to_owned();
        self
    }

    fn new(
        server_name: impl Into<String>,
        state: ServiceState,
        api_available: bool,
        database: impl Into<String>,
    ) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            server_name: server_name.into(),
            state,
            api_available,
            api_url: api_base_url(),
            database: database.into(),
            cloud: "not_paired".to_owned(),
            pending_sync: 0,
            last_backup: None,
            model: None,
            last_updated_utc: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_url_is_loopback_only() {
        assert_eq!(api_base_url(), "http://127.0.0.1:47831");
    }

    #[test]
    fn offline_snapshot_never_reports_api_available() {
        let snapshot = HealthSnapshot::offline("unavailable");
        assert_eq!(snapshot.state, ServiceState::Offline);
        assert!(!snapshot.api_available);
    }

    #[test]
    fn needs_attention_keeps_api_available_for_diagnostics() {
        let snapshot = HealthSnapshot::needs_attention(PRODUCT_NAME, "integrity_check_failed");
        assert_eq!(snapshot.state, ServiceState::NeedsAttention);
        assert!(snapshot.api_available);
        assert_eq!(snapshot.database, "integrity_check_failed");
    }

    #[test]
    fn cloud_state_updates_health_snapshot() {
        let cloud = CloudConnectionSnapshot {
            state: CloudConnectionState::Connected,
            ..CloudConnectionSnapshot::default()
        };
        assert_eq!(
            HealthSnapshot::running(PRODUCT_NAME, "ready")
                .with_cloud(&cloud)
                .cloud,
            "connected"
        );
    }
}
