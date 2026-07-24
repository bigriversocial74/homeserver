mod app;
mod config;
mod database;
mod http;

use anyhow::{bail, Result};
use config::AppConfig;
use microgifter_homeserver_core::{HealthSnapshot, SERVICE_NAME};
use rusqlite::Connection;
use std::sync::Mutex;
use tokio::sync::watch;
use tracing::{error, info};
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
        let mut snapshot = HealthSnapshot::running(&self.config.server_name, "ready");
        snapshot.pending_sync = self
            .connection
            .lock()
            .ok()
            .and_then(|connection| database::pending_sync_count(&connection).ok())
            .unwrap_or_default();
        snapshot
    }
}

fn configure_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("microgifter_homeserver_service=info,tower_http=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init();
}

#[tokio::main]
async fn main() -> Result<()> {
    configure_logging();
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "console".to_owned());

    match command.as_str() {
        "console" => run_console().await,
        "service" => run_service(),
        "version" | "--version" | "-V" => {
            println!("{} {}", SERVICE_NAME, env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => bail!("unknown command '{command}'; expected console, service, or version"),
    }
}

async fn run_console() -> Result<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = shutdown_tx.send(true);
        }
    });
    app::run(shutdown_rx).await
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
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    std::thread::spawn(move || {
        let _ = stop_rx.recv();
        let _ = shutdown_tx.send(true);
    });

    let result = runtime.block_on(app::run(shutdown_rx));
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
