use crate::{backup, config::AppConfig, database, http, update, update_store, AppState};
use anyhow::{Context, Result};
use chrono::Utc;
use microgifter_homeserver_core::{API_HOST, API_PORT};
use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};
use tokio::sync::{oneshot, watch};
use tracing::{error, info, warn};

pub async fn run(
    config: AppConfig,
    shutdown: watch::Receiver<bool>,
    ready: Option<oneshot::Sender<()>>,
) -> Result<()> {
    info!(data_dir = %config.data_dir.display(), "starting HomeServer service");

    let restore_outcome = match backup::apply_pending_restore(&config) {
        Ok(outcome) => outcome,
        Err(error) => {
            error!(?error, "invalid staged restore was quarantined");
            quarantine_pending_restore(&config)?;
            None
        }
    };
    let connection = database::initialize(&config.database_path)?;
    update_store::initialize(&connection)?;
    if let Some(outcome) = restore_outcome {
        match outcome {
            backup::RestoreOutcome::Applied {
                restore_id,
                backup_id,
                rollback_path,
            } => {
                database::record_restore_applied(
                    &connection,
                    &restore_id,
                    &backup_id,
                    rollback_path.as_deref(),
                )?;
                info!(%restore_id, %backup_id, "staged HomeServer restore applied");
            }
            backup::RestoreOutcome::RolledBack {
                restore_id,
                backup_id,
                failure_code,
            } => {
                database::record_restore_rolled_back(
                    &connection,
                    &restore_id,
                    &backup_id,
                    &failure_code,
                )?;
                warn!(%restore_id, %backup_id, %failure_code, "staged restore failed and the previous database was restored");
            }
        }
    }
    if let Some(result) = update::consume_application_result(&config)? {
        update_store::record_application_result(&connection, &result)?;
        info!(
            update_id = %result.update_id,
            state = %result.state.as_str(),
            "HomeServer updater result recorded"
        );
    }

    let state = Arc::new(AppState::new(config, connection));
    let address: SocketAddr = format!("{API_HOST}:{API_PORT}").parse()?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("unable to bind local API at {address}"))?;

    let backup_scheduler = tokio::spawn(run_backup_scheduler(state.clone(), shutdown.clone()));
    let update_scheduler = tokio::spawn(run_update_scheduler(state.clone(), shutdown.clone()));
    info!(%address, "HomeServer local API ready");
    if let Some(ready) = ready {
        let _ = ready.send(());
    }

    let result = axum::serve(listener, http::router(state))
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await;
    backup_scheduler.abort();
    update_scheduler.abort();
    result?;
    Ok(())
}

async fn run_backup_scheduler(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut interval = tokio::time::interval_at(start, Duration::from_secs(15 * 60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let scheduled_state = state.clone();
                match tokio::task::spawn_blocking(move || scheduled_state.create_automatic_backup_if_due()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => error!(?error, "scheduled HomeServer backup failed"),
                    Err(error) => error!(?error, "scheduled HomeServer backup task failed"),
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

async fn run_update_scheduler(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let start = tokio::time::Instant::now() + Duration::from_secs(5 * 60);
    let mut interval = tokio::time::interval_at(start, Duration::from_secs(6 * 60 * 60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(error) = state.check_for_updates().await {
                    warn!(?error, "scheduled HomeServer update check failed");
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    if *shutdown.borrow() {
        return;
    }

    while shutdown.changed().await.is_ok() {
        if *shutdown.borrow() {
            return;
        }
    }
}

fn quarantine_pending_restore(config: &AppConfig) -> Result<()> {
    let suffix = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    for (path, label) in [
        (config.pending_restore_plan_path(), "plan"),
        (config.pending_restore_database_path(), "database"),
    ] {
        if path.exists() {
            let quarantined = config
                .restore_dir
                .join(format!("invalid-restore-{label}-{suffix}"));
            move_replace(&path, &quarantined)?;
        }
    }
    Ok(())
}

fn move_replace(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        std::fs::remove_file(destination)?;
    }
    std::fs::rename(source, destination)?;
    Ok(())
}
