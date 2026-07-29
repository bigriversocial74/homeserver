use crate::{microgifter_connection, AppState};
use anyhow::{bail, ensure, Context, Result};
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

const MIGRATION: &str =
    include_str!("../../../database/migrations/0015_vp3_software_authority.sql");
const MIGRATION_KEY: &str = "0015_vp3_software_authority";
const LEGACY_AUTHORITY: &str = "microgifter_legacy";
const VP3_AUTHORITY: &str = "vp3";

#[derive(Debug, Clone, Serialize)]
pub struct SoftwareAuthoritySnapshot {
    pub current_authority: String,
    pub target_authority: String,
    pub cutover_state: String,
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

#[derive(Debug, Serialize)]
struct AuthorityApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<AuthorityApiError>)>;

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    health_check(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "VP3 software-authority migration is not registered exactly once"
    );
    let singleton_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM homeserver_software_authority WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        singleton_count == 1,
        "HomeServer software-authority state is unavailable"
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
    connection
        .query_row(
            "SELECT current_authority,target_authority,cutover_state,vp3_device_id,vp3_license_id,vp3_lease_id,vp3_lease_expires_at_utc,update_eligible,allowed_update_channels_json,last_vp3_heartbeat_at_utc,last_error_code FROM homeserver_software_authority WHERE singleton_id=1",
            [],
            |row| {
                let current_authority: String = row.get(0)?;
                let channels_json: String = row.get(8)?;
                let allowed_update_channels =
                    serde_json::from_str::<Vec<String>>(&channels_json).unwrap_or_default();
                Ok(SoftwareAuthoritySnapshot {
                    legacy_microgifter_fallback_active: current_authority == LEGACY_AUTHORITY,
                    current_authority,
                    target_authority: row.get(1)?,
                    cutover_state: row.get(2)?,
                    vp3_device_id: row.get(3)?,
                    vp3_license_id: row.get(4)?,
                    vp3_lease_id: row.get(5)?,
                    vp3_lease_expires_at_utc: row.get(6)?,
                    update_eligible: row.get::<_, i64>(7)? == 1,
                    allowed_update_channels,
                    last_vp3_heartbeat_at_utc: row.get(9)?,
                    last_error_code: row.get(10)?,
                })
            },
        )
        .context("unable to load HomeServer software-authority state")
}

pub fn ensure_update_download_allowed(connection: &Connection, update_id: &str) -> Result<()> {
    let snapshot = status_snapshot(connection)?;
    match snapshot.current_authority.as_str() {
        LEGACY_AUTHORITY => {
            microgifter_connection::ensure_update_download_allowed(connection, update_id)
        }
        VP3_AUTHORITY => {
            ensure!(
                snapshot.cutover_state == "active",
                "VP3 software authority is not active"
            );
            ensure!(
                snapshot.update_eligible,
                "VP3 license does not permit this update"
            );
            let expires_at = snapshot
                .vp3_lease_expires_at_utc
                .as_deref()
                .context("VP3 entitlement lease expiration is unavailable")?;
            let expires_at = DateTime::parse_from_rfc3339(expires_at)
                .context("VP3 entitlement lease expiration is invalid")?
                .with_timezone(&Utc);
            ensure!(expires_at > Utc::now(), "VP3 entitlement lease has expired");
            ensure!(
                snapshot.vp3_device_id.is_some() && snapshot.vp3_license_id.is_some(),
                "VP3 licensed device identity is incomplete"
            );
            Ok(())
        }
        other => bail!("unsupported HomeServer software authority '{other}'"),
    }
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
    let snapshot = status_snapshot(connection)?;
    connection.execute(
        "INSERT INTO software_authority_receipts (receipt_id,authority_key,event_type,update_id,version,disposition,failure_code,submission_state,created_at_utc) VALUES (?1,?2,'update.result',?3,?4,?5,?6,?7,?8)",
        params![
            Uuid::new_v4().to_string(),
            snapshot.current_authority,
            update_id,
            version,
            disposition,
            failure_code,
            if snapshot.legacy_microgifter_fallback_active {
                "legacy_forwarded"
            } else {
                "pending_vp3_submission"
            },
            Utc::now().to_rfc3339(),
        ],
    )?;
    if snapshot.legacy_microgifter_fallback_active {
        microgifter_connection::record_update_result_receipt(
            connection,
            update_id,
            version,
            disposition,
            failure_code,
        )?;
    }
    Ok(())
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
    fn default_authority_targets_vp3_without_breaking_legacy_update_access() {
        let connection = Connection::open_in_memory().expect("database");
        connection
            .execute_batch("CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);")
            .expect("schema migration table");
        initialize(&connection).expect("software authority migration");
        let snapshot = status_snapshot(&connection).expect("authority snapshot");
        assert_eq!(snapshot.current_authority, LEGACY_AUTHORITY);
        assert_eq!(snapshot.target_authority, VP3_AUTHORITY);
        assert_eq!(snapshot.cutover_state, "awaiting_vp3_activation");
        assert!(snapshot.legacy_microgifter_fallback_active);
        assert!(!snapshot.update_eligible);
    }
}
