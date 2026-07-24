use crate::AppState;
use microgifter_homeserver_core::CloudConnectionState;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::{info, warn};

const SYNC_INTERVAL: Duration = Duration::from_secs(60);

pub async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let mut interval = tokio::time::interval(SYNC_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    info!("HomeServer synchronization worker stopped");
                    return;
                }
            }
            _ = interval.tick() => {
                let connection = match state.cloud_snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        warn!(?error, "unable to inspect HomeServer cloud state");
                        continue;
                    }
                };
                if matches!(connection.state, CloudConnectionState::NotPaired | CloudConnectionState::Revoked) {
                    continue;
                }
                if let Err(error) = state.enqueue_heartbeat() {
                    warn!(?error, "unable to queue HomeServer heartbeat");
                }
                if let Err(error) = state.sync_once().await {
                    warn!(?error, "HomeServer synchronization attempt failed");
                }
            }
        }
    }
}
