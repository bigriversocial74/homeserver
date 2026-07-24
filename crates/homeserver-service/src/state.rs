use crate::{cloud, config::AppConfig, database, secrets};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use microgifter_homeserver_core::{
    CloudConnectionSnapshot, CloudConnectionState, EnqueueSyncRequest, HealthSnapshot,
    PairCloudRequest, ServiceState, SyncRunSnapshot,
};
use rusqlite::Connection;
use serde_json::json;
use std::sync::{Mutex, MutexGuard};
use tracing::{error, warn};
use uuid::Uuid;

const ALLOWED_LOCAL_OPERATIONS: &[&str] = &[
    "device.heartbeat",
    "local.settings.snapshot",
    "cache.refresh.request",
];

pub struct AppState {
    config: AppConfig,
    connection: Mutex<Connection>,
    cloud: cloud::CloudClient,
}

impl AppState {
    pub fn new(config: AppConfig, connection: Connection) -> Result<Self> {
        Ok(Self {
            config,
            connection: Mutex::new(connection),
            cloud: cloud::CloudClient::new()?,
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|error| anyhow!("HomeServer database lock was poisoned: {error}"))
    }

    pub fn snapshot(&self) -> HealthSnapshot {
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

        let cloud = database::cloud_connection(&connection)
            .map(|record| record.snapshot)
            .unwrap_or_else(|error| {
                warn!(?error, "unable to read HomeServer cloud state");
                CloudConnectionSnapshot {
                    state: CloudConnectionState::Degraded,
                    last_error: Some("connection_state_failed".to_owned()),
                    ..CloudConnectionSnapshot::default()
                }
            });
        let mut snapshot =
            HealthSnapshot::running(&self.config.server_name, "ready").with_cloud(&cloud);
        snapshot.pending_sync = match database::pending_sync_count(&connection) {
            Ok(count) => count,
            Err(error) => {
                warn!(?error, "unable to read pending synchronization count");
                snapshot.state = ServiceState::NeedsAttention;
                snapshot.database = "queue_status_failed".to_owned();
                0
            }
        };
        snapshot
    }

    pub fn cloud_snapshot(&self) -> Result<CloudConnectionSnapshot> {
        Ok(database::cloud_connection(&*self.connection()?)?.snapshot)
    }

    pub async fn pair_cloud(&self, request: PairCloudRequest) -> Result<CloudConnectionSnapshot> {
        let installation_id = database::installation_id(&*self.connection()?)?;
        let outcome = self
            .cloud
            .pair(
                &request.cloud_base_url,
                &request.pairing_code,
                &installation_id,
                &self.config.server_name,
            )
            .await?;

        secrets::save(&installation_id, &outcome.secrets)?;
        if let Err(error) = database::save_cloud_connection(
            &*self.connection()?,
            &outcome.cloud_base_url,
            &outcome.device_id,
            &outcome.public_key_base64,
            &outcome.scopes,
        ) {
            let _ = secrets::delete(&installation_id);
            return Err(error).context("unable to persist HomeServer cloud pairing state");
        }

        let record = database::cloud_connection(&*self.connection()?)?;
        if let Err(error) = self.cloud.status(&record, &outcome.secrets).await {
            database::mark_cloud_error(
                &*self.connection()?,
                &public_cloud_error(&error),
                cloud::authentication_failed(&error),
            )?;
            return Err(error).context("pairing completed but signed cloud verification failed");
        }
        database::mark_cloud_success(&*self.connection()?)?;
        self.enqueue_heartbeat()?;
        self.cloud_snapshot()
    }

    pub fn disconnect_cloud(&self) -> Result<CloudConnectionSnapshot> {
        let installation_id = database::installation_id(&*self.connection()?)?;
        secrets::delete(&installation_id)?;
        database::clear_cloud_connection(&*self.connection()?)?;
        Ok(CloudConnectionSnapshot::default())
    }

    pub fn enqueue_sync(&self, request: EnqueueSyncRequest) -> Result<String> {
        let operation_type = request.operation_type.trim().to_lowercase();
        if !ALLOWED_LOCAL_OPERATIONS.contains(&operation_type.as_str()) {
            bail!("synchronization operation is not enabled for HomeServer v1");
        }
        let idempotency_key = request
            .idempotency_key
            .unwrap_or_else(|| format!("homeserver:{}", Uuid::new_v4().simple()));
        validate_idempotency_key(&idempotency_key)?;
        database::enqueue_sync(
            &*self.connection()?,
            &idempotency_key,
            &operation_type,
            &request.payload,
        )?;
        Ok(idempotency_key)
    }

    pub fn enqueue_heartbeat(&self) -> Result<String> {
        let connection = self.connection()?;
        let installation_id = database::installation_id(&connection)?;
        let bucket = Utc::now().timestamp() / 300;
        let key = format!("heartbeat:{installation_id}:{bucket}");
        database::enqueue_sync(
            &connection,
            &key,
            "device.heartbeat",
            &json!({
                "installation_id": installation_id,
                "server_name": &self.config.server_name,
                "version": env!("CARGO_PKG_VERSION"),
            }),
        )?;
        Ok(key)
    }

    pub async fn sync_once(&self) -> Result<SyncRunSnapshot> {
        let (record, installation_id, operations) = {
            let mut connection = self.connection()?;
            let record = database::cloud_connection(&connection)?;
            match record.snapshot.state {
                CloudConnectionState::NotPaired => {
                    let pending = database::pending_sync_count(&connection)?;
                    return Ok(SyncRunSnapshot {
                        processed: 0,
                        accepted: 0,
                        rejected: 0,
                        review: 0,
                        pending,
                    });
                }
                CloudConnectionState::Revoked => {
                    bail!("HomeServer cloud credentials were revoked; pair the device again");
                }
                _ => {}
            }
            let installation_id = database::installation_id(&connection)?;
            let operations = database::claim_due_sync(&mut connection, 25)?;
            (record, installation_id, operations)
        };

        let secrets = match secrets::load(&installation_id) {
            Ok(secrets) => secrets,
            Err(error) => {
                database::mark_cloud_error(
                    &*self.connection()?,
                    "credential_vault_unavailable",
                    false,
                )?;
                return Err(error);
            }
        };

        if operations.is_empty() {
            match self.cloud.status(&record, &secrets).await {
                Ok(()) => database::mark_cloud_success(&*self.connection()?)?,
                Err(error) => {
                    database::mark_cloud_error(
                        &*self.connection()?,
                        &public_cloud_error(&error),
                        cloud::authentication_failed(&error),
                    )?;
                    return Err(error);
                }
            }
            return Ok(SyncRunSnapshot {
                processed: 0,
                accepted: 0,
                rejected: 0,
                review: 0,
                pending: database::pending_sync_count(&*self.connection()?)?,
            });
        }

        let receipts = match self.cloud.sync(&record, &secrets, &operations).await {
            Ok(receipts) => receipts,
            Err(error) => {
                let connection = self.connection()?;
                database::retry_operations(&connection, &operations)?;
                database::mark_cloud_error(
                    &connection,
                    &public_cloud_error(&error),
                    cloud::authentication_failed(&error),
                )?;
                return Err(error);
            }
        };
        if let Err(error) = validate_receipts(&operations, &receipts) {
            let connection = self.connection()?;
            database::retry_operations(&connection, &operations)?;
            database::mark_cloud_error(&connection, "invalid_receipt_set", false)?;
            return Err(error);
        }

        let mut accepted = 0;
        let mut rejected = 0;
        let mut review = 0;
        for receipt in &receipts {
            match receipt.disposition.as_str() {
                "accepted" => accepted += 1,
                "rejected" => rejected += 1,
                "review" => review += 1,
                _ => unreachable!("receipt validation rejects unknown dispositions"),
            }
        }
        let mut connection = self.connection()?;
        database::apply_receipts(&mut connection, &receipts)?;
        database::mark_cloud_success(&connection)?;
        let pending = database::pending_sync_count(&connection)?;

        Ok(SyncRunSnapshot {
            processed: receipts.len() as u64,
            accepted,
            rejected,
            review,
            pending,
        })
    }
}

fn validate_idempotency_key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 190
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.:-".contains(character))
    {
        bail!("idempotency key is invalid");
    }
    Ok(())
}

fn validate_receipts(
    operations: &[database::QueuedOperation],
    receipts: &[database::ReceiptRecord],
) -> Result<()> {
    if operations.len() != receipts.len() {
        bail!("Microgifter returned an incomplete synchronization receipt set");
    }
    for operation in operations {
        let matching = receipts
            .iter()
            .filter(|receipt| {
                receipt.idempotency_key == operation.idempotency_key
                    && receipt.operation_type == operation.operation_type
            })
            .count();
        if matching != 1 {
            bail!("Microgifter returned an invalid synchronization receipt set");
        }
    }
    for receipt in receipts {
        if !matches!(
            receipt.disposition.as_str(),
            "accepted" | "rejected" | "review"
        ) {
            bail!("Microgifter returned an unsupported synchronization disposition");
        }
    }
    Ok(())
}

fn public_cloud_error(error: &anyhow::Error) -> String {
    let text = error.to_string();
    if text.starts_with("cloud_authentication_failed:") {
        "authentication_failed".to_owned()
    } else if text.starts_with("cloud_request_rejected:") {
        "request_rejected".to_owned()
    } else {
        "cloud_unavailable".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_declared_low_risk_operations_are_allowed() {
        assert!(ALLOWED_LOCAL_OPERATIONS.contains(&"device.heartbeat"));
        assert!(!ALLOWED_LOCAL_OPERATIONS.contains(&"commerce.order.create"));
    }

    #[test]
    fn idempotency_keys_are_bounded_and_portable() {
        assert!(validate_idempotency_key("local.settings:abc-123").is_ok());
        assert!(validate_idempotency_key("not valid").is_err());
        assert!(validate_idempotency_key(&"x".repeat(191)).is_err());
    }
}
