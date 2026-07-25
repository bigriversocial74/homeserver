use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Duration, Utc};
use microgifter_homeserver_core::{BackupCatalog, BackupKind, BackupRecord, BackupState};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const INITIAL_MIGRATION: &str = include_str!("../../../database/migrations/0001_initial.sql");
const BACKUP_MIGRATION: &str =
    include_str!("../../../database/migrations/0002_backup_recovery.sql");
const INITIAL_MIGRATION_KEY: &str = "0001_initial";
const BACKUP_MIGRATION_KEY: &str = "0002_backup_recovery";

pub fn initialize(path: &Path) -> Result<Connection> {
    let mut connection = Connection::open(path)
        .with_context(|| format!("unable to open database at {}", path.display()))?;

    configure_connection(&connection)?;

    let transaction = connection.transaction()?;
    transaction.execute_batch(INITIAL_MIGRATION)?;
    transaction.execute_batch(BACKUP_MIGRATION)?;
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
    maintain_history(&connection)?;
    Ok(connection)
}

pub fn configure_connection(connection: &Connection) -> Result<()> {
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "wal_autocheckpoint", 1_000_i64)?;
    connection.pragma_update(None, "journal_size_limit", 64_i64 * 1024 * 1024)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "trusted_schema", "OFF")?;
    connection.pragma_update(None, "busy_timeout", 5_000_i64)?;
    Ok(())
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let quick_check: String =
        connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    ensure!(
        quick_check == "ok",
        "SQLite quick_check returned '{quick_check}'"
    );

    for migration in [INITIAL_MIGRATION_KEY, BACKUP_MIGRATION_KEY] {
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key = ?1",
            params![migration],
            |row| row.get(0),
        )?;
        ensure!(
            count == 1,
            "migration '{migration}' is not registered exactly once"
        );
    }

    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM service_events WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-90 days')",
        [],
    )?;
    transaction.execute(
        "DELETE FROM service_events WHERE event_id NOT IN (SELECT event_id FROM service_events ORDER BY created_at_utc DESC,event_id DESC LIMIT 5000)",
        [],
    )?;
    transaction.execute(
        "DELETE FROM restore_requests WHERE state IN ('applied','rolled_back','failed','cancelled') AND restore_id NOT IN (SELECT restore_id FROM restore_requests WHERE state IN ('applied','rolled_back','failed','cancelled') ORDER BY updated_at_utc DESC,restore_id DESC LIMIT 1000)",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn delete_restore_request(connection: &Connection, restore_id: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM restore_requests WHERE restore_id=?1 AND state IN ('staging','staged')",
        params![restore_id],
    )?;
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

pub fn backup_settings(connection: &Connection) -> Result<(u32, u32)> {
    Ok((
        setting_u32(connection, "backup_retention_count", 14, 1, 365)?,
        setting_u32(connection, "backup_interval_hours", 24, 1, 168)?,
    ))
}

fn setting_u32(
    connection: &Connection,
    key: &str,
    default: u32,
    minimum: u32,
    maximum: u32,
) -> Result<u32> {
    let value = connection
        .query_row(
            "SELECT setting_value FROM homeserver_settings WHERE setting_key=?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(default);
    Ok(value.clamp(minimum, maximum))
}

pub fn insert_backup_creating(
    connection: &Connection,
    backup_id: &str,
    kind: &BackupKind,
    encryption: &str,
    file_name: &str,
    storage_path: &Path,
    note: Option<&str>,
) -> Result<()> {
    connection.execute(
        "INSERT INTO backup_records (backup_id,kind,encryption,state,file_name,storage_path,note) VALUES (?1,?2,?3,'creating',?4,?5,?6)",
        params![
            backup_id,
            kind.as_str(),
            encryption,
            file_name,
            storage_path.to_string_lossy(),
            note.map(|value| value.chars().take(500).collect::<String>()),
        ],
    )?;
    Ok(())
}

pub fn mark_backup_ready(
    connection: &Connection,
    backup_id: &str,
    size_bytes: u64,
    archive_sha256: &str,
    database_sha256: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE backup_records SET state='ready',size_bytes=?1,archive_sha256=?2,database_sha256=?3,failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE backup_id=?4",
        params![size_bytes as i64, archive_sha256, database_sha256, backup_id],
    )?;
    Ok(())
}

pub fn mark_backup_verified(connection: &Connection, backup_id: &str) -> Result<()> {
    connection.execute(
        "UPDATE backup_records SET state='verified',verified_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE backup_id=?1",
        params![backup_id],
    )?;
    Ok(())
}

pub fn mark_backup_failed(
    connection: &Connection,
    backup_id: &str,
    failure_code: &str,
) -> Result<()> {
    connection.execute(
        "UPDATE backup_records SET state='failed',failure_code=?1,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE backup_id=?2",
        params![failure_code.chars().take(120).collect::<String>(), backup_id],
    )?;
    Ok(())
}

pub fn mark_backup_restore_staged(connection: &Connection, backup_id: &str) -> Result<()> {
    connection.execute(
        "UPDATE backup_records SET state='restore_staged',failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE backup_id=?1",
        params![backup_id],
    )?;
    Ok(())
}

pub fn create_restore_request(
    connection: &Connection,
    restore_id: &str,
    backup_id: &str,
    pending_database_path: &Path,
    confirmation: &str,
) -> Result<()> {
    connection.execute(
        "INSERT INTO restore_requests (restore_id,backup_id,state,pending_database_path,confirmation) VALUES (?1,?2,'staged',?3,?4)",
        params![
            restore_id,
            backup_id,
            pending_database_path.to_string_lossy(),
            confirmation,
        ],
    )?;
    Ok(())
}

pub fn record_restore_applied(
    connection: &Connection,
    restore_id: &str,
    backup_id: &str,
    rollback_database_path: Option<&Path>,
) -> Result<()> {
    connection.execute(
        "INSERT INTO restore_requests (restore_id,backup_id,state,confirmation,rollback_database_path,applied_at_utc,updated_at_utc) VALUES (?1,?2,'applied','RESTORE',?3,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(restore_id) DO UPDATE SET state='applied',rollback_database_path=excluded.rollback_database_path,applied_at_utc=excluded.applied_at_utc,failure_code=NULL,updated_at_utc=excluded.updated_at_utc",
        params![
            restore_id,
            backup_id,
            rollback_database_path.map(|path| path.to_string_lossy().to_string()),
        ],
    )?;
    connection.execute(
        "UPDATE backup_records SET state='restored',restored_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE backup_id=?1",
        params![backup_id],
    )?;
    connection.execute(
        "INSERT INTO service_events (event_type,message,metadata_json) VALUES ('backup.restore_applied','A staged HomeServer restore was applied',json_object('restore_id',?1,'backup_id',?2))",
        params![restore_id, backup_id],
    )?;
    Ok(())
}

pub fn record_restore_rolled_back(
    connection: &Connection,
    restore_id: &str,
    backup_id: &str,
    failure_code: &str,
) -> Result<()> {
    connection.execute(
        "INSERT INTO restore_requests (restore_id,backup_id,state,confirmation,failure_code,updated_at_utc) VALUES (?1,?2,'rolled_back','RESTORE',?3,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(restore_id) DO UPDATE SET state='rolled_back',failure_code=excluded.failure_code,updated_at_utc=excluded.updated_at_utc",
        params![restore_id, backup_id, failure_code],
    )?;
    Ok(())
}

pub fn backup_by_id(connection: &Connection, backup_id: &str) -> Result<BackupRecord> {
    connection
        .query_row(
            "SELECT backup_id,kind,state,encryption,file_name,storage_path,size_bytes,archive_sha256,database_sha256,note,created_at_utc,verified_at_utc,restored_at_utc,failure_code FROM backup_records WHERE backup_id=?1",
            params![backup_id],
            backup_record_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("backup was not found"))
}

pub fn backup_catalog(connection: &Connection, restore_pending: bool) -> Result<BackupCatalog> {
    let (retention_count, interval_hours) = backup_settings(connection)?;
    let mut statement = connection.prepare(
        "SELECT backup_id,kind,state,encryption,file_name,storage_path,size_bytes,archive_sha256,database_sha256,note,created_at_utc,verified_at_utc,restored_at_utc,failure_code FROM backup_records ORDER BY created_at_utc DESC,backup_id DESC LIMIT 100",
    )?;
    let backups = statement
        .query_map([], backup_record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let last_automatic_backup_utc = connection
        .query_row(
            "SELECT created_at_utc FROM backup_records WHERE kind='automatic' AND state IN ('ready','verified','restored') ORDER BY created_at_utc DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(parse_utc)
        .transpose()?;

    Ok(BackupCatalog {
        backups,
        retention_count,
        interval_hours,
        last_automatic_backup_utc,
        restore_pending,
    })
}

pub fn automatic_backup_due(connection: &Connection, now: DateTime<Utc>) -> Result<bool> {
    let (_, interval_hours) = backup_settings(connection)?;
    let last = connection
        .query_row(
            "SELECT created_at_utc FROM backup_records WHERE kind='automatic' AND state IN ('ready','verified','restored') ORDER BY created_at_utc DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(parse_utc)
        .transpose()?;
    Ok(last
        .map(|value| now - value >= Duration::hours(interval_hours as i64))
        .unwrap_or(true))
}

pub fn retention_candidates(connection: &Connection) -> Result<Vec<(String, PathBuf)>> {
    let (retention_count, _) = backup_settings(connection)?;
    let mut statement = connection.prepare(
        "SELECT backup_id,storage_path FROM backup_records WHERE kind='automatic' AND state IN ('ready','verified','restored','failed') ORDER BY created_at_utc DESC,backup_id DESC LIMIT -1 OFFSET ?1",
    )?;
    let rows = statement
        .query_map(params![retention_count as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn unreferenced_pre_update_backups(
    connection: &Connection,
) -> Result<Vec<(String, PathBuf)>> {
    let mut statement = connection.prepare(
        "SELECT backup_id,storage_path FROM backup_records WHERE kind='pre_update' AND state IN ('ready','verified','restored','failed') AND backup_id NOT IN (SELECT pre_update_backup_id FROM update_records WHERE pre_update_backup_id IS NOT NULL) ORDER BY created_at_utc DESC,backup_id DESC",
    )?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn delete_backup_record(connection: &Connection, backup_id: &str) -> Result<()> {
    let staged: i64 = connection.query_row(
        "SELECT COUNT(*) FROM restore_requests WHERE backup_id=?1 AND state IN ('staging','staged','applying')",
        params![backup_id],
        |row| row.get(0),
    )?;
    if staged > 0 {
        bail!("backup is referenced by a pending restore");
    }
    connection.execute(
        "DELETE FROM backup_records WHERE backup_id=?1",
        params![backup_id],
    )?;
    Ok(())
}

fn backup_record_from_row(row: &Row<'_>) -> rusqlite::Result<BackupRecord> {
    let kind_value = row.get::<_, String>(1)?;
    let state_value = row.get::<_, String>(2)?;
    let created = row.get::<_, String>(10)?;
    let verified = row.get::<_, Option<String>>(11)?;
    let restored = row.get::<_, Option<String>>(12)?;

    Ok(BackupRecord {
        backup_id: row.get(0)?,
        kind: parse_backup_kind(&kind_value).map_err(to_sql_error)?,
        state: parse_backup_state(&state_value).map_err(to_sql_error)?,
        encryption: row.get(3)?,
        file_name: row.get(4)?,
        storage_path: row.get(5)?,
        size_bytes: row.get::<_, i64>(6)?.max(0) as u64,
        archive_sha256: row.get(7)?,
        database_sha256: row.get(8)?,
        note: row.get(9)?,
        created_at_utc: parse_utc(created).map_err(to_sql_error)?,
        verified_at_utc: verified.map(parse_utc).transpose().map_err(to_sql_error)?,
        restored_at_utc: restored.map(parse_utc).transpose().map_err(to_sql_error)?,
        failure_code: row.get(13)?,
    })
}

fn parse_backup_kind(value: &str) -> Result<BackupKind> {
    match value {
        "automatic" => Ok(BackupKind::Automatic),
        "manual" => Ok(BackupKind::Manual),
        "recovery" => Ok(BackupKind::Recovery),
        "pre_update" => Ok(BackupKind::PreUpdate),
        _ => bail!("unsupported backup kind '{value}'"),
    }
}

fn parse_backup_state(value: &str) -> Result<BackupState> {
    match value {
        "creating" => Ok(BackupState::Creating),
        "ready" => Ok(BackupState::Ready),
        "verified" => Ok(BackupState::Verified),
        "restore_staged" => Ok(BackupState::RestoreStaged),
        "restored" => Ok(BackupState::Restored),
        "failed" => Ok(BackupState::Failed),
        _ => bail!("unsupported backup state '{value}'"),
    }
}

fn parse_utc(value: String) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|date| date.with_timezone(&Utc))
        .with_context(|| format!("invalid UTC timestamp '{value}'"))
}

fn to_sql_error(error: anyhow::Error) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.to_string(),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn initialization_is_idempotent_and_preserves_installation_identity() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("homeserver.sqlite3");

        let first = initialize(&path).expect("first initialization");
        let first_id = installation_id(&first).expect("first installation id");
        health_check(&first).expect("first database health check");
        let synchronous: i64 = first
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .expect("synchronous pragma");
        assert_eq!(synchronous, 2);
        drop(first);

        let second = initialize(&path).expect("second initialization");
        let second_id = installation_id(&second).expect("second installation id");
        health_check(&second).expect("second database health check");

        assert_eq!(first_id, second_id);
        for migration in [INITIAL_MIGRATION_KEY, BACKUP_MIGRATION_KEY] {
            let migration_count: i64 = second
                .query_row(
                    "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
                    params![migration],
                    |row| row.get(0),
                )
                .expect("migration count");
            assert_eq!(migration_count, 1);
        }
    }

    #[test]
    fn backup_catalog_round_trips_typed_records() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("homeserver.sqlite3");
        let connection = initialize(&path).expect("database initialization");
        let backup_path = directory.path().join("backup.mghbackup");
        insert_backup_creating(
            &connection,
            "backup-1",
            &BackupKind::Manual,
            "device_key_aes256gcm",
            "backup.mghbackup",
            &backup_path,
            Some("test"),
        )
        .expect("insert backup");
        mark_backup_ready(&connection, "backup-1", 100, "archive", "database").expect("mark ready");

        let catalog = backup_catalog(&connection, false).expect("catalog");
        assert_eq!(catalog.backups.len(), 1);
        assert_eq!(catalog.backups[0].kind, BackupKind::Manual);
        assert_eq!(catalog.backups[0].state, BackupState::Ready);
    }

    #[test]
    fn pending_sync_count_tracks_active_work() {
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
                "INSERT INTO sync_queue (idempotency_key, operation_type, payload_json, state) VALUES (?1, 'test.processing', '{}', 'processing')",
                params![Uuid::new_v4().to_string()],
            )
            .expect("processing insert");
        connection
            .execute(
                "INSERT INTO sync_queue (idempotency_key, operation_type, payload_json, state) VALUES (?1, 'test.accepted', '{}', 'accepted')",
                params![Uuid::new_v4().to_string()],
            )
            .expect("accepted insert");

        assert_eq!(pending_sync_count(&connection).expect("pending count"), 2);
    }
}
