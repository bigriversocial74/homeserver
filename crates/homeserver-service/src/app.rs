#[path = "activity.rs"]
mod activity;

#[path = "cloud_pairing_v2.rs"]
mod cloud_pairing_v2;

#[path = "cloud_registry.rs"]
pub(crate) mod cloud_registry;

#[path = "cloud_connector.rs"]
mod cloud_connector;

#[allow(
    dead_code,
    reason = "POD runtime result schema accepts optional adapter metadata"
)]
#[path = "app/pod_provider_runtime.rs"]
mod pod_provider_runtime;

#[allow(
    dead_code,
    reason = "VP3 response schemas retain forward-compatible server contract fields"
)]
#[path = "vp3_client.rs"]
mod vp3_client;

#[path = "vp3_device_binding.rs"]
mod vp3_device_binding;

#[path = "federated_settings.rs"]
mod federated_settings;

#[path = "federated_settings_signature.rs"]
mod federated_settings_signature;

#[path = "app/wrapper_core.rs"]
mod wrapper_core;

#[path = "app/wrapper_grants.rs"]
mod wrapper_grants;

#[path = "app/wrapper_jobs.rs"]
mod wrapper_jobs;

#[path = "app/wrapper_agents.rs"]
mod wrapper_agents;

#[path = "app/wrapper_privacy.rs"]
mod wrapper_privacy;

#[path = "app/wrapper_runtime.rs"]
mod wrapper_runtime;

use crate::{
    agent_runtime, backup, config::AppConfig, database, document_extraction, http, knowledge_vault,
    mcp_runtime, microgifter_connection, model_center, openrouter_provider, operational_data,
    review_intelligence, semantic_vault, software_authority, update, update_store, AppState,
};
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
    activity::initialize(&connection)?;
    update_store::initialize(&connection)?;
    software_authority::initialize(&connection)?;
    vp3_client::initialize(&connection)?;
    federated_settings::initialize(&connection)?;
    cloud_connector::initialize(&connection)?;
    cloud_registry::initialize(&connection)?;
    microgifter_connection::initialize(&connection)?;
    pod_provider_runtime::initialize(&connection)?;
    wrapper_core::initialize(&connection)?;
    wrapper_grants::initialize(&connection)?;
    wrapper_jobs::initialize(&connection)?;
    wrapper_agents::initialize(&connection)?;
    operational_data::initialize(&connection)?;
    knowledge_vault::initialize(&connection, &config)?;
    wrapper_privacy::initialize(&connection)?;
    wrapper_privacy::maintain_history(&connection)?;
    wrapper_runtime::initialize(&connection)?;
    document_extraction::initialize(&connection)?;
    model_center::initialize(&connection)?;
    openrouter_provider::initialize(&connection)?;
    semantic_vault::initialize(&connection)?;
    review_intelligence::initialize(&connection)?;
    agent_runtime::initialize(&connection)?;
    mcp_runtime::initialize(&connection)?;
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
        software_authority::record_update_result_receipt(
            &connection,
            &result.update_id,
            &result.target_version,
            result.state.as_str(),
            result.failure_code.as_deref(),
        )?;
        info!(
            update_id = %result.update_id,
            state = %result.state.as_str(),
            "HomeServer updater result recorded"
        );
    }

    let state = Arc::new(AppState::new(config, connection));
    if let Err(error) = state.maintain_runtime_history() {
        warn!(
            ?error,
            "HomeServer runtime history retention failed during startup"
        );
    }
    let address: SocketAddr = format!("{API_HOST}:{API_PORT}").parse()?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("unable to bind local API at {address}"))?;

    let backup_scheduler = tokio::spawn(run_backup_scheduler(state.clone(), shutdown.clone()));
    let update_scheduler = tokio::spawn(run_update_scheduler(state.clone(), shutdown.clone()));
    let cloud_worker = tokio::spawn(cloud_registry::run(state.clone(), shutdown.clone()));
    let microgifter_connection_worker =
        tokio::spawn(microgifter_connection::run(state.clone(), shutdown.clone()));
    let vp3_authority_worker = tokio::spawn(vp3_client::run(state.clone(), shutdown.clone()));
    let pod_provider_worker =
        tokio::spawn(pod_provider_runtime::run(state.clone(), shutdown.clone()));
    let agent_tool_runtime_worker =
        tokio::spawn(wrapper_runtime::run(state.clone(), shutdown.clone()));
    let review_intelligence_worker = tokio::spawn(run_review_intelligence_scheduler(
        state.clone(),
        shutdown.clone(),
    ));
    info!(%address, "HomeServer local API ready");
    if let Some(ready) = ready {
        let _ = ready.send(());
    }

    let registry_router = cloud_registry::router(state.clone()).layer(axum::middleware::from_fn(
        cloud_pairing_v2::reject_legacy_pairing,
    ));
    let vp3_router = vp3_client::router(state.clone()).layer(axum::middleware::from_fn_with_state(
        state.clone(),
        vp3_device_binding::bind_activation_identity,
    ));
    let router = http::secure(
        http::router(state.clone())
            .merge(activity::router(state.clone()))
            .merge(cloud_connector::router(state.clone()))
            .merge(registry_router)
            .merge(cloud_pairing_v2::router(state.clone()))
            .merge(software_authority::router(state.clone()))
            .merge(vp3_device_binding::router(state.clone()))
            .merge(vp3_router)
            .merge(federated_settings::router(state.clone()))
            .merge(microgifter_connection::router(state.clone()))
            .merge(pod_provider_runtime::router(state.clone()))
            .merge(wrapper_core::router(state.clone()))
            .merge(wrapper_grants::router(state.clone()))
            .merge(wrapper_jobs::router(state.clone()))
            .merge(wrapper_agents::router(state.clone()))
            .merge(wrapper_privacy::router(state.clone()))
            .merge(wrapper_runtime::router(state.clone()))
            .merge(knowledge_vault::router(state.clone()))
            .merge(model_center::router(state.clone()))
            .merge(openrouter_provider::router(state.clone()))
            .merge(semantic_vault::router(state.clone()))
            .merge(operational_data::router(state.clone()))
            .merge(review_intelligence::router(state.clone()))
            .merge(agent_runtime::router(state.clone()))
            .merge(mcp_runtime::router(state.clone())),
    );
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await;
    if result.is_ok() {
        if let Err(error) = activity::record_service_stopped(&state) {
            warn!(
                ?error,
                "unable to record graceful HomeServer service shutdown"
            );
        }
    }
    backup_scheduler.abort();
    update_scheduler.abort();
    cloud_worker.abort();
    microgifter_connection_worker.abort();
    vp3_authority_worker.abort();
    pod_provider_worker.abort();
    agent_tool_runtime_worker.abort();
    review_intelligence_worker.abort();
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
                match tokio::task::spawn_blocking(move || {
                    if let Err(error) = scheduled_state.prune_logs() {
                        warn!(?error, "scheduled HomeServer log retention failed");
                    }
                    if let Err(error) = scheduled_state.maintain_runtime_history() {
                        warn!(?error, "scheduled HomeServer runtime retention failed");
                    }
                    if let Ok(connection) = scheduled_state.connection() {
                        if let Err(error) = pod_provider_runtime::maintain_history(&connection) {
                            warn!(?error, "scheduled POD provider retention failed");
                        }
                        if let Err(error) = openrouter_provider::maintain_history(&connection) {
                            warn!(?error, "scheduled OpenRouter receipt retention failed");
                        }
                        if let Err(error) = vp3_client::maintain_history(&connection) {
                            warn!(?error, "scheduled VP3 authority retention failed");
                        }
                        if let Err(error) = federated_settings::maintain_history(&connection) {
                            warn!(?error, "scheduled federated settings retention failed");
                        }
                        if let Err(error) = wrapper_core::maintain_history(&connection) {
                            warn!(?error, "scheduled wrapper registry retention failed");
                        }
                        if let Err(error) = wrapper_grants::maintain_history(&connection) {
                            warn!(?error, "scheduled wrapper grant retention failed");
                        }
                        if let Err(error) = wrapper_jobs::maintain_history(&connection) {
                            warn!(?error, "scheduled wrapper job retention failed");
                        }
                        if let Err(error) = wrapper_agents::maintain_history(&connection) {
                            warn!(?error, "scheduled wrapper agent retention failed");
                        }
                        if let Err(error) = wrapper_runtime::maintain_history(&connection) {
                            warn!(?error, "scheduled agent tool runtime retention failed");
                        }
                    }
                    scheduled_state.create_automatic_backup_if_due()
                }).await {
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

async fn run_review_intelligence_scheduler(
    state: Arc<AppState>,
    mut shutdown: watch::Receiver<bool>,
) {
    let start = tokio::time::Instant::now() + Duration::from_secs(2 * 60);
    let mut interval = tokio::time::interval_at(start, Duration::from_secs(15 * 60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                match review_intelligence::run_automatic_processing_cycle(state.clone()).await {
                    Ok(summary) if summary.enabled && summary.failed_operations > 0 => {
                        warn!(
                            failures = summary.failed_operations,
                            connections = summary.connections_considered,
                            datasets = summary.datasets_synchronized,
                            "automatic Review Intelligence cycle completed with errors"
                        );
                    }
                    Ok(summary) if summary.enabled => {
                        info!(
                            connections = summary.connections_considered,
                            datasets = summary.datasets_synchronized,
                            records = summary.records_received,
                            events = summary.events_received,
                            analyses = summary.analyses_run,
                            "automatic Review Intelligence cycle completed"
                        );
                    }
                    Ok(_) => {}
                    Err(error) => warn!(?error, "automatic Review Intelligence cycle failed"),
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
    let check_start = tokio::time::Instant::now() + Duration::from_secs(5 * 60);
    let mut check_interval =
        tokio::time::interval_at(check_start, Duration::from_secs(6 * 60 * 60));
    let result_start = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut result_interval = tokio::time::interval_at(result_start, Duration::from_secs(15));
    loop {
        tokio::select! {
            _ = check_interval.tick() => {
                let use_legacy_manifest = state
                    .connection()
                    .ok()
                    .and_then(|connection| software_authority::status_snapshot(&connection).ok())
                    .map_or(true, |authority| authority.current_authority != "vp3");
                if use_legacy_manifest {
                    if let Err(error) = state.check_for_updates().await {
                        warn!(?error, "scheduled legacy HomeServer update check failed");
                    }
                }
            }
            _ = result_interval.tick() => {
                if let Err(error) = state.consume_update_result_if_present() {
                    warn!(?error, "unable to consume HomeServer updater result");
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
