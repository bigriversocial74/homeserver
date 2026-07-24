use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub const PRODUCT_NAME: &str = "Microgifter HomeServer";
pub const SERVICE_NAME: &str = "MicrogifterHomeServer";
pub const API_HOST: &str = "127.0.0.1";
pub const API_PORT: u16 = 47_831;
pub const UPDATE_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const UPDATE_KEY_ID: &str = "homeserver-release-2026-01";

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
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
}

impl UpdateChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Staged,
    Applying,
    Succeeded,
    Failed,
    RolledBack,
}

impl UpdateState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Checking => "checking",
            Self::Current => "current",
            Self::Available => "available",
            Self::Downloading => "downloading",
            Self::Staged => "staged",
            Self::Applying => "applying",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::RolledBack => "rolled_back",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateInstallerContract {
    pub url: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub authenticode_thumbprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateManifestPayload {
    pub schema_version: u32,
    pub product: String,
    pub channel: UpdateChannel,
    pub version: String,
    pub minimum_version: Option<String>,
    pub published_at_utc: DateTime<Utc>,
    pub release_notes: String,
    pub installer: UpdateInstallerContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedUpdateManifest {
    pub key_id: String,
    pub payload: UpdateManifestPayload,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateRecord {
    pub update_id: String,
    pub version: String,
    pub channel: UpdateChannel,
    pub state: UpdateState,
    pub release_notes: String,
    pub installer_file_name: String,
    pub installer_size_bytes: u64,
    pub installer_sha256: String,
    pub authenticode_thumbprint: String,
    pub checked_at_utc: DateTime<Utc>,
    pub downloaded_at_utc: Option<DateTime<Utc>>,
    pub applied_at_utc: Option<DateTime<Utc>>,
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateStatus {
    pub current_version: String,
    pub channel: UpdateChannel,
    pub state: UpdateState,
    pub manifest_url: String,
    pub update: Option<UpdateRecord>,
    pub apply_pending: bool,
    pub last_checked_at_utc: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateActionResult {
    pub status: UpdateStatus,
    pub message: String,
    pub restart_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyUpdateRequest {
    pub confirmation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateApplicationPlan {
    pub schema_version: u32,
    pub update_id: String,
    pub current_version: String,
    pub target_version: String,
    pub installer_path: String,
    pub installer_size_bytes: u64,
    pub installer_sha256: String,
    pub authenticode_thumbprint: String,
    pub install_dir: String,
    pub data_dir: String,
    pub rollback_dir: String,
    pub archived_installer_path: String,
    pub result_path: String,
    pub service_name: String,
    pub health_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateApplicationResult {
    pub schema_version: u32,
    pub update_id: String,
    pub target_version: String,
    pub state: UpdateState,
    pub message: String,
    pub failure_code: Option<String>,
    pub completed_at_utc: DateTime<Utc>,
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
    pub update: String,
    pub update_version: Option<String>,
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
            update: "idle".to_owned(),
            update_version: None,
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

    #[test]
    fn update_state_contract_is_stable() {
        assert_eq!(UpdateState::Available.as_str(), "available");
        assert_eq!(UpdateState::RolledBack.as_str(), "rolled_back");
        assert_eq!(UpdateChannel::Stable.as_str(), "stable");
    }
}
