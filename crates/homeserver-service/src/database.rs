use anyhow::{bail, ensure, Context, Result};
use microgifter_homeserver_core::{CloudConnectionSnapshot, CloudConnectionState};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

const INITIAL_MIGRATION: &str = include_str!("../../../database/migrations/0001_initial.sql");
const CLOUD_MIGRATION: &str = include_str!("../../../database/migrations/0002_cloud_pairing_sync.sql");
const INITIAL_MIGRATION_KEY: &str = "0001_initial";
const CLOUD_MIGRATION_KEY: &str = "0002_cloud_pairing_sync";

#[derive(Debug, Clone)]
pub struct QueuedOperation {
    pub queue_id: i64,
    pub idempotency_key: String,
    pub operation_type: String,
    pub payload: Value,
    pub attempts: u32,
}

#[derive(Debug, Clone)]
pub struct CloudConnectionRecord {
    pub snapshot: CloudConnectionSnapshot,
    pub public_key_base64: String,
}

#[derive(Debug, Clone)]
pub struct ReceiptRecord {
    pub receipt_id: String,
    pub idempotency_key: String,
    pub operation_type: String,
    pub disposition: String,
    pub reason_code: Option<String>,
    pub response: Value,
}

pub fn initialize(path: &Path) -> Result<Connection> {
    let mut connection = Connection::open(path)
        .with_context(|| format!("unable to open database at {}", path.display()))?;

    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    connection.pragma_update(None, "busy_timeout", 5_000_i64)?;

    let transaction = connection.transaction()?;
    transaction.execute_batch(INITIAL_MIGRATION)?;
    transaction.execute_batch(CLOUD_MIGRATION)?;
    transaction.execute(
        "INSERT OR IGNORE INTO homeserver_settings (setting_key, setting_value) VALUES ('installation_id', ?1)",
        params![Uuid::new_v4().to_string()],
    )?;
    transaction.execute(
        "INSERT INTO service_events (event_type, message) VALUES ('service.database_ready', 'Local database opened and verified')",
        [],
    )?;
    transaction.commit()?;

    health_check(&connection)?;
    Ok(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let quick_check: String =
        connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    ensure!(quick_check == "ok", "SQLite quick_check returned '{quick_check}'");

    for migration in [INITIAL_MIGRATION_KEY, CLOUD_MIGRATION_KEY] {
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key = ?1",
            params![migration],
            |row| row.get(0),
        )?;
        ensure!(count == 1, "migration '{migration}' is not registered exactly once");
    }

    Ok(())
}

pub fn installation_id(connection: &Connection) -> Result<String> {
    connection
        .query_row(
            "SELECT setting_value FROM homeserver_settings WHERE setting_key = 'installation_id'",
            [],
            |row| row.get(0),
        )
        .context("installation identity is unavailable")
}

pub fn pending_sync_count(connection: &Connection) -> Result<u64> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sync_queue WHERE state IN ('pending','processing')",
        [],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

pub fn cloud_connection(connection: &Connection) -> Result<CloudConnectionRecord> {
    let row = connection
        .query_row(
            "SELECT cloud_base_url,device_id,public_key_base64,state,scopes_json,paired_at_utc,last_success_utc,last_error FROM cloud_connection WHERE singleton_id=1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()?;

    let Some((base_url, device_id, public_key, state, scopes_json, paired_at, last_success, last_error)) = row else {
        return Ok(CloudConnectionRecord {
            snapshot: CloudConnectionSnapshot::default(),
            public_key_base64: String::new(),
        });
    };
    let state = match state.as_str() {
        "pairing" => CloudConnectionState::Pairing,
        "connected" => CloudConnectionState::Connected,
        "degraded" => CloudConnectionState::Degraded,
        "revoked" => CloudConnectionState::Revoked,
        _ => CloudConnectionState::Degraded,
    };
    let scopes = serde_json::from_str::<Vec<String>>(&scopes_json).unwrap_or_default();
    Ok(CloudConnectionRecord {
        snapshot: CloudConnectionSnapshot {
            state,
            cloud_base_url: Some(base_url),
            device_id: Some(device_id),
            scopes,
            paired_at_utc: Some(paired_at),
            last_success_utc: last_success,
            last_error,
        },
        public_key_base64: public_key,
    })
}

pub fn save_cloud_connection(
    connection: &Connection,
    cloud_base_url: &str,
    device_id: &str,
    public_key_base64: &str,
    scopes: &[String],
) -> Result<()> {
    connection.execute(
        "INSERT INTO cloud_connection (singleton_id,cloud_base_url,device_id,public_key_base64,state,scopes_json,paired_at_utc,last_success_utc,last_error,updated_at_utc)
         VALUES (1,?1,?2,?3,'connected',?4,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(singleton_id) DO UPDATE SET cloud_base_url=excluded.cloud_base_url,device_id=excluded.device_id,public_key_base64=excluded.public_key_base64,state='connected',scopes_json=excluded.scopes_json,paired_at_utc=excluded.paired_at_utc,last_success_utc=excluded.last_success_utc,last_error=NULL,updated_at_utc=excluded.updated_at_utc",
        params![cloud_base_url, device_id, public_key_base64, serde_json::to_string(scopes)?],
    )?;
    Ok(())
}

pub fn mark_cloud_success(connection: &Connection) -> Result<()> {
    connection.execute(
        "UPDATE cloud_connection SET state='connected',last_success_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        [],
    )?;
    Ok(())
}

pub fn mark_cloud_error(connection: &Connection, reason: &str, revoked: bool) -> Result<()> {
    let state = if revoked { "revoked" } else { "degraded" };
    connection.execute(
        "UPDATE cloud_connection SET state=?1,last_error=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![state, reason.chars().take(500).collect::<String>()],
    )?;
    Ok(())
}

pub fn clear_cloud_connection(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM cloud_connection WHERE singleton_id=1", [])?;
    connection.execute(
        "UPDATE sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='processing'",
        [],
    )?;
    Ok(())
}

pub fn enqueue_sync(
    connection: &Connection,
    idempotency_key: &str,
    operation_type: &str,
    payload: &Value,
) -> Result<i64> {
    let payload_json = serde_json::to_string(payload)?;
    let existing = connection
        .query_row(
            "SELECT queue_id,operation_type,payload_json FROM sync_queue WHERE idempotency_key=?1",
            params![idempotency_key],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        )
        .optional()?;
    if let Some((queue_id, existing_type, existing_payload)) = existing {
        if existing_type != operation_type || existing_payload != payload_json {
            bail!("idempotency key is already bound to different synchronization work");
        }
        return Ok(queue_id);
    }
    connection.execute(
        "INSERT INTO sync_queue (idempotency_key,operation_type,payload_json,state,attempts,available_at_utc,created_at_utc,updated_at_utc) VALUES (?1,?2,?3,'pending',0,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![idempotency_key, operation_type, payload_json],
    )?;
    Ok(connection.last_insert_rowid())
}

pub fn claim_due_sync(connection: &mut Connection, limit: usize) -> Result<Vec<QueuedOperation>> {
    let transaction = connection.transaction()?;
    transaction.execute(
        "UPDATE sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE state='processing' AND updated_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-5 minutes')",
        [],
    )?;
    let mut statement = transaction.prepare(
        "SELECT queue_id,idempotency_key,operation_type,payload_json,attempts FROM sync_queue WHERE state='pending' AND available_at_utc<=strftime('%Y-%m-%dT%H:%M:%fZ','now') ORDER BY queue_id LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![limit.max(1) as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    let mut operations = Vec::with_capacity(rows.len());
    for (queue_id, idempotency_key, operation_type, payload_json, attempts) in rows {
        transaction.execute(
            "UPDATE sync_queue SET state='processing',attempts=attempts+1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE queue_id=?1 AND state='pending'",
            params![queue_id],
        )?;
        operations.push(QueuedOperation {
            queue_id,
            idempotency_key,
            operation_type,
            payload: serde_json::from_str(&payload_json)
                .with_context(|| format!("sync queue payload {queue_id} is invalid"))?,
            attempts: attempts.max(0) as u32 + 1,
        });
    }
    transaction.commit()?;
    Ok(operations)
}

pub fn apply_receipts(connection: &mut Connection, receipts: &[ReceiptRecord]) -> Result<()> {
    let transaction = connection.transaction()?;
    for receipt in receipts {
        let state = match receipt.disposition.as_str() {
            "accepted" => "accepted",
            "rejected" => "rejected",
            "review" => "review",
            other => bail!("unsupported cloud receipt disposition '{other}'"),
        };
        transaction.execute(
            "UPDATE sync_queue SET state=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE idempotency_key=?2",
            params![state, receipt.idempotency_key],
        )?;
        transaction.execute(
            "INSERT INTO sync_receipts (receipt_id,idempotency_key,operation_type,disposition,reason_code,response_json,received_at_utc) VALUES (?1,?2,?3,?4,?5,?6,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(idempotency_key) DO UPDATE SET receipt_id=excluded.receipt_id,operation_type=excluded.operation_type,disposition=excluded.disposition,reason_code=excluded.reason_code,response_json=excluded.response_json,received_at_utc=excluded.received_at_utc",
            params![receipt.receipt_id, receipt.idempotency_key, receipt.operation_type, receipt.disposition, receipt.reason_code, serde_json::to_string(&receipt.response)?],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

pub fn retry_operations(connection: &Connection, operations: &[QueuedOperation]) -> Result<()> {
    for operation in operations {
        let delay_seconds = (2_u64.pow(operation.attempts.min(8)) * 5).min(1_800);
        let modifier = format!("+{delay_seconds} seconds");
        connection.execute(
            "UPDATE sync_queue SET state='pending',available_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now',?1),updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE queue_id=?2 AND state='processing'",
            params![modifier, operation.queue_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn open() -> Connection {
        let directory = tempdir().expect("temporary directory");
        let path = directory.keep().join("homeserver.sqlite3");
        initialize(&path).expect("database initialization")
    }

    #[test]
    fn initialization_is_idempotent_and_preserves_installation_identity() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("homeserver.sqlite3");
        let first = initialize(&path).expect("first initialization");
        let first_id = installation_id(&first).expect("first installation id");
        drop(first);
        let second = initialize(&path).expect("second initialization");
        assert_eq!(first_id, installation_id(&second).expect("second installation id"));
        health_check(&second).expect("database health");
    }

    #[test]
    fn cloud_connection_round_trips() {
        let connection = open();
        save_cloud_connection(
            &connection,
            "https://microgifter.com",
            "00000000-0000-4000-8000-000000000001",
            "public",
            &["homeserver.status".to_owned()],
        )
        .expect("save connection");
        let record = cloud_connection(&connection).expect("connection snapshot");
        assert_eq!(record.snapshot.state, CloudConnectionState::Connected);
        assert_eq!(record.snapshot.device_id.as_deref(), Some("00000000-0000-4000-8000-000000000001"));
    }

    #[test]
    fn queue_is_idempotent_and_receipts_finalize_work() {
        let mut connection = open();
        let payload = serde_json::json!({"status": "ready"});
        let first = enqueue_sync(&connection, "test:1", "device.heartbeat", &payload).expect("enqueue");
        let second = enqueue_sync(&connection, "test:1", "device.heartbeat", &payload).expect("replay");
        assert_eq!(first, second);
        assert!(enqueue_sync(&connection, "test:1", "device.heartbeat", &serde_json::json!({"different": true})).is_err());

        let operations = claim_due_sync(&mut connection, 10).expect("claim");
        assert_eq!(operations.len(), 1);
        apply_receipts(
            &mut connection,
            &[ReceiptRecord {
                receipt_id: Uuid::new_v4().to_string(),
                idempotency_key: "test:1".to_owned(),
                operation_type: "device.heartbeat".to_owned(),
                disposition: "accepted".to_owned(),
                reason_code: None,
                response: serde_json::json!({"accepted": true}),
            }],
        )
        .expect("apply receipt");
        assert_eq!(pending_sync_count(&connection).expect("pending count"), 0);
    }
}
