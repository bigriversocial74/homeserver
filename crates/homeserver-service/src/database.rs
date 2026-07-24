use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use uuid::Uuid;

const INITIAL_MIGRATION: &str = include_str!("../../../database/migrations/0001_initial.sql");

pub fn initialize(path: &Path) -> Result<Connection> {
    let mut connection = Connection::open(path)
        .with_context(|| format!("unable to open database at {}", path.display()))?;

    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "busy_timeout", 5_000_i64)?;

    let transaction = connection.transaction()?;
    transaction.execute_batch(INITIAL_MIGRATION)?;
    transaction.execute(
        "INSERT OR IGNORE INTO homeserver_settings (setting_key, setting_value) VALUES ('installation_id', ?1)",
        params![Uuid::new_v4().to_string()],
    )?;
    transaction.execute(
        "INSERT INTO service_events (event_type, message) VALUES ('service.database_ready', 'Local database initialized')",
        [],
    )?;
    transaction.commit()?;

    Ok(connection)
}

pub fn pending_sync_count(connection: &Connection) -> Result<u64> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sync_queue WHERE state = 'pending'",
        [],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}
