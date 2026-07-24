mod app;
mod backup;
mod backup_key;
mod config;
mod database;
mod http;
mod recovery_transfer;
mod update;
mod update_apply;
mod update_store;

use anyhow::{anyhow, bail, Context, Result};
use config::AppConfig;
use microgifter_homeserver_core::{
    ApplyUpdateRequest, BackupActionResult, BackupCatalog, BackupKind, BackupReferenceRequest,
    BackupState, CreateBackupRequest, HealthSnapshot, UpdateActionResult, UpdateState,
    UpdateStatus, SERVICE_NAME,
};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};
use tokio::sync::watch;
use tracing::{error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

pub struct AppState {
    config: AppConfig,
    connection: Mutex<Connection>,
}

impl AppState {
    fn new(config: AppConfig, connection: Connection) -> Self {
        Self {
            config,
            connection: Mutex::new(connection),
        }
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|error| anyhow!("HomeServer database lock was poisoned: {error}"))
    }

    fn snapshot(&self) -> HealthSnapshot {
        let connection = match self.connection() {
            Ok(connection) => connection,
            Err(error) => {
                error!(?error, "HomeServer database lock was poisoned");
                return HealthSnapshot::needs_attention(
                    &self.config.server_name,
                    "database_lock_failed",
                );
            }
        };

        if let Err(error) = database::health_check(&connection) {
            error!(?error, "HomeServer database health check failed");
            return HealthSnapshot::needs_attention(
                &self.config.server_name,
                "integrity_check_failed",
            );
        }
        if let Err(error) = update_store::health_check(&connection) {
            error!(?error, "HomeServer update database health check failed");
            return HealthSnapshot::needs_attention(
                &self.config.server_name,
                "update_integrity_check_failed",
            );
        }

        let mut snapshot = HealthSnapshot::running(&self.config.server_name, "ready");
        snapshot.pending_sync = match database::pending_sync_count(&connection) {
            Ok(count) => count,
            Err(error) => {
                warn!(?error, "unable to read pending synchronization count");
                snapshot.state = microgifter_homeserver_core::ServiceState::NeedsAttention;
                snapshot.database = "queue_status_failed".to_owned();
                0
            }
        };

        match database::backup_catalog(
            &connection,
            self.config.pending_restore_plan_path().exists(),
        ) {
            Ok(catalog) => {
                snapshot.restore_pending = catalog.restore_pending;
                snapshot.last_backup = catalog
                    .backups
                    .first()
                    .map(|record| record.created_at_utc.to_rfc3339());
                snapshot.backup = if catalog.restore_pending {
                    "restore_staged".to_owned()
                } else if catalog
                    .backups
                    .iter()
                    .any(|record| record.state == BackupState::Failed)
                {
                    "needs_attention".to_owned()
                } else {
                    "ready".to_owned()
                };
            }
            Err(error) => {
                warn!(?error, "unable to read backup catalog");
                snapshot.state = microgifter_homeserver_core::ServiceState::NeedsAttention;
                snapshot.backup = "catalog_failed".to_owned();
            }
        }

        match update_store::status(
            &connection,
            &self.config.update_manifest_url,
            self.config.update_plan_path().exists(),
        ) {
            Ok(status) => {
                snapshot.update = status.state.as_str().to_owned();
                snapshot.update_version = status.update.map(|record| record.version);
            }
            Err(error) => {
                warn!(?error, "unable to read signed update state");
                snapshot.state = microgifter_homeserver_core::ServiceState::NeedsAttention;
                snapshot.update = "status_failed".to_owned();
            }
        }
        snapshot
    }

    fn backup_catalog(&self) -> Result<BackupCatalog> {
        database::backup_catalog(
            &*self.connection()?,
            self.config.pending_restore_plan_path().exists(),
        )
    }

    fn create_backup(&self, request: CreateBackupRequest) -> Result<BackupActionResult> {
        match &request.kind {
            BackupKind::Manual | BackupKind::Recovery => {
                backup::create_backup(&*self.connection()?, &self.config, request)
            }
            BackupKind::Automatic | BackupKind::PreUpdate => {
                bail!("backup kind is reserved for internal HomeServer operations")
            }
        }
    }

    fn verify_backup(&self, request: BackupReferenceRequest) -> Result<BackupActionResult> {
        backup::verify_backup(&*self.connection()?, &self.config, request)
    }

    fn stage_restore(&self, request: BackupReferenceRequest) -> Result<BackupActionResult> {
        backup::stage_restore(&*self.connection()?, &self.config, request)
    }

    fn create_automatic_backup_if_due(&self) -> Result<()> {
        if let Some(record) = backup::create_automatic_if_due(&*self.connection()?, &self.config)? {
            info!(backup_id = %record.backup_id, "scheduled encrypted backup created");
        }
        Ok(())
    }

    fn new_import_path(&self) -> PathBuf {
        self.config.new_import_path()
    }

    fn import_recovery_package(
        &self,
        temporary_path: PathBuf,
        passphrase: String,
    ) -> Result<BackupActionResult> {
        recovery_transfer::import_recovery_package(
            &*self.connection()?,
            &self.config,
            &temporary_path,
            passphrase,
        )
    }

    fn recovery_package_for_export(
        &self,
        backup_id: &str,
    ) -> Result<recovery_transfer::ExportPackage> {
        recovery_transfer::package_for_export(&*self.connection()?, &self.config, backup_id)
    }

    fn update_status(&self) -> Result<UpdateStatus> {
        update_store::status(
            &*self.connection()?,
            &self.config.update_manifest_url,
            self.config.update_plan_path().exists(),
        )
    }

    async fn check_for_updates(&self) -> Result<UpdateActionResult> {
        update_store::begin_check(&*self.connection()?)?;
        let verified = match update::fetch_and_verify_manifest(
            &self.config,
            env!("CARGO_PKG_VERSION"),
        )
        .await
        {
            Ok(verified) => verified,
            Err(error) => {
                let _ = update_store::record_check_failure(
                    &*self.connection()?,
                    &public_update_failure(&error),
                );
                return Err(error);
            }
        };

        if !update::manifest_is_newer(&verified, env!("CARGO_PKG_VERSION"))? {
            update_store::record_current(&*self.connection()?)?;
            return Ok(UpdateActionResult {
                status: self.update_status()?,
                message: "Microgifter HomeServer is current.".to_owned(),
                restart_required: false,
            });
        }

        update_store::save_available(
            &*self.connection()?,
            &verified.update_id,
            &self.config.update_manifest_url,
            &verified.manifest,
        )?;
        Ok(UpdateActionResult {
            status: self.update_status()?,
            message: format!(
                "Microgifter HomeServer {} is available and its release manifest is valid.",
                verified.manifest.payload.version
            ),
            restart_required: false,
        })
    }

    async fn download_update(&self) -> Result<UpdateActionResult> {
        let stored = {
            let connection = self.connection()?;
            let available = update_store::latest_in_state(&connection, UpdateState::Available)?;
            update_store::mark_downloading(&connection, &available.record.update_id)?
        };
        let path = match update::download_and_verify_installer(
            &self.config,
            &stored.record,
            &stored.manifest,
        )
        .await
        {
            Ok(path) => path,
            Err(error) => {
                let _ = update_store::mark_failure(
                    &*self.connection()?,
                    &stored.record.update_id,
                    &public_update_failure(&error),
                );
                return Err(error);
            }
        };
        let staged =
            update_store::mark_staged(&*self.connection()?, &stored.record.update_id, &path)?;
        Ok(UpdateActionResult {
            status: self.update_status()?,
            message: format!(
                "HomeServer {} was downloaded, hashed, and Authenticode-verified.",
                staged.record.version
            ),
            restart_required: false,
        })
    }

    fn apply_update(&self, request: ApplyUpdateRequest) -> Result<UpdateActionResult> {
        ensure_update_confirmation(&request.confirmation)?;
        let stored = update_store::latest_in_state(&*self.connection()?, UpdateState::Staged)?;
        let installer_path = stored
            .installer_path
            .as_deref()
            .context("staged update installer path is unavailable")?;
        update_apply::verify_staged_installer(&stored.record, installer_path)?;

        let backup_result = backup::create_backup(
            &*self.connection()?,
            &self.config,
            CreateBackupRequest {
                kind: BackupKind::PreUpdate,
                passphrase: None,
                note: Some(format!("Before update to {}", stored.record.version)),
            },
        )?;
        info!(
            backup_id = %backup_result.backup.backup_id,
            update_id = %stored.record.update_id,
            "pre-update encrypted backup created"
        );

        let rollback_path = self
            .config
            .update_rollback_dir
            .join(&stored.record.update_id);
        update_store::mark_applying(
            &*self.connection()?,
            &stored.record.update_id,
            &rollback_path,
        )?;
        if let Err(error) =
            update_apply::prepare_and_launch(&self.config, &stored.record, installer_path)
        {
            let _ = update_store::mark_failure(
                &*self.connection()?,
                &stored.record.update_id,
                &public_update_failure(&error),
            );
            return Err(error);
        }

        Ok(UpdateActionResult {
            status: self.update_status()?,
            message: "The verified updater was launched. HomeServer will restart and report success or automatic rollback."
                .to_owned(),
            restart_required: true,
        })
    }
}

fn ensure_update_confirmation(value: &str) -> Result<()> {
    if value != "UPDATE" {
        bail!("type UPDATE to apply the staged HomeServer release");
    }
    Ok(())
}

fn public_update_failure(error: &anyhow::Error) -> String {
    let text = error.to_string().to_lowercase();
    if text.contains("signature") || text.contains("signing key") {
        "manifest_signature_failed"
    } else if text.contains("https") || text.contains("redirect") {
        "update_transport_rejected"
    } else if text.contains("authenticode") {
        "authenticode_verification_failed"
    } else if text.contains("sha-256") || text.contains("size") || text.contains("truncated") {
        "installer_integrity_failed"
    } else if text.contains("version") {
        "update_version_rejected"
    } else {
        "update_operation_failed"
    }
    .to_owned()
}

fn configure_logging(config: &AppConfig, service_mode: bool) -> Option<WorkerGuard> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("microgifter_homeserver_service=info,tower_http=info"));

    if service_mode {
        let appender = tracing_appender::rolling::daily(
            &config.logs_dir,
            "microgifter-homeserver-service.log",
        );
        let (writer, guard) = tracing_appender::non_blocking(appender);
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(writer)
            .try_init();
        Some(guard)
    } else {
        let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
        None
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "console".to_owned());

    if matches!(command.as_str(), "version" | "--version" | "-V") {
        println!("{} {}", SERVICE_NAME, env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let config = AppConfig::load()?;
    let _log_guard = configure_logging(&config, command == "service");

    match command.as_str() {
        "console" => run_console(config).await,
        "service" => run_service(),
        _ => bail!("unknown command '{command}'; expected console, service, or version"),
    }
}

async fn run_console(config: AppConfig) -> Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown_tx.send(true);
        }
    });
    app::run(config, shutdown_rx, None).await
}

#[cfg(windows)]
fn run_service() -> Result<()> {
    windows_service::service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

#[cfg(not(windows))]
fn run_service() -> Result<()> {
    bail!("Windows service mode is only supported on Windows")
}

#[cfg(windows)]
windows_service::define_windows_service!(ffi_service_main, service_main);

#[cfg(windows)]
fn service_main(_arguments: Vec<std::ffi::OsString>) {
    if let Err(error) = windows_service_runtime() {
        error!(?error, "HomeServer Windows service failed");
    }
}

#[cfg(windows)]
fn windows_service_runtime() -> Result<()> {
    use std::{sync::mpsc, time::Duration};
    use tokio::sync::oneshot;
    use windows_service::{
        service::{
            ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
    };

    let (stop_tx, stop_rx) = mpsc::channel();
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, handler)?;
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(15),
        process_id: None,
    })?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let config = AppConfig::load()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    std::thread::spawn(move || {
        let _ = stop_rx.recv();
        let _ = shutdown_tx.send(true);
    });

    let result: Result<()> = runtime.block_on(async {
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let app_handle = tokio::spawn(app::run(config, shutdown_rx, Some(ready_tx)));

        match tokio::time::timeout(Duration::from_secs(15), ready_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Err(anyhow!("HomeServer stopped before reporting readiness")),
            Err(_) => return Err(anyhow!("HomeServer readiness timed out")),
        }

        status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        app_handle
            .await
            .context("HomeServer service task terminated unexpectedly")??;
        Ok(())
    });

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::StopPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 1,
        wait_hint: Duration::from_secs(5),
        process_id: None,
    })?;

    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: if result.is_ok() {
            ServiceExitCode::Win32(0)
        } else {
            ServiceExitCode::Win32(1)
        },
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    info!("HomeServer Windows service stopped");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_application_requires_exact_confirmation() {
        assert!(ensure_update_confirmation("UPDATE").is_ok());
        assert!(ensure_update_confirmation("update").is_err());
        assert!(ensure_update_confirmation(" UPDATE ").is_err());
    }
}
