mod app;
mod config;
mod database;
mod http;

use anyhow::{anyhow, bail, Context, Result};
use config::AppConfig;
use microgifter_homeserver_core::{HealthSnapshot, SERVICE_NAME};
use rusqlite::Connection;
use std::sync::Mutex;
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

    fn snapshot(&self) -> HealthSnapshot {
        let connection = match self.connection.lock() {
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
        snapshot
    }
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
        let (ready_tx, ready_rx) = oneshot::channel();
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
