use crate::{config::AppConfig, database, http, AppState};
use anyhow::{Context, Result};
use microgifter_homeserver_core::{API_HOST, API_PORT};
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::{oneshot, watch};
use tracing::info;

pub async fn run(
    config: AppConfig,
    shutdown: watch::Receiver<bool>,
    ready: Option<oneshot::Sender<()>>,
) -> Result<()> {
    info!(data_dir = %config.data_dir.display(), "starting HomeServer service");

    let connection = database::initialize(&config.database_path)?;
    let state = Arc::new(AppState::new(config, connection));
    let address: SocketAddr = format!("{API_HOST}:{API_PORT}").parse()?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("unable to bind local API at {address}"))?;

    info!(%address, "HomeServer local API ready");
    if let Some(ready) = ready {
        let _ = ready.send(());
    }

    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await?;

    Ok(())
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
