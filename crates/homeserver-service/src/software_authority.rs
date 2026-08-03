use crate::{microgifter_connection, AppState};
use anyhow::{ensure, Context, Result};
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

const VP3_MIGRATION: &str =
    include_str!("../../../database/migrations/0017_vp3_software_authority.sql");
const VP3_MIGRATION_KEY: &str = "0017_vp3_software_authority";
const MICROGIFTER_PRIMARY_MIGRATION: &str =
    include_str!("../../../database/migrations/0033_microgifter_primary_software_authority.sql");
const MICROGIFTER_PRIMARY_MIGRATION_KEY: &str = "0033_microgifter_primary_software_authority";
const MICROGIFTER_AUTHORITY: &str = "microgifter";
const LEGACY_RECEIPT_AUTHORITY_KEY: &str = "microgifter_legacy";

#[derive(Debug, Clone, Serialize)]
pub struct SoftwareAuthoritySnapshot {
    pub current_authority: String,
    pub target_authority: String,
    pub cutover_state: String,
    pub primary_provider_key: String,
    pub microgifter_primary_active: bool,
    pub vp3_optional: bool,
    pub vp3_device_id: Option<String>,
    pub vp3_license_id: Option<String>,
    pub vp3_lease_id: Option<String>,
    pub vp3_lease_expires_at_utc: Option<String>,
    pub update_eligible: bool,
    pub allowed_update_channels: Vec<String>,
    pub last_vp3_heartbeat_at_utc: Option<String>,
    pub last_error_code: Option<String>,
    pub legacy_microgifter_fallback_active: bool,
}

#[derive(Debug, Default)]
struct LegacyVp3AuthorityState {
    device_id: Option<String>,
    license_id: Option<String>,
    lease_id: Option<String>,
    lease_expires_at_utc: Option<String>,
    last_heartbeat_at_utc: Option<String>,
    last_error_code: Option<String>,
}

#[derive(Debug, Serialize)]
struct AuthorityApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<AuthorityApiError>)>;

pub fn initialize(connection: &Connection) -> Result<()> {
    // Keep the historical VP3 migration because installed databases may already
    // contain its audit state. The additive migration below restores Microgifter
    // as the primary authority without deleting VP3 history or wrapper records.
    connection.execute_batch(VP3_MIGRATION)?;
    connection.execute_batch(MICROGIFTER_PRIMARY_MIGRATION)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    for migration_key in [VP3_MIGRATION_KEY, MICROGIFTER_PRIMARY_MIGRATION_KEY] {
        let migration_count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
            params![migration_key],
            |row| row.get(0),
        )?;
        ensure!(
            migration_count == 1,
            "software-authority migration '{migration_key}' is not registered exactly once"
        );
    }

    let primary_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM homeserver_primary_authority WHERE singleton_id=1 AND provider_key='microgifter'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        primary_count == 1,
        "Microgifter primary software-authority state is unavailable"
    );

    let _: i64 = connection.query_row(
        "SELECT COUNT(*) FROM software_authority_receipts",
        [],
        |row| row.get(0),
    )?;
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM software_authority_receipts WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    transaction.execute(
        "DELETE FROM software_authority_receipts WHERE receipt_id NOT IN (SELECT receipt_id FROM software_authority_receipts ORDER BY created_at_utc DESC,receipt_id DESC LIMIT 5000)",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/software-authority/status", get(status_handler))
        .with_state(state)
}

async fn status_handler(
    State(state): State<Arc<AppState>>,
) -> ApiResult<SoftwareAuthoritySnapshot> {
    tokio::task::spawn_blocking(move || status_snapshot(&*state.connection()?))
        .await
        .map_err(|error| internal_error("software_authority_task_failed", error))?
        .map(Json)
        .map_err(|error| internal_error("software_authority_status_failed", error))
}

pub fn status_snapshot(connection: &Connection) -> Result<SoftwareAuthoritySnapshot> {
    let (provider_key, authority_state, migrated_from): (String, String, Option<String>) =
        connection
            .query_row(
                "SELECT provider_key,state,migrated_from FROM homeserver_primary_authority WHERE singleton_id=1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .context("unable to load primary HomeServer software-authority state")?;

    let legacy_vp3 = connection
        .query_row(
            "SELECT vp3_device_id,vp3_license_id,vp3_lease_id,vp3_lease_expires_at_utc,last_vp3_heartbeat_at_utc,last_error_code FROM homeserver_software_authority WHERE singleton_id=1",
            [],
            |row| {
                Ok(LegacyVp3AuthorityState {
                    device_id: row.get(0)?,
                    license_id: row.get(1)?,
                    lease_id: row.get(2)?,
                    lease_expires_at_utc: row.get(3)?,
                    last_heartbeat_at_utc: row.get(4)?,
                    last_error_code: row.get(5)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default();

    let (update_eligible, allowed_update_channels) = microgifter_update_status(connection)?;
    let vp3_optional = migrated_from.as_deref() == Some("vp3") || legacy_vp3.device_id.is_some();

    Ok(SoftwareAuthoritySnapshot {
        current_authority: provider_key.clone(),
        target_authority: MICROGIFTER_AUTHORITY.to_owned(),
        cutover_state: authority_state.clone(),
        primary_provider_key: provider_key.clone(),
        microgifter_primary_active: provider_key == MICROGIFTER_AUTHORITY
            && authority_state == "active",
        vp3_optional,
        vp3_device_id: legacy_vp3.device_id,
        vp3_license_id: legacy_vp3.license_id,
        vp3_lease_id: legacy_vp3.lease_id,
        vp3_lease_expires_at_utc: legacy_vp3.lease_expires_at_utc,
        update_eligible,
        allowed_update_channels,
        last_vp3_heartbeat_at_utc: legacy_vp3.last_heartbeat_at_utc,
        last_error_code: legacy_vp3.last_error_code,
        legacy_microgifter_fallback_active: false,
    })
}

fn microgifter_update_status(connection: &Connection) -> Result<(bool, Vec<String>)> {
    if !table_exists(connection, "provider_connection_profiles")?
        || !table_exists(connection, "cloud_connections")?
        || !table_exists(connection, "provider_entitlement_leases")?
    {
        return Ok((false, Vec::new()));
    }

    let row: Option<(i64, Option<String>, String, String)> = connection
        .query_row(
            "SELECT p.update_eligible,p.entitlement_expires_at_utc,p.lifecycle_state,COALESCE(l.allowed_update_channels_json,'[]') FROM provider_connection_profiles p JOIN cloud_connections c ON c.connection_id=p.connection_id LEFT JOIN provider_entitlement_leases l ON l.lease_id=p.entitlement_lease_id WHERE p.provider_key='microgifter' ORDER BY c.is_default DESC,p.updated_at_utc DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;

    let Some((eligible, expires_at, lifecycle_state, channels_json)) = row else {
        return Ok((false, Vec::new()));
    };

    let lease_active = expires_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc) > Utc::now())
        .unwrap_or(false);
    let lifecycle_active = matches!(lifecycle_state.as_str(), "active" | "offline" | "grace");
    let channels = serde_json::from_str::<Vec<String>>(&channels_json).unwrap_or_default();

    Ok((eligible == 1 && lease_active && lifecycle_active, channels))
}

fn table_exists(connection: &Connection, table_name: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        params![table_name],
        |row| row.get::<_, i64>(0),
    )? == 1)
}

pub fn ensure_update_download_allowed(connection: &Connection, update_id: &str) -> Result<()> {
    // Microgifter is the primary entitlement/update authority. The provider client
    // preserves independent bootstrap, security and recovery updates while gating
    // feature-class downloads with the current signed Microgifter lease.
    microgifter_connection::ensure_update_download_allowed(connection, update_id)
}

pub fn ensure_update_install_window(connection: &Connection) -> Result<()> {
    microgifter_connection::ensure_update_install_window(connection)
}

pub fn record_update_result_receipt(
    connection: &Connection,
    update_id: &str,
    version: &str,
    disposition: &str,
    failure_code: Option<&str>,
) -> Result<()> {
    connection.execute(
        "INSERT INTO software_authority_receipts (receipt_id,authority_key,event_type,update_id,version,disposition,failure_code,submission_state,created_at_utc) VALUES (?1,?2,'update.result',?3,?4,?5,?6,'legacy_forwarded',?7)",
        params![
            Uuid::new_v4().to_string(),
            LEGACY_RECEIPT_AUTHORITY_KEY,
            update_id,
            version,
            disposition,
            failure_code,
            Utc::now().to_rfc3339(),
        ],
    )?;

    microgifter_connection::record_update_result_receipt(
        connection,
        update_id,
        version,
        disposition,
        failure_code,
    )
}

fn internal_error(
    code: &'static str,
    error: impl std::fmt::Display,
) -> (StatusCode, Json<AuthorityApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(AuthorityApiError {
            ok: false,
            error: code,
            message: error.to_string(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microgifter_is_restored_as_primary_without_deleting_vp3_history() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);")
            .expect("schema migration table");
        initialize(&connection).expect("software authority migrations");

        let snapshot = status_snapshot(&connection).expect("authority snapshot");
        assert_eq!(snapshot.current_authority, MICROGIFTER_AUTHORITY);
        assert_eq!(snapshot.target_authority, MICROGIFTER_AUTHORITY);
        assert_eq!(snapshot.cutover_state, "active");
        assert!(snapshot.microgifter_primary_active);
        assert!(!snapshot.legacy_microgifter_fallback_active);
        assert!(!snapshot.update_eligible);
    }
}
