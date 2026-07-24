use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;
use uuid::Uuid;

const INITIAL_MIGRATION: &str = include_str!("../../../database/migrations/0001_initial.sql");
const INITIAL_MIGRATION_KEY: &str = "0001_initial";

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
    ensure!(
        quick_check == "ok",
        "SQLite quick_check returned '{quick_check}'"
    );

    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key = ?1",
        params![INITIAL_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "initial migration is not registered exactly once"
    );

    Ok(())
}

pub fn pending_sync_count(connection: &Connection) -> Result<u64> {
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sync_queue WHERE state = 'pending'",
        [],
        |row| row.get(0),
    )?;
    Ok(count.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn installation_id(connection: &Connection) -> String {
        connection
            .query_row(
                "SELECT setting_value FROM homeserver_settings WHERE setting_key = 'installation_id'",
                [],
                |row| row.get(0),
            )
            .expect("installation id should exist")
    }

    #[test]
    fn initialization_is_idempotent_and_preserves_installation_identity() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("homeserver.sqlite3");

        let first = initialize(&path).expect("first initialization");
        let first_id = installation_id(&first);
        health_check(&first).expect("first database health check");
        drop(first);

        let second = initialize(&path).expect("second initialization");
        let second_id = installation_id(&second);
        health_check(&second).expect("second database health check");

        assert_eq!(first_id, second_id);
        let migration_count: i64 = second
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE migration_key = '0001_initial'",
                [],
                |row| row.get(0),
            )
            .expect("migration count");
        assert_eq!(migration_count, 1);
    }

    #[test]
    fn pending_sync_count_tracks_only_pending_work() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("homeserver.sqlite3");
        let connection = initialize(&path).expect("database initialization");

        connection
            .execute(
                "INSERT INTO sync_queue (idempotency_key, operation_type, payload_json, state) VALUES (?1, 'test.pending', '{}', 'pending')",
                params![Uuid::new_v4().to_string()],
            )
            .expect("pending insert");
        connection
            .execute(
                "INSERT INTO sync_queue (idempotency_key, operation_type, payload_json, state) VALUES (?1, 'test.accepted', '{}', 'accepted')",
                params![Uuid::new_v4().to_string()],
            )
            .expect("accepted insert");

        assert_eq!(pending_sync_count(&connection).expect("pending count"), 1);
    }
}
