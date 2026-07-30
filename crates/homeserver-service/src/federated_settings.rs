use crate::AppState;
use anyhow::{anyhow, bail, ensure, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use keyring::Entry;
use reqwest::Url;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Arc, time::Duration};
use uuid::Uuid;

const MIGRATION: &str = include_str!("../../../database/migrations/0019_federated_settings.sql");
const MIGRATION_KEY: &str = "0019_federated_settings";
const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";
const CREDENTIAL_KEY: &str = "vp3-software-authority-device-credential";
const MAX_CONTROL_BODY_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 128 * 1024;
const MAX_SETTINGS: usize = 64;
const MAX_VALUE_CHARS: usize = 200;
const MAX_RECEIPTS: i64 = 5_000;

pub type SettingsApiResult<T> = Result<Json<T>, (StatusCode, Json<SettingsApiError>)>;

#[derive(Debug, Serialize)]
pub struct SettingsApiError {
    ok: bool,
    error: &'static str,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FederatedSetting {
    pub setting_key: String,
    pub label: String,
    pub description: String,
    pub category: String,
    pub authority: String,
    pub value_type: String,
    pub allowed_values: Option<Vec<Value>>,
    pub value: Value,
    pub local_revision: u64,
    pub cloud_revision: u64,
    pub source_authority: String,
    pub dirty: bool,
    pub last_conflict_reason: Option<String>,
    pub editable_in_vp3: bool,
    pub editable_in_homeserver: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FederatedSettingsSnapshot {
    pub schema: String,
    pub configured: bool,
    pub max_cloud_revision: u64,
    pub snapshot_hash: Option<String>,
    pub last_synced_at_utc: Option<String>,
    pub last_error_code: Option<String>,
    pub dirty_count: u64,
    pub settings: Vec<FederatedSetting>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSettingRequest {
    pub setting_key: String,
    pub value: Value,
    pub expected_local_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct CloudEnvelope<T> {
    data: T,
}

#[derive(Debug, Clone, Serialize)]
struct DeviceSyncRequest {
    device_public_id: String,
    request_id: String,
    base_revision: u64,
    updates: Vec<DeviceSettingUpdate>,
}

#[derive(Debug, Clone, Serialize)]
struct DeviceSettingUpdate {
    setting_key: String,
    value: Value,
    expected_revision: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CloudSnapshot {
    schema: String,
    account_id: i64,
    device_public_id: Option<String>,
    max_revision: u64,
    settings: Vec<CloudSetting>,
    generated_at: String,
    snapshot_hash: String,
    signed_document: String,
    signature: String,
    signing_key_id: String,
    signature_algorithm: String,
    signed_document_hash: String,
    #[serde(default)]
    replayed: bool,
    #[serde(default)]
    applied: Vec<CloudApplied>,
    #[serde(default)]
    conflicts: Vec<CloudConflict>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CloudSetting {
    setting_key: String,
    label: String,
    description: String,
    category: String,
    authority: String,
    value_type: String,
    allowed_values: Option<Vec<Value>>,
    value: Value,
    revision: u64,
    source_authority: String,
    scope: String,
    editable_in_vp3: bool,
    editable_in_homeserver: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CloudApplied {
    setting_key: String,
    revision: u64,
    #[serde(default)]
    index: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CloudConflict {
    setting_key: String,
    reason: String,
    #[serde(default)]
    current_revision: u64,
}

#[derive(Debug, Clone)]
struct CatalogDefinition {
    setting_key: String,
    label: String,
    description: String,
    category: String,
    authority: String,
    value_type: String,
    default_value_json: String,
    allowed_values_json: Option<String>,
    visible_in_vp3: bool,
    visible_in_homeserver: bool,
}

#[derive(Debug, Clone)]
struct StoredValue {
    value_json: String,
    local_revision: u64,
    cloud_revision: u64,
    source_authority: String,
    dirty: bool,
    last_conflict_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct DeviceIdentity {
    account_id: i64,
    device_public_id: String,
}

pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(MIGRATION)?;
    health_check(connection)?;
    maintain_history(connection)
}

pub fn health_check(connection: &Connection) -> Result<()> {
    let migration_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM schema_migrations WHERE migration_key=?1",
        params![MIGRATION_KEY],
        |row| row.get(0),
    )?;
    ensure!(
        migration_count == 1,
        "federated settings migration is not registered exactly once"
    );
    let catalog_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM federated_setting_catalog WHERE sensitivity='non_secret'",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        catalog_count >= 11,
        "federated settings catalog is incomplete"
    );
    let sync_state_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM federated_settings_sync_state WHERE singleton_id=1",
        [],
        |row| row.get(0),
    )?;
    ensure!(
        sync_state_count == 1,
        "federated settings sync state is unavailable"
    );
    Ok(())
}

pub fn maintain_history(connection: &Connection) -> Result<()> {
    connection.execute(
        "DELETE FROM federated_settings_sync_receipts WHERE created_at_utc < strftime('%Y-%m-%dT%H:%M:%fZ','now','-365 days')",
        [],
    )?;
    connection.execute(
        "DELETE FROM federated_settings_sync_receipts WHERE receipt_id NOT IN (SELECT receipt_id FROM federated_settings_sync_receipts ORDER BY created_at_utc DESC,receipt_id DESC LIMIT ?1)",
        params![MAX_RECEIPTS],
    )?;
    Ok(())
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/federated-settings", get(snapshot_handler))
        .route("/v1/federated-settings/update", post(update_handler))
        .route("/v1/federated-settings/sync", post(sync_handler))
        .layer(DefaultBodyLimit::max(MAX_CONTROL_BODY_BYTES))
        .with_state(state)
}

async fn snapshot_handler(
    State(state): State<Arc<AppState>>,
) -> SettingsApiResult<FederatedSettingsSnapshot> {
    tokio::task::spawn_blocking(move || snapshot(&state))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| api_error("federated_settings_snapshot_failed", error))
}

async fn update_handler(
    State(state): State<Arc<AppState>>,
    Json(request): Json<UpdateSettingRequest>,
) -> SettingsApiResult<FederatedSettingsSnapshot> {
    tokio::task::spawn_blocking(move || update_local(&state, request))
        .await
        .map_err(task_error)?
        .map(Json)
        .map_err(|error| api_error("federated_setting_update_failed", error))
}

async fn sync_handler(
    State(state): State<Arc<AppState>>,
) -> SettingsApiResult<FederatedSettingsSnapshot> {
    synchronize(&state)
        .await
        .map(Json)
        .map_err(|error| api_error("federated_settings_sync_failed", error))
}

pub fn snapshot(state: &AppState) -> Result<FederatedSettingsSnapshot> {
    let connection = state.connection()?;
    snapshot_with_connection(&connection)
}

fn snapshot_with_connection(connection: &Connection) -> Result<FederatedSettingsSnapshot> {
    let configured: bool = connection
        .query_row(
            "SELECT COUNT(*) FROM vp3_authority_client_state WHERE singleton_id=1 AND activation_state='active'",
            [],
            |row| row.get::<_, i64>(0),
        )?
        == 1;
    let (max_cloud_revision, snapshot_hash, last_synced_at_utc, last_error_code): (
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = connection.query_row(
        "SELECT max_cloud_revision,snapshot_hash,last_synced_at_utc,last_error_code FROM federated_settings_sync_state WHERE singleton_id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;

    let mut statement = connection.prepare(
        "SELECT setting_key,label,description,category,authority,value_type,default_value_json,allowed_values_json,visible_in_vp3,visible_in_homeserver FROM federated_setting_catalog WHERE sensitivity='non_secret' ORDER BY category,setting_key",
    )?;
    let definitions = statement
        .query_map([], definition_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut settings = Vec::with_capacity(definitions.len());
    let mut dirty_count = 0_u64;
    for definition in definitions {
        let stored = stored_value(connection, &definition.setting_key)?;
        let value = serde_json::from_str::<Value>(
            stored
                .as_ref()
                .map(|item| item.value_json.as_str())
                .unwrap_or(&definition.default_value_json),
        )?;
        let allowed_values = definition
            .allowed_values_json
            .as_deref()
            .map(serde_json::from_str::<Vec<Value>>)
            .transpose()?;
        let dirty = stored.as_ref().is_some_and(|item| item.dirty);
        if dirty {
            dirty_count += 1;
        }
        settings.push(FederatedSetting {
            setting_key: definition.setting_key,
            label: definition.label,
            description: definition.description,
            category: definition.category,
            authority: definition.authority.clone(),
            value_type: definition.value_type,
            allowed_values,
            value,
            local_revision: stored.as_ref().map_or(0, |item| item.local_revision),
            cloud_revision: stored.as_ref().map_or(0, |item| item.cloud_revision),
            source_authority: stored.as_ref().map_or_else(
                || "default".to_owned(),
                |item| item.source_authority.clone(),
            ),
            dirty,
            last_conflict_reason: stored.and_then(|item| item.last_conflict_reason),
            editable_in_vp3: definition.visible_in_vp3 && definition.authority != "homeserver",
            editable_in_homeserver: definition.visible_in_homeserver
                && definition.authority != "vp3",
        });
    }

    Ok(FederatedSettingsSnapshot {
        schema: "homeserver.federated-settings.v1".to_owned(),
        configured,
        max_cloud_revision: max_cloud_revision.max(0) as u64,
        snapshot_hash,
        last_synced_at_utc,
        last_error_code,
        dirty_count,
        settings,
    })
}

fn update_local(
    state: &AppState,
    request: UpdateSettingRequest,
) -> Result<FederatedSettingsSnapshot> {
    validate_setting_key(&request.setting_key)?;
    let connection = state.connection()?;
    let definition = definition(&connection, &request.setting_key)?;
    ensure!(
        definition.authority != "vp3",
        "this setting is controlled by VP3"
    );
    let value = validate_value(&definition, request.value)?;
    let transaction = connection.unchecked_transaction()?;
    let stored = stored_value(&transaction, &request.setting_key)?;
    let current_revision = stored.as_ref().map_or(0, |item| item.local_revision);
    ensure!(
        current_revision == request.expected_local_revision,
        "the local setting changed before this update was saved"
    );
    let next_revision = current_revision
        .checked_add(1)
        .context("local setting revision overflow")?;
    let cloud_revision = stored.as_ref().map_or(0, |item| item.cloud_revision);
    let value_json = canonical_json(&value)?;
    transaction.execute(
        "INSERT INTO federated_setting_values (setting_key,value_json,value_hash,local_revision,cloud_revision,source_authority,dirty,last_conflict_reason,updated_at_utc) VALUES (?1,?2,?3,?4,?5,'homeserver',1,NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(setting_key) DO UPDATE SET value_json=excluded.value_json,value_hash=excluded.value_hash,local_revision=excluded.local_revision,source_authority='homeserver',dirty=1,last_conflict_reason=NULL,updated_at_utc=excluded.updated_at_utc",
        params![
            request.setting_key,
            value_json,
            sha256_text(&value_json),
            next_revision as i64,
            cloud_revision as i64,
        ],
    )?;
    record_receipt(
        &transaction,
        &format!("LOCAL-{}", Uuid::new_v4().simple()),
        "local_update",
        cloud_revision,
        cloud_revision,
        None,
        "applied",
        0,
    )?;
    transaction.commit()?;
    snapshot_with_connection(&connection)
}

async fn synchronize(state: &AppState) -> Result<FederatedSettingsSnapshot> {
    let identity = device_identity(state)?;
    let credential = load_credential()?;
    let (base_revision, updates) = {
        let connection = state.connection()?;
        let base_revision: i64 = connection.query_row(
            "SELECT max_cloud_revision FROM federated_settings_sync_state WHERE singleton_id=1",
            [],
            |row| row.get(0),
        )?;
        let mut statement = connection.prepare(
            "SELECT v.setting_key,v.value_json,v.cloud_revision FROM federated_setting_values v JOIN federated_setting_catalog c ON c.setting_key=v.setting_key WHERE v.dirty=1 AND c.authority IN ('homeserver','shared') ORDER BY v.setting_key LIMIT ?1",
        )?;
        let rows = statement.query_map(params![MAX_SETTINGS as i64], |row| {
            let value_json: String = row.get(1)?;
            Ok(DeviceSettingUpdate {
                setting_key: row.get(0)?,
                value: serde_json::from_str(&value_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        value_json.len(),
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
                expected_revision: row.get::<_, i64>(2)?.max(0) as u64,
            })
        })?;
        (
            base_revision.max(0) as u64,
            rows.collect::<rusqlite::Result<Vec<_>>>()?,
        )
    };

    let request_id = format!("FSS-{}", Uuid::new_v4().simple());
    let request = DeviceSyncRequest {
        device_public_id: identity.device_public_id.clone(),
        request_id: request_id.clone(),
        base_revision,
        updates,
    };
    let response: CloudSnapshot = post_settings_sync(state, &credential, &request).await?;
    validate_cloud_snapshot(state, &identity, &response)?;
    apply_cloud_snapshot(state, &request_id, base_revision, &response)?;
    snapshot(state)
}

fn apply_cloud_snapshot(
    state: &AppState,
    request_id: &str,
    base_revision: u64,
    cloud: &CloudSnapshot,
) -> Result<()> {
    let applied: HashSet<&str> = cloud
        .applied
        .iter()
        .map(|item| item.setting_key.as_str())
        .collect();
    let conflicts = cloud
        .conflicts
        .iter()
        .map(|item| (item.setting_key.as_str(), item.reason.as_str()))
        .collect::<std::collections::HashMap<_, _>>();
    let connection = state.connection()?;
    let transaction = connection.unchecked_transaction()?;

    for cloud_setting in &cloud.settings {
        let definition = definition(&transaction, &cloud_setting.setting_key)?;
        ensure!(
            definition.authority == cloud_setting.authority
                && definition.value_type == cloud_setting.value_type,
            "VP3 setting definition does not match the local catalog"
        );
        let cloud_value = validate_value(&definition, cloud_setting.value.clone())?;
        let stored = stored_value(&transaction, &cloud_setting.setting_key)?;
        let local_dirty = stored.as_ref().is_some_and(|item| item.dirty);
        let conflict_reason = conflicts.get(cloud_setting.setting_key.as_str()).copied();
        let preserve_local = definition.authority != "vp3"
            && local_dirty
            && conflict_reason.is_some()
            && !applied.contains(cloud_setting.setting_key.as_str());

        if preserve_local {
            transaction.execute(
                "UPDATE federated_setting_values SET cloud_revision=?1,last_conflict_reason=?2,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE setting_key=?3",
                params![
                    cloud_setting.revision as i64,
                    conflict_reason,
                    cloud_setting.setting_key,
                ],
            )?;
            continue;
        }

        let value_json = canonical_json(&cloud_value)?;
        let next_local_revision = stored
            .as_ref()
            .map_or(1, |item| item.local_revision.saturating_add(1));
        let source_authority = if cloud_setting.source_authority == "homeserver" {
            "homeserver"
        } else {
            "vp3"
        };
        transaction.execute(
            "INSERT INTO federated_setting_values (setting_key,value_json,value_hash,local_revision,cloud_revision,source_authority,dirty,last_conflict_reason,updated_at_utc) VALUES (?1,?2,?3,?4,?5,?6,0,NULL,strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(setting_key) DO UPDATE SET value_json=excluded.value_json,value_hash=excluded.value_hash,local_revision=excluded.local_revision,cloud_revision=excluded.cloud_revision,source_authority=excluded.source_authority,dirty=0,last_conflict_reason=NULL,updated_at_utc=excluded.updated_at_utc",
            params![
                cloud_setting.setting_key,
                value_json,
                sha256_text(&value_json),
                next_local_revision as i64,
                cloud_setting.revision as i64,
                source_authority,
            ],
        )?;
    }

    let result = if cloud.conflicts.is_empty() {
        "applied"
    } else if cloud.applied.is_empty() {
        "conflict"
    } else {
        "partial"
    };
    transaction.execute(
        "UPDATE federated_settings_sync_state SET max_cloud_revision=?1,snapshot_hash=?2,last_synced_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now'),last_error_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE singleton_id=1",
        params![cloud.max_revision as i64, cloud.snapshot_hash],
    )?;
    record_receipt(
        &transaction,
        request_id,
        "device_sync",
        base_revision,
        cloud.max_revision,
        Some(&cloud.snapshot_hash),
        result,
        cloud.conflicts.len() as u64,
    )?;
    transaction.commit()?;
    Ok(())
}

fn validate_cloud_snapshot(
    state: &AppState,
    identity: &DeviceIdentity,
    cloud: &CloudSnapshot,
) -> Result<()> {
    super::federated_settings_signature::verify(
        super::federated_settings_signature::SignedSnapshotEvidence {
            public_key_base64: &state.config.vp3_lease_public_key_base64(),
            expected_key_id: &state.config.vp3_lease_key_id(),
            algorithm: &cloud.signature_algorithm,
            key_id: &cloud.signing_key_id,
            signed_document: &cloud.signed_document,
            signature: &cloud.signature,
            signed_document_hash: &cloud.signed_document_hash,
            schema: &cloud.schema,
            account_id: cloud.account_id,
            device_public_id: identity.device_public_id.as_str(),
            max_revision: cloud.max_revision,
            snapshot_hash: &cloud.snapshot_hash,
            generated_at: &cloud.generated_at,
            settings: serde_json::to_value(&cloud.settings)?,
            replayed: cloud.replayed,
            applied: serde_json::to_value(&cloud.applied)?,
            conflicts: serde_json::to_value(&cloud.conflicts)?,
        },
    )?;
    ensure!(
        cloud.schema == "vp3.federated-settings.v1",
        "VP3 federated settings schema is unsupported"
    );
    ensure!(
        cloud.account_id == identity.account_id,
        "VP3 federated settings account identity does not match"
    );
    ensure!(
        cloud.device_public_id.as_deref() == Some(identity.device_public_id.as_str()),
        "VP3 federated settings device identity does not match"
    );
    ensure!(
        cloud.settings.len() <= MAX_SETTINGS,
        "VP3 federated settings snapshot is too large"
    );
    ensure!(
        valid_sha256(&cloud.snapshot_hash),
        "VP3 federated settings snapshot hash is invalid"
    );
    let identity_value = json!({
        "schema": cloud.schema,
        "account_id": cloud.account_id,
        "device_public_id": cloud.device_public_id,
        "max_revision": cloud.max_revision,
        "settings": cloud.settings,
    });
    ensure!(
        sha256_text(&canonical_json(&identity_value)?).eq_ignore_ascii_case(&cloud.snapshot_hash),
        "VP3 federated settings snapshot hash does not match its content"
    );
    let mut keys = HashSet::new();
    let mut settings_by_key = std::collections::HashMap::new();
    for setting in &cloud.settings {
        validate_setting_key(&setting.setting_key)?;
        ensure!(
            keys.insert(setting.setting_key.as_str()),
            "VP3 federated settings snapshot contains duplicate keys"
        );
        ensure!(
            !secret_like_key(&setting.setting_key),
            "VP3 federated settings snapshot contains a secret-like key"
        );
        settings_by_key.insert(setting.setting_key.as_str(), setting);
    }
    let mut applied_keys = HashSet::new();
    for applied in &cloud.applied {
        validate_setting_key(&applied.setting_key)?;
        ensure!(
            applied_keys.insert(applied.setting_key.as_str()),
            "VP3 federated settings result contains duplicate applied keys"
        );
        let setting = settings_by_key
            .get(applied.setting_key.as_str())
            .context("VP3 federated settings result references an unknown applied key")?;
        ensure!(
            setting.revision == applied.revision
                && setting.source_authority == "homeserver"
                && setting.editable_in_homeserver,
            "VP3 federated settings applied evidence does not match the signed setting"
        );
    }
    let mut conflict_keys = HashSet::new();
    for conflict in &cloud.conflicts {
        validate_setting_key(&conflict.setting_key)?;
        ensure!(
            conflict_keys.insert(conflict.setting_key.as_str()),
            "VP3 federated settings result contains duplicate conflict keys"
        );
        ensure!(
            !applied_keys.contains(conflict.setting_key.as_str()),
            "VP3 federated settings result marks one key applied and conflicted"
        );
        let setting = settings_by_key
            .get(conflict.setting_key.as_str())
            .context("VP3 federated settings result references an unknown conflict key")?;
        ensure!(
            matches!(conflict.reason.as_str(), "revision" | "vp3_authority"),
            "VP3 federated settings conflict reason is invalid"
        );
        if conflict.reason == "revision" {
            ensure!(
                conflict.current_revision == setting.revision,
                "VP3 federated settings revision conflict does not match the signed setting"
            );
        }
    }
    ensure!(
        !cloud.replayed || (cloud.applied.is_empty() && cloud.conflicts.is_empty()),
        "VP3 federated settings replay response contains mutation instructions"
    );
    Ok(())
}

async fn post_settings_sync(
    state: &AppState,
    credential: &str,
    request: &DeviceSyncRequest,
) -> Result<CloudSnapshot> {
    let base = Url::parse(&format!(
        "{}/",
        state.config.vp3_base_url()?.trim_end_matches('/')
    ))
    .context("VP3 base URL is invalid")?;
    let url = base
        .join("api/homeserver/v1/settings-sync.php")
        .context("VP3 settings sync endpoint is invalid")?;
    ensure!(
        url.scheme() == "https" && url.host_str() == base.host_str(),
        "VP3 settings sync endpoint escaped the configured authority host"
    );
    let response = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!(
            "Microgifter-HomeServer/{} Federated-Settings/1",
            env!("CARGO_PKG_VERSION")
        ))
        .build()?
        .post(url)
        .bearer_auth(credential)
        .header("X-Request-ID", &request.request_id)
        .header(reqwest::header::ACCEPT, "application/json")
        .json(request)
        .send()
        .await
        .context("VP3 federated settings request failed")?;
    decode_cloud_envelope(response).await
}

async fn decode_cloud_envelope<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_RESPONSE_BYTES as u64,
            "VP3 federated settings response exceeds the size limit"
        );
    }
    let bytes = response
        .bytes()
        .await
        .context("unable to read VP3 federated settings response")?;
    ensure!(
        bytes.len() <= MAX_RESPONSE_BYTES,
        "VP3 federated settings response exceeds the size limit"
    );
    if !status.is_success() {
        let message = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("VP3 settings sync was rejected with HTTP {status}"));
        bail!(bounded(&message, 500));
    }
    let envelope: CloudEnvelope<T> =
        serde_json::from_slice(&bytes).context("VP3 settings sync JSON is invalid")?;
    Ok(envelope.data)
}

fn device_identity(state: &AppState) -> Result<DeviceIdentity> {
    state
        .connection()?
        .query_row(
            "SELECT account_id,device_public_id FROM vp3_authority_client_state WHERE singleton_id=1 AND activation_state='active'",
            [],
            |row| {
                Ok(DeviceIdentity {
                    account_id: row.get(0)?,
                    device_public_id: row.get(1)?,
                })
            },
        )
        .context("VP3 device activation is not configured")
}

fn load_credential() -> Result<String> {
    Entry::new(CREDENTIAL_SERVICE, CREDENTIAL_KEY)?
        .get_password()
        .context("VP3 device credential is unavailable from the operating-system vault")
}

fn definition(connection: &Connection, setting_key: &str) -> Result<CatalogDefinition> {
    connection
        .query_row(
            "SELECT setting_key,label,description,category,authority,value_type,default_value_json,allowed_values_json,visible_in_vp3,visible_in_homeserver FROM federated_setting_catalog WHERE setting_key=?1 AND sensitivity='non_secret'",
            params![setting_key],
            definition_from_row,
        )
        .context("the federated setting was not found")
}

fn definition_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CatalogDefinition> {
    Ok(CatalogDefinition {
        setting_key: row.get(0)?,
        label: row.get(1)?,
        description: row.get(2)?,
        category: row.get(3)?,
        authority: row.get(4)?,
        value_type: row.get(5)?,
        default_value_json: row.get(6)?,
        allowed_values_json: row.get(7)?,
        visible_in_vp3: row.get::<_, i64>(8)? == 1,
        visible_in_homeserver: row.get::<_, i64>(9)? == 1,
    })
}

fn stored_value(connection: &Connection, setting_key: &str) -> Result<Option<StoredValue>> {
    connection
        .query_row(
            "SELECT value_json,local_revision,cloud_revision,source_authority,dirty,last_conflict_reason FROM federated_setting_values WHERE setting_key=?1",
            params![setting_key],
            |row| {
                Ok(StoredValue {
                    value_json: row.get(0)?,
                    local_revision: row.get::<_, i64>(1)?.max(0) as u64,
                    cloud_revision: row.get::<_, i64>(2)?.max(0) as u64,
                    source_authority: row.get(3)?,
                    dirty: row.get::<_, i64>(4)? == 1,
                    last_conflict_reason: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn validate_value(definition: &CatalogDefinition, value: Value) -> Result<Value> {
    match definition.value_type.as_str() {
        "boolean" => ensure!(
            value.is_boolean(),
            "the setting value must be true or false"
        ),
        "integer" => ensure!(
            value.as_i64().is_some(),
            "the setting value must be an integer"
        ),
        "string" => {
            let text = value
                .as_str()
                .context("the setting value must be a string")?;
            ensure!(
                text.chars().count() <= MAX_VALUE_CHARS && !text.chars().any(char::is_control),
                "the setting string is invalid"
            );
        }
        "enum" => {
            let text = value
                .as_str()
                .context("the setting value must be an enum string")?;
            let allowed = definition
                .allowed_values_json
                .as_deref()
                .context("the setting enum has no allowed values")?;
            let allowed: Vec<Value> = serde_json::from_str(allowed)?;
            ensure!(
                allowed
                    .iter()
                    .any(|candidate| candidate.as_str() == Some(text)),
                "the setting value is not permitted"
            );
        }
        _ => bail!("the setting value type is unsupported"),
    }
    Ok(value)
}

fn validate_setting_key(value: &str) -> Result<()> {
    ensure!(
        (3..=120).contains(&value.len())
            && value.starts_with(|character: char| character.is_ascii_lowercase())
            && value.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || "_.-".contains(character)
            }),
        "the setting key is invalid"
    );
    ensure!(
        !secret_like_key(value),
        "secret-like settings cannot be federated"
    );
    Ok(())
}

fn secret_like_key(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "secret",
        "password",
        "credential",
        "private_key",
        "api_key",
        "token",
        "prompt",
        "conversation",
        "file_content",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn record_receipt(
    connection: &Connection,
    request_id: &str,
    direction: &str,
    base_revision: u64,
    applied_revision: u64,
    snapshot_hash: Option<&str>,
    result: &str,
    conflict_count: u64,
) -> Result<()> {
    connection.execute(
        "INSERT INTO federated_settings_sync_receipts (receipt_id,request_id,direction,base_revision,applied_revision,snapshot_hash,result,conflict_count,created_at_utc) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            Uuid::new_v4().to_string(),
            request_id,
            direction,
            base_revision as i64,
            applied_revision as i64,
            snapshot_hash,
            result,
            conflict_count as i64,
        ],
    )?;
    Ok(())
}

fn canonical_json(value: &Value) -> Result<String> {
    serde_json::to_string(&canonical_value(value))
        .context("unable to serialize canonical federated settings JSON")
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut normalized = Map::new();
            for key in keys {
                if let Some(item) = object.get(key) {
                    normalized.insert(key.clone(), canonical_value(item));
                }
            }
            Value::Object(normalized)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        _ => value.clone(),
    }
}

fn sha256_text(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn bounded(value: &str, maximum: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(maximum)
        .collect()
}

fn task_error(error: tokio::task::JoinError) -> (StatusCode, Json<SettingsApiError>) {
    api_error("federated_settings_task_failed", anyhow!(error))
}

fn api_error(
    code: &'static str,
    error: impl std::fmt::Display,
) -> (StatusCode, Json<SettingsApiError>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(SettingsApiError {
            ok: false,
            error: code,
            message: bounded(&error.to_string(), 500),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE schema_migrations (migration_key TEXT PRIMARY KEY);")
            .unwrap();
        initialize(&connection).unwrap();
        connection
    }

    #[test]
    fn catalog_defaults_are_non_secret_and_complete() {
        let connection = connection();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM federated_setting_catalog WHERE sensitivity='non_secret'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count >= 11);
        let secret_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM federated_setting_catalog WHERE setting_key LIKE '%secret%' OR setting_key LIKE '%credential%' OR setting_key LIKE '%password%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(secret_count, 0);
    }

    #[test]
    fn local_update_rejects_vp3_authority_and_stale_revision() {
        let connection = connection();
        let vp3 = definition(&connection, "updates.channel").unwrap();
        assert_eq!(vp3.authority, "vp3");
        let local = definition(&connection, "updates.install_window").unwrap();
        assert_eq!(local.authority, "homeserver");
        let value = validate_value(&local, json!("03:00-04:00")).unwrap();
        assert_eq!(value, json!("03:00-04:00"));
    }

    #[test]
    fn canonical_snapshot_hash_is_order_independent() {
        let first = json!({"b": 2, "a": {"d": 4, "c": 3}});
        let second = json!({"a": {"c": 3, "d": 4}, "b": 2});
        assert_eq!(
            sha256_text(&canonical_json(&first).unwrap()),
            sha256_text(&canonical_json(&second).unwrap())
        );
    }

    #[test]
    fn secret_like_keys_are_rejected() {
        for key in [
            "commerce.secret_key",
            "provider.api_key",
            "account.password",
            "agent.prompt",
        ] {
            assert!(validate_setting_key(key).is_err());
        }
        assert!(validate_setting_key("commerce.default_currency").is_ok());
    }
}
