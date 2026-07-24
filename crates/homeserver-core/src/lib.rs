use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

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
pub enum BackupKind {
    Automatic,
    Manual,
    Recovery,
    PreUpdate,
}

impl BackupKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Manual => "manual",
            Self::Recovery => "recovery",
            Self::PreUpdate => "pre_update",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackupState {
    Creating,
    Ready,
    Verified,
    RestoreStaged,
    Restored,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupRecord {
    pub backup_id: String,
    pub kind: BackupKind,
    pub state: BackupState,
    pub encryption: String,
    pub file_name: String,
    pub storage_path: String,
    pub size_bytes: u64,
    pub archive_sha256: Option<String>,
    pub database_sha256: Option<String>,
    pub note: Option<String>,
    pub created_at_utc: DateTime<Utc>,
    pub verified_at_utc: Option<DateTime<Utc>>,
    pub restored_at_utc: Option<DateTime<Utc>>,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupCatalog {
    pub backups: Vec<BackupRecord>,
    pub retention_count: u32,
    pub interval_hours: u32,
    pub last_automatic_backup_utc: Option<DateTime<Utc>>,
    pub restore_pending: bool,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateBackupRequest {
    pub kind: BackupKind,
    pub passphrase: Option<String>,
    pub note: Option<String>,
}

impl Drop for CreateBackupRequest {
    fn drop(&mut self) {
        if let Some(passphrase) = &mut self.passphrase {
            passphrase.zeroize();
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupReferenceRequest {
    pub backup_id: String,
    pub passphrase: Option<String>,
    pub confirmation: Option<String>,
}

impl Drop for BackupReferenceRequest {
    fn drop(&mut self) {
        if let Some(passphrase) = &mut self.passphrase {
            passphrase.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupActionResult {
    pub backup: BackupRecord,
    pub message: String,
    pub restart_required: bool,
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
    pub backup: String,
    pub last_backup: Option<String>,
    pub restore_pending: bool,
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
            backup: "ready".to_owned(),
            last_backup: None,
            restore_pending: false,
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
    fn backup_kind_contract_is_stable() {
        assert_eq!(BackupKind::Automatic.as_str(), "automatic");
        assert_eq!(BackupKind::Recovery.as_str(), "recovery");
        assert_eq!(BackupKind::PreUpdate.as_str(), "pre_update");
    }
}
