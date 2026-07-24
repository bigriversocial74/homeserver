use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use microgifter_homeserver_core::{
    SignedUpdateManifest, UpdateApplicationResult, UpdateChannel, UpdateRecord, UpdateState,
    UpdateStatus,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};

const UPDATE_MIGRATION: &str = include_str!("../../../database/migrations/0003_signed_updates.sql");
const UPDATE_MIGRATION_KEY: &str = "0003_signed_updates";

#[derive(Debug, Clone)]
pub struct StoredUpdate {
    pub record: UpdateRecord,
    pub manifest: SignedUpdateManifest,
    pub installer_path: Option<PathBuf>,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(UPDATE_MIGRATION)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![UPDATE_MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "signed update migration is not registered exactly once"
    );
    let runtime_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM update_runtime WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        runtime_count == 1,
        "signed update runtime state is unavailable"
    );
    Ok(())
}

pub fn status(
    connection: &Connection,
    manifest_url: &str,
    apply_pending: bool,
) -> Result<UpdateStatus> {
    let (state_value, checked, last_error) = connection.query_row(
        "SELECT state,last_checked_at_utc,last_error FROM update_runtime WHERE singleton_id=1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;
    let update = latest_update(connection)?.map(|stored| stored.record);
    Ok(UpdateStatus {
        current_version: env!("CARGO_PKG_VERSION").to_owned(),
        channel: UpdateChannel::Stable,
        state: parse_update_state(&state_value)?,
        manifest_url: manifest_url.to_owned(),
        update,
        apply_pending,
        last_checked_at_utc: checked.map(parse_utc).transpose()?,
        last_error,
    })
}

pub fn begin_check(connection: &Connection) -> Result<()> {
    set_runtime(connection, UpdateState::Checking, None, false)
}

pub fn record_current(connection: &Connection) -> Result<()> {
    set_runtime(connection, UpdateState::Current, None, true)?;
    record_event(
        connection,
        None,
        "update.current",
        "HomeServer is running the current stable version",
        "{}",
    )
}

pub fn record_check_failure(connection: &Connection, failure_code: &str) -> Result<()> {
    set_runtime(
        connection,
        UpdateState::Failed,
        Some(&bounded(failure_code, 120)),
        true,
    )
}

pub fn save_available(
    connection: &Connection,
    update_id: &str,
    manifest_url: &str,
    manifest: &SignedUpdateManifest,
) -> Result<StoredUpdate> {
    let payload = &manifest.payload;
    let manifest_json = serde_json::to_string(manifest)?;
    connection.execute(
        "INSERT INTO update_records (update_id,version,channel,state,manifest_url,manifest_json,release_notes,installer_url,installer_file_name,installer_size_bytes,installer_sha256,authenticode_thumbprint,checked_at_utc,updated_at_utc)
         VALUES (?1,?2,'stable','available',?3,?4,?5,?6,?7,?8,?9,?10,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(update_id) DO UPDATE SET version=excluded.version,channel=excluded.channel,state='available',manifest_url=excluded.manifest_url,manifest_json=excluded.manifest_json,release_notes=excluded.release_notes,installer_url=excluded.installer_url,installer_file_name=excluded.installer_file_name,installer_size_bytes=excluded.installer_size_bytes,installer_sha256=excluded.installer_sha256,authenticode_thumbprint=excluded.authenticode_thumbprint,checked_at_utc=excluded.checked_at_utc,failure_code=NULL,updated_at_utc=excluded.updated_at_utc",
        params![
            update_id,
            payload.version,
            manifest_url,
            manifest_json,
            bounded(&payload.release_notes, 20_000),
            payload.installer.url,
            payload.installer.file_name,
            payload.installer.size_bytes as i64,
            payload.installer.sha256.to_lowercase(),
            payload.installer.authenticode_thumbprint.to_uppercase(),
        ],
    )?;
    set_runtime(connection, UpdateState::Available, None, true)?;
    record_event(
        connection,
        Some(update_id),
        "update.available",
        "A verified HomeServer update is available",
        &serde_json::json!({"version": payload.version}).to_string(),
    )?;
    update_by_id(connection, update_id)
}

pub fn mark_downloading(connection: &Connection, update_id: &str) -> Result<StoredUpdate> {
    set_record_state(connection, update_id, UpdateState::Downloading, None)?;
    set_runtime(connection, UpdateState::Downloading, None, false)?;
    update_by_id(connection, update_id)
}

pub fn mark_staged(
    connection: &Connection,
    update_id: &str,
    installer_path: &Path,
) -> Result<StoredUpdate> {
    connection.execute(
        "UPDATE update_records SET state='staged',installer_path=?1,downloaded_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE update_id=?2",
        params![installer_path.to_string_lossy(), update_id],
    )?;
    set_runtime(connection, UpdateState::Staged, None, false)?;
    record_event(
        connection,
        Some(update_id),
        "update.staged",
        "A signed HomeServer update was downloaded and staged",
        "{}",
    )?;
    update_by_id(connection, update_id)
}

pub fn mark_applying(
    connection: &Connection,
    update_id: &str,
    rollback_path: &Path,
) -> Result<StoredUpdate> {
    connection.execute(
        "UPDATE update_records SET state='applying',rollback_path=?1,failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE update_id=?2 AND state='staged'",
        params![rollback_path.to_string_lossy(), update_id],
    )?;
    set_runtime(connection, UpdateState::Applying, None, false)?;
    record_event(
        connection,
        Some(update_id),
        "update.applying",
        "The HomeServer updater helper was launched",
        "{}",
    )?;
    update_by_id(connection, update_id)
}

pub fn mark_failure(connection: &Connection, update_id: &str, failure_code: &str) -> Result<()> {
    set_record_state(
        connection,
        update_id,
        UpdateState::Failed,
        Some(&bounded(failure_code, 120)),
    )?;
    set_runtime(
        connection,
        UpdateState::Failed,
        Some(&bounded(failure_code, 120)),
        false,
    )?;
    record_event(
        connection,
        Some(update_id),
        "update.failed",
        "HomeServer update processing failed",
        &serde_json::json!({"failure_code": failure_code}).to_string(),
    )
}

pub fn record_application_result(
    connection: &Connection,
    result: &UpdateApplicationResult,
) -> Result<()> {
    ensure!(
        matches!(
            result.state,
            UpdateState::Succeeded | UpdateState::RolledBack | UpdateState::Failed
        ),
        "update application result has an unsupported state"
    );
    let state = result.state.as_str();
    connection.execute(
        "UPDATE update_records SET state=?1,applied_at_utc=CASE WHEN ?1='succeeded' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE applied_at_utc END,failure_code=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE update_id=?3",
        params![state, result.failure_code, result.update_id],
    )?;
    set_runtime(
        connection,
        result.state.clone(),
        result.failure_code.as_deref(),
        false,
    )?;
    record_event(
        connection,
        Some(&result.update_id),
        match result.state {
            UpdateState::Succeeded => "update.succeeded",
            UpdateState::RolledBack => "update.rolled_back",
            _ => "update.failed",
        },
        &result.message,
        &serde_json::json!({
            "target_version": result.target_version,
            "failure_code": result.failure_code,
        })
        .to_string(),
    )
}

pub fn latest_update(connection: &Connection) -> Result<Option<StoredUpdate>> {
    connection
        .query_row(
            "SELECT update_id,version,channel,state,manifest_json,release_notes,installer_file_name,installer_path,installer_size_bytes,installer_sha256,authenticode_thumbprint,checked_at_utc,downloaded_at_utc,applied_at_utc,failure_code FROM update_records ORDER BY updated_at_utc DESC,created_at_utc DESC LIMIT 1",
            [],
            stored_update_from_row,
        )
        .optional()
        .map_err(Into::into)
}

pub fn update_by_id(connection: &Connection, update_id: &str) -> Result<StoredUpdate> {
    connection
        .query_row(
            "SELECT update_id,version,channel,state,manifest_json,release_notes,installer_file_name,installer_path,installer_size_bytes,installer_sha256,authenticode_thumbprint,checked_at_utc,downloaded_at_utc,applied_at_utc,failure_code FROM update_records WHERE update_id=?1",
            params![update_id],
            stored_update_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("update was not found"))
}

pub fn latest_in_state(connection: &Connection, state: UpdateState) -> Result<StoredUpdate> {
    connection
        .query_row(
            "SELECT update_id,version,channel,state,manifest_json,release_notes,installer_file_name,installer_path,installer_size_bytes,installer_sha256,authenticode_thumbprint,checked_at_utc,downloaded_at_utc,applied_at_utc,failure_code FROM update_records WHERE state=?1 ORDER BY updated_at_utc DESC LIMIT 1",
            params![state.as_str()],
            stored_update_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("no {} update is available", state.as_str()))
}

fn set_runtime(
    connection: &Connection,
    state: UpdateState,
    last_error: Option<&str>,
    checked_now: bool,
) -> Result<()> {
    connection.execute(
        "UPDATE update_runtime SET state=?1,last_checked_at_utc=CASE WHEN ?2=1 THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE last_checked_at_utc END,last_error=?3,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![state.as_str(), if checked_now { 1 } else { 0 }, last_error],
    )?;
    Ok(())
}

fn set_record_state(
    connection: &Connection,
    update_id: &str,
    state: UpdateState,
    failure_code: Option<&str>,
) -> Result<()> {
    let changed = connection.execute(
        "UPDATE update_records SET state=?1,failure_code=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE update_id=?3",
        params![state.as_str(), failure_code, update_id],
    )?;
    ensure!(changed == 1, "update was not found");
    Ok(())
}

fn record_event(
    connection: &Connection,
    update_id: Option<&str>,
    event_type: &str,
    message: &str,
    metadata_json: &str,
) -> Result<()> {
    connection.execute(
        "INSERT INTO update_events (update_id,event_type,message,metadata_json) VALUES (?1,?2,?3,?4)",
        params![
            update_id,
            bounded(event_type, 100),
            bounded(message, 500),
            bounded(metadata_json, 20_000),
        ],
    )?;
    Ok(())
}

fn stored_update_from_row(row: &Row<'_>) -> rusqlite::Result<StoredUpdate> {
    let channel = parse_update_channel(&row.get::<_, String>(2)?).map_err(to_sql_error)?;
    let state = parse_update_state(&row.get::<_, String>(3)?).map_err(to_sql_error)?;
    let manifest_json = row.get::<_, String>(4)?;
    let manifest = serde_json::from_str::<SignedUpdateManifest>(&manifest_json)
        .map_err(|error| to_sql_error(error.into()))?;
    let checked = parse_utc(row.get::<_, String>(11)?).map_err(to_sql_error)?;
    let downloaded = row
        .get::<_, Option<String>>(12)?
        .map(parse_utc)
        .transpose()
        .map_err(to_sql_error)?;
    let applied = row
        .get::<_, Option<String>>(13)?
        .map(parse_utc)
        .transpose()
        .map_err(to_sql_error)?;
    let installer_path = row.get::<_, Option<String>>(7)?.map(PathBuf::from);
    Ok(StoredUpdate {
        record: UpdateRecord {
            update_id: row.get(0)?,
            version: row.get(1)?,
            channel,
            state,
            release_notes: row.get(5)?,
            installer_file_name: row.get(6)?,
            installer_size_bytes: row.get::<_, i64>(8)?.max(0) as u64,
            installer_sha256: row.get(9)?,
            authenticode_thumbprint: row.get(10)?,
            checked_at_utc: checked,
            downloaded_at_utc: downloaded,
            applied_at_utc: applied,
            failure_code: row.get(14)?,
        },
        manifest,
        installer_path,
    })
}

fn parse_update_channel(value: &str) -> Result<UpdateChannel> {
    match value {
        "stable" => Ok(UpdateChannel::Stable),
        _ => bail!("unsupported update channel '{value}'"),
    }
}

fn parse_update_state(value: &str) -> Result<UpdateState> {
    match value {
        "idle" => Ok(UpdateState::Idle),
        "checking" => Ok(UpdateState::Checking),
        "current" => Ok(UpdateState::Current),
        "available" => Ok(UpdateState::Available),
        "downloading" => Ok(UpdateState::Downloading),
        "staged" => Ok(UpdateState::Staged),
        "applying" => Ok(UpdateState::Applying),
        "succeeded" => Ok(UpdateState::Succeeded),
        "failed" => Ok(UpdateState::Failed),
        "rolled_back" => Ok(UpdateState::RolledBack),
        _ => bail!("unsupported update state '{value}'"),
    }
}

fn parse_utc(value: String) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|date| date.with_timezone(&Utc))
        .with_context(|| format!("invalid UTC timestamp '{value}'"))
}

fn bounded(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
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
    use crate::database;
    use microgifter_homeserver_core::{
        UpdateInstallerContract, UpdateManifestPayload, UPDATE_KEY_ID,
        UPDATE_MANIFEST_SCHEMA_VERSION,
    };
    use tempfile::tempdir;

    fn manifest() -> SignedUpdateManifest {
        SignedUpdateManifest {
            key_id: UPDATE_KEY_ID.to_owned(),
            payload: UpdateManifestPayload {
                schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
                product: "Microgifter HomeServer".to_owned(),
                channel: UpdateChannel::Stable,
                version: "0.2.0".to_owned(),
                minimum_version: Some("0.1.0".to_owned()),
                published_at_utc: Utc::now(),
                release_notes: "Test release".to_owned(),
                installer: UpdateInstallerContract {
                    url: "https://updates.microgifter.com/setup.exe".to_owned(),
                    file_name: "Microgifter-HomeServer-Setup.exe".to_owned(),
                    size_bytes: 5_000_000,
                    sha256: "a".repeat(64),
                    authenticode_thumbprint: "b".repeat(40),
                },
            },
            signature: "test".to_owned(),
        }
    }

    #[test]
    fn update_migration_and_state_are_idempotent() {
        let directory = tempdir().unwrap();
        let connection =
            database::initialize(&directory.path().join("homeserver.sqlite3")).unwrap();
        initialize(&connection).unwrap();
        initialize(&connection).unwrap();
        health_check(&connection).unwrap();
        let status = status(
            &connection,
            "https://updates.microgifter.com/manifest.json",
            false,
        )
        .unwrap();
        assert_eq!(status.state, UpdateState::Idle);
    }

    #[test]
    fn available_update_round_trips_signed_manifest_and_staged_path() {
        let directory = tempdir().unwrap();
        let connection =
            database::initialize(&directory.path().join("homeserver.sqlite3")).unwrap();
        initialize(&connection).unwrap();
        let stored = save_available(
            &connection,
            "update:0.2.0:aaaaaaaaaaaaaaaa",
            "https://updates.microgifter.com/manifest.json",
            &manifest(),
        )
        .unwrap();
        assert_eq!(stored.record.state, UpdateState::Available);
        let staged_path = directory.path().join("setup.exe");
        let staged = mark_staged(&connection, &stored.record.update_id, &staged_path).unwrap();
        assert_eq!(staged.record.state, UpdateState::Staged);
        assert_eq!(
            staged.installer_path.as_deref(),
            Some(staged_path.as_path())
        );
    }
}
