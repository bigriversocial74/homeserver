use crate::{backup_key, config::AppConfig, database};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{bail, ensure, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use microgifter_homeserver_core::{
    BackupActionResult, BackupKind, BackupRecord, BackupReferenceRequest, CreateBackupRequest,
};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{backup::Backup, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tar::{Archive, Builder, Header};
use uuid::Uuid;
use zeroize::Zeroizing;

const PACKAGE_MAGIC: &[u8; 8] = b"MGHSBK03";
const PACKAGE_VERSION: u32 = 3;
const MAX_DATABASE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PACKAGE_BYTES: u64 = 320 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MIN_RECOVERY_PASSPHRASE_CHARS: usize = 12;
const MAX_RECOVERY_PASSPHRASE_CHARS: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupManifest {
    format_version: u32,
    backup_id: String,
    kind: BackupKind,
    created_at_utc: DateTime<Utc>,
    product: String,
    application_version: String,
    database_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackageHeader {
    format_version: u32,
    backup_id: String,
    kind: BackupKind,
    encryption: String,
    created_at_utc: DateTime<Utc>,
    nonce_base64: String,
    salt_base64: Option<String>,
    archive_sha256: String,
    database_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestorePlan {
    format_version: u32,
    restore_id: String,
    backup_id: String,
    staged_at_utc: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum RestoreOutcome {
    Applied {
        restore_id: String,
        backup_id: String,
        rollback_path: Option<PathBuf>,
    },
    RolledBack {
        restore_id: String,
        backup_id: String,
        failure_code: String,
    },
}

struct ExtractedBackup {
    directory: PathBuf,
    database_path: PathBuf,
}

pub fn create_backup(
    connection: &Connection,
    config: &AppConfig,
    request: CreateBackupRequest,
) -> Result<BackupActionResult> {
    validate_request(&request)?;
    let backup_id = Uuid::new_v4().to_string();
    let created_at = Utc::now();
    let timestamp = created_at.format("%Y%m%dT%H%M%SZ");
    let file_name = format!(
        "microgifter-homeserver-{}-{}-{}.mghbackup",
        request.kind.as_str(),
        timestamp,
        &backup_id[..8]
    );
    let destination_directory = match &request.kind {
        BackupKind::Recovery => &config.recovery_dir,
        _ => &config.backups_dir,
    };
    let package_path = destination_directory.join(&file_name);
    let encryption = match &request.kind {
        BackupKind::Recovery => "passphrase_argon2id_aes256gcm",
        _ => "device_key_aes256gcm",
    };

    database::insert_backup_creating(
        connection,
        &backup_id,
        &request.kind,
        encryption,
        &file_name,
        &package_path,
        request.note.as_deref(),
    )?;

    let result = create_backup_inner(
        connection,
        config,
        &backup_id,
        created_at,
        &package_path,
        request.kind.clone(),
        request.passphrase.as_deref(),
    );

    match result {
        Ok((size_bytes, archive_sha256, database_sha256)) => {
            database::mark_backup_ready(
                connection,
                &backup_id,
                size_bytes,
                &archive_sha256,
                &database_sha256,
            )?;
            if request.kind == BackupKind::Automatic {
                enforce_retention(connection)?;
            }
            Ok(BackupActionResult {
                backup: database::backup_by_id(connection, &backup_id)?,
                message: if request.kind == BackupKind::Recovery {
                    "Encrypted recovery package created.".to_owned()
                } else {
                    "Encrypted HomeServer backup created.".to_owned()
                },
                restart_required: false,
            })
        }
        Err(error) => {
            let _ = fs::remove_file(&package_path);
            let _ = database::mark_backup_failed(connection, &backup_id, "backup_creation_failed");
            Err(error)
        }
    }
}

fn create_backup_inner(
    connection: &Connection,
    config: &AppConfig,
    backup_id: &str,
    created_at: DateTime<Utc>,
    package_path: &Path,
    kind: BackupKind,
    passphrase: Option<&str>,
) -> Result<(u64, String, String)> {
    let snapshot_path = config
        .staging_dir
        .join(format!("backup-{backup_id}.sqlite3"));
    let archive_path = config
        .staging_dir
        .join(format!("backup-{backup_id}.tar.gz"));
    let package_temp_path = package_path.with_extension("mghbackup.tmp");

    let result = (|| -> Result<(u64, String, String)> {
        create_sqlite_snapshot(connection, &snapshot_path)?;
        let database_size = fs::metadata(&snapshot_path)?.len();
        ensure!(
            database_size <= MAX_DATABASE_BYTES,
            "HomeServer database exceeds the v1 backup size limit"
        );
        let database_sha256 = sha256_file(&snapshot_path)?;
        let manifest = BackupManifest {
            format_version: PACKAGE_VERSION,
            backup_id: backup_id.to_owned(),
            kind: kind.clone(),
            created_at_utc: created_at,
            product: "Microgifter HomeServer".to_owned(),
            application_version: env!("CARGO_PKG_VERSION").to_owned(),
            database_sha256: database_sha256.clone(),
        };
        create_archive(&snapshot_path, &archive_path, &manifest)?;
        let archive_size = fs::metadata(&archive_path)?.len();
        ensure!(
            archive_size <= MAX_PACKAGE_BYTES,
            "HomeServer backup archive exceeds the v1 package size limit"
        );
        let archive = fs::read(&archive_path)?;
        let archive_sha256 = sha256_bytes(&archive);
        let installation_id = database::installation_id(connection)?;
        let (key, salt) = encryption_key(config, &installation_id, &kind, passphrase)?;
        let key = Zeroizing::new(key);
        let mut nonce_bytes = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(key.as_ref())
            .map_err(|_| anyhow::anyhow!("unable to initialize backup encryption"))?;
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), archive.as_ref())
            .map_err(|_| anyhow::anyhow!("unable to encrypt HomeServer backup"))?;
        let header = PackageHeader {
            format_version: PACKAGE_VERSION,
            backup_id: backup_id.to_owned(),
            kind,
            encryption: if salt.is_some() {
                "passphrase_argon2id_aes256gcm".to_owned()
            } else {
                "device_key_aes256gcm".to_owned()
            },
            created_at_utc: created_at,
            nonce_base64: URL_SAFE_NO_PAD.encode(nonce_bytes),
            salt_base64: salt.map(|value| URL_SAFE_NO_PAD.encode(value)),
            archive_sha256: archive_sha256.clone(),
            database_sha256: database_sha256.clone(),
        };
        let header_json = serde_json::to_vec(&header)?;
        ensure!(
            header_json.len() <= MAX_HEADER_BYTES,
            "HomeServer backup header is unexpectedly large"
        );

        let mut output = File::create(&package_temp_path)?;
        output.write_all(PACKAGE_MAGIC)?;
        output.write_all(&(header_json.len() as u32).to_be_bytes())?;
        output.write_all(&header_json)?;
        output.write_all(&ciphertext)?;
        output.sync_all()?;
        drop(output);
        fs::rename(&package_temp_path, package_path)?;
        let size_bytes = fs::metadata(package_path)?.len();
        Ok((size_bytes, archive_sha256, database_sha256))
    })();

    let _ = fs::remove_file(&snapshot_path);
    let _ = fs::remove_file(&archive_path);
    let _ = fs::remove_file(&package_temp_path);
    result
}

pub fn verify_backup(
    connection: &Connection,
    config: &AppConfig,
    request: BackupReferenceRequest,
) -> Result<BackupActionResult> {
    let record = database::backup_by_id(connection, &request.backup_id)?;
    let extracted =
        decrypt_and_extract(connection, config, &record, request.passphrase.as_deref())?;
    fs::remove_dir_all(&extracted.directory)?;
    database::mark_backup_verified(connection, &record.backup_id)?;
    Ok(BackupActionResult {
        backup: database::backup_by_id(connection, &record.backup_id)?,
        message: "Backup encryption, hashes, archive, and SQLite integrity verified.".to_owned(),
        restart_required: false,
    })
}

pub fn stage_restore(
    connection: &Connection,
    config: &AppConfig,
    request: BackupReferenceRequest,
) -> Result<BackupActionResult> {
    ensure!(
        request.confirmation.as_deref() == Some("RESTORE"),
        "restore confirmation must equal RESTORE"
    );
    ensure!(
        !config.pending_restore_plan_path().exists()
            && !config.pending_restore_database_path().exists(),
        "another HomeServer restore is already staged"
    );
    let record = database::backup_by_id(connection, &request.backup_id)?;
    let extracted =
        decrypt_and_extract(connection, config, &record, request.passphrase.as_deref())?;
    let restore_id = Uuid::new_v4().to_string();
    let pending_database = config.pending_restore_database_path();
    let pending_temp = pending_database.with_extension("sqlite3.tmp");
    let plan_path = config.pending_restore_plan_path();
    let plan = RestorePlan {
        format_version: PACKAGE_VERSION,
        restore_id: restore_id.clone(),
        backup_id: record.backup_id.clone(),
        staged_at_utc: Utc::now(),
    };

    let staging_result = (|| -> Result<()> {
        if pending_temp.exists() {
            fs::remove_file(&pending_temp)?;
        }
        fs::copy(&extracted.database_path, &pending_temp)?;
        File::options()
            .write(true)
            .open(&pending_temp)?
            .sync_all()?;
        fs::rename(&pending_temp, &pending_database)?;
        write_json_atomic(&plan_path, &plan)?;
        database::create_restore_request(
            connection,
            &restore_id,
            &record.backup_id,
            &pending_database,
            "RESTORE",
        )?;
        database::mark_backup_restore_staged(connection, &record.backup_id)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&extracted.directory);
    if let Err(error) = staging_result {
        let _ = fs::remove_file(&pending_temp);
        let _ = fs::remove_file(&pending_database);
        let _ = fs::remove_file(&plan_path);
        let _ = database::delete_restore_request(connection, &restore_id);
        return Err(error).context("unable to persist the staged HomeServer restore");
    }

    Ok(BackupActionResult {
        backup: database::backup_by_id(connection, &record.backup_id)?,
        message: "Restore staged and verified. Restart HomeServer to apply it.".to_owned(),
        restart_required: true,
    })
}

pub fn apply_pending_restore(config: &AppConfig) -> Result<Option<RestoreOutcome>> {
    let plan_path = config.pending_restore_plan_path();
    if !plan_path.exists() {
        return Ok(None);
    }
    let plan: RestorePlan = serde_json::from_slice(&fs::read(&plan_path)?)?;
    ensure!(
        plan.format_version == PACKAGE_VERSION,
        "staged restore format is unsupported"
    );
    let pending_database = config.pending_restore_database_path();
    ensure!(
        pending_database.exists(),
        "staged restore database is missing"
    );
    verify_sqlite_database(&pending_database)?;

    let rollback_path = activate_staged_database(
        &config.database_path,
        &pending_database,
        &config.restore_dir,
        &plan.restore_id,
    )?;
    match verify_sqlite_database(&config.database_path) {
        Ok(()) => {
            fs::remove_file(&plan_path)?;
            Ok(Some(RestoreOutcome::Applied {
                restore_id: plan.restore_id,
                backup_id: plan.backup_id,
                rollback_path,
            }))
        }
        Err(error) => {
            let failed_path = config.restore_dir.join(format!(
                "failed-restore-{}-{}.sqlite3",
                Utc::now().format("%Y%m%dT%H%M%SZ"),
                &plan.restore_id[..8]
            ));
            let _ = fs::rename(&config.database_path, failed_path);
            remove_sqlite_sidecars(&config.database_path);
            if let Some(rollback_path) = rollback_path {
                fs::rename(rollback_path, &config.database_path)?;
            }
            fs::remove_file(&plan_path)?;
            Ok(Some(RestoreOutcome::RolledBack {
                restore_id: plan.restore_id,
                backup_id: plan.backup_id,
                failure_code: format!("restore_integrity_failed:{error}"),
            }))
        }
    }
}

fn activate_staged_database(
    current_database: &Path,
    pending_database: &Path,
    restore_directory: &Path,
    restore_id: &str,
) -> Result<Option<PathBuf>> {
    remove_sqlite_sidecars(current_database);
    let rollback_path = if current_database.exists() {
        let path = restore_directory.join(format!(
            "rollback-{}-{}.sqlite3",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            &restore_id[..8]
        ));
        fs::rename(current_database, &path)?;
        Some(path)
    } else {
        None
    };

    if let Err(activation_error) = fs::rename(pending_database, current_database) {
        if let Some(rollback_path) = rollback_path.as_ref() {
            fs::rename(rollback_path, current_database).with_context(|| {
                format!(
                    "unable to reactivate the previous HomeServer database after restore activation failed: {activation_error}"
                )
            })?;
        }
        return Err(activation_error).context("unable to activate the staged HomeServer database");
    }
    Ok(rollback_path)
}

pub fn create_automatic_if_due(
    connection: &Connection,
    config: &AppConfig,
) -> Result<Option<BackupRecord>> {
    if !database::automatic_backup_due(connection, Utc::now())? {
        return Ok(None);
    }
    let result = create_backup(
        connection,
        config,
        CreateBackupRequest {
            kind: BackupKind::Automatic,
            passphrase: None,
            note: Some("Scheduled encrypted backup".to_owned()),
        },
    )?;
    Ok(Some(result.backup))
}

fn validate_request(request: &CreateBackupRequest) -> Result<()> {
    match &request.kind {
        BackupKind::Recovery => validate_passphrase(request.passphrase.as_deref())?,
        _ => ensure!(
            request.passphrase.as_deref().unwrap_or_default().is_empty(),
            "passphrases are only accepted for recovery packages"
        ),
    }
    if let Some(note) = &request.note {
        ensure!(note.chars().count() <= 500, "backup note is too long");
    }
    Ok(())
}

fn validate_passphrase(passphrase: Option<&str>) -> Result<()> {
    let passphrase = passphrase.unwrap_or_default();
    let count = passphrase.chars().count();
    ensure!(
        (MIN_RECOVERY_PASSPHRASE_CHARS..=MAX_RECOVERY_PASSPHRASE_CHARS).contains(&count),
        "recovery passphrase must contain between 12 and 256 characters"
    );
    Ok(())
}

fn encryption_key(
    config: &AppConfig,
    installation_id: &str,
    kind: &BackupKind,
    passphrase: Option<&str>,
) -> Result<([u8; 32], Option<[u8; 16]>)> {
    if *kind == BackupKind::Recovery {
        validate_passphrase(passphrase)?;
        let mut salt = [0_u8; 16];
        OsRng.fill_bytes(&mut salt);
        let key = derive_passphrase_key(passphrase.unwrap_or_default(), &salt)?;
        Ok((key, Some(salt)))
    } else {
        Ok((backup_key::load_or_create(config, installation_id)?, None))
    }
}

fn decryption_key(
    config: &AppConfig,
    installation_id: &str,
    header: &PackageHeader,
    passphrase: Option<&str>,
) -> Result<[u8; 32]> {
    match header.encryption.as_str() {
        "device_key_aes256gcm" => backup_key::load(config, installation_id),
        "passphrase_argon2id_aes256gcm" => {
            validate_passphrase(passphrase)?;
            let salt = header
                .salt_base64
                .as_deref()
                .context("recovery package salt is missing")?;
            let salt = URL_SAFE_NO_PAD
                .decode(salt)
                .context("recovery package salt is invalid")?;
            ensure!(salt.len() == 16, "recovery package salt is invalid");
            let mut salt_array = [0_u8; 16];
            salt_array.copy_from_slice(&salt);
            derive_passphrase_key(passphrase.unwrap_or_default(), &salt_array)
        }
        _ => bail!("unsupported HomeServer backup encryption"),
    }
}

fn derive_passphrase_key(passphrase: &str, salt: &[u8; 16]) -> Result<[u8; 32]> {
    let params = Params::new(65_536, 3, 1, Some(32))
        .map_err(|error| anyhow::anyhow!("invalid Argon2id parameters: {error}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| anyhow::anyhow!("unable to derive recovery key: {error}"))?;
    Ok(key)
}

fn create_sqlite_snapshot(source: &Connection, destination_path: &Path) -> Result<()> {
    let _ = fs::remove_file(destination_path);
    let mut destination = Connection::open(destination_path)?;
    {
        let backup = Backup::new(source, &mut destination)?;
        backup.run_to_completion(128, Duration::from_millis(10), None)?;
    }
    database::configure_connection(&destination)?;
    database::health_check(&destination)?;
    destination.close().map_err(|(_, error)| error)?;
    File::options()
        .write(true)
        .open(destination_path)?
        .sync_all()?;
    Ok(())
}

fn create_archive(
    database_path: &Path,
    archive_path: &Path,
    manifest: &BackupManifest,
) -> Result<()> {
    let output = File::create(archive_path)?;
    let encoder = GzEncoder::new(output, Compression::default());
    let mut builder = Builder::new(encoder);
    let manifest_json = serde_json::to_vec(manifest)?;
    let mut header = Header::new_gnu();
    header.set_size(manifest_json.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    builder.append_data(&mut header, "manifest.json", Cursor::new(manifest_json))?;
    builder.append_path_with_name(database_path, "homeserver.sqlite3")?;
    let encoder = builder.into_inner()?;
    let output = encoder.finish()?;
    output.sync_all()?;
    Ok(())
}

fn decrypt_and_extract(
    connection: &Connection,
    config: &AppConfig,
    record: &BackupRecord,
    passphrase: Option<&str>,
) -> Result<ExtractedBackup> {
    let package_path = PathBuf::from(&record.storage_path);
    let package_size = fs::metadata(&package_path)
        .with_context(|| format!("backup package {} is unavailable", package_path.display()))?
        .len();
    ensure!(
        package_size <= MAX_PACKAGE_BYTES,
        "backup package exceeds the size limit"
    );
    let package = fs::read(&package_path)?;
    let (header, ciphertext) = parse_package(&package)?;
    ensure!(
        header.backup_id == record.backup_id,
        "backup identity does not match its catalog record"
    );
    ensure!(
        header.kind == record.kind,
        "backup kind does not match its catalog record"
    );
    let installation_id = database::installation_id(connection)?;
    let key = Zeroizing::new(decryption_key(
        config,
        &installation_id,
        &header,
        passphrase,
    )?);
    let nonce = URL_SAFE_NO_PAD
        .decode(&header.nonce_base64)
        .context("backup nonce is invalid")?;
    ensure!(nonce.len() == 12, "backup nonce is invalid");
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("unable to initialize backup decryption"))?;
    let archive = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext)
        .map_err(|_| {
            anyhow::anyhow!("backup passphrase, device key, or package integrity is invalid")
        })?;
    ensure!(
        sha256_bytes(&archive) == header.archive_sha256,
        "backup archive hash does not match"
    );

    let directory = config
        .staging_dir
        .join(format!("verify-{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&directory)?;
    let extraction_result = extract_archive(&archive, &directory, &header);
    if let Err(error) = extraction_result {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }
    let database_path = directory.join("homeserver.sqlite3");
    Ok(ExtractedBackup {
        directory,
        database_path,
    })
}

fn parse_package(package: &[u8]) -> Result<(PackageHeader, &[u8])> {
    ensure!(package.len() >= 12, "backup package is truncated");
    ensure!(
        &package[..8] == PACKAGE_MAGIC,
        "backup package magic is invalid"
    );
    let header_length = u32::from_be_bytes(package[8..12].try_into()?) as usize;
    ensure!(
        header_length > 0 && header_length <= MAX_HEADER_BYTES,
        "backup package header length is invalid"
    );
    ensure!(
        package.len() > 12 + header_length,
        "backup package is truncated"
    );
    let header: PackageHeader = serde_json::from_slice(&package[12..12 + header_length])?;
    ensure!(
        header.format_version == PACKAGE_VERSION,
        "backup package format is unsupported"
    );
    Ok((header, &package[12 + header_length..]))
}

fn extract_archive(archive: &[u8], directory: &Path, header: &PackageHeader) -> Result<()> {
    let decoder = GzDecoder::new(Cursor::new(archive));
    let mut tar = Archive::new(decoder);
    let mut manifest: Option<BackupManifest> = None;
    let mut database_written = false;
    let mut entry_count = 0_u8;

    for entry in tar.entries()? {
        entry_count = entry_count
            .checked_add(1)
            .context("backup archive contains too many entries")?;
        ensure!(entry_count <= 2, "backup archive contains unexpected entries");
        let mut entry = entry?;
        ensure!(
            entry.header().entry_type().is_file(),
            "backup archive entries must be regular files"
        );
        let path = entry.path()?.into_owned();
        match path.to_string_lossy().as_ref() {
            "manifest.json" => {
                ensure!(manifest.is_none(), "backup archive contains duplicate manifests");
                ensure!(
                    entry.size() <= MAX_HEADER_BYTES as u64,
                    "backup manifest is too large"
                );
                let mut bytes = Vec::with_capacity(entry.size() as usize);
                let mut limited = entry.take(MAX_HEADER_BYTES as u64 + 1);
                limited.read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() <= MAX_HEADER_BYTES,
                    "backup manifest is too large"
                );
                manifest = Some(serde_json::from_slice(&bytes)?);
            }
            "homeserver.sqlite3" => {
                ensure!(
                    !database_written,
                    "backup archive contains duplicate databases"
                );
                ensure!(
                    entry.size() <= MAX_DATABASE_BYTES,
                    "backup database exceeds the size limit"
                );
                let destination = directory.join("homeserver.sqlite3");
                let mut output = File::create(&destination)?;
                let copied = std::io::copy(
                    &mut entry.take(MAX_DATABASE_BYTES + 1),
                    &mut output,
                )?;
                ensure!(
                    copied <= MAX_DATABASE_BYTES,
                    "backup database exceeds the size limit"
                );
                output.sync_all()?;
                database_written = true;
            }
            _ => bail!("backup archive contains an unsupported path"),
        }
    }

    ensure!(entry_count == 2, "backup archive is incomplete");
    let manifest = manifest.context("backup manifest is missing")?;
    ensure!(database_written, "backup database is missing");
    ensure!(
        manifest.format_version == PACKAGE_VERSION,
        "backup manifest version is unsupported"
    );
    ensure!(
        manifest.backup_id == header.backup_id,
        "backup manifest identity does not match"
    );
    ensure!(
        manifest.kind == header.kind,
        "backup manifest kind does not match"
    );
    ensure!(
        manifest.database_sha256 == header.database_sha256,
        "backup database hash contract does not match"
    );
    let database_path = directory.join("homeserver.sqlite3");
    ensure!(
        fs::metadata(&database_path)?.len() <= MAX_DATABASE_BYTES,
        "backup database exceeds the size limit"
    );
    ensure!(
        sha256_file(&database_path)? == header.database_sha256,
        "backup database hash does not match"
    );
    verify_sqlite_database(&database_path)?;
    Ok(())
}

fn verify_sqlite_database(path: &Path) -> Result<()> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    database::health_check(&connection)
}

pub fn enforce_pre_update_retention(connection: &Connection) -> Result<()> {
    for (backup_id, path) in database::unreferenced_pre_update_backups(connection)? {
        if path.exists() {
            fs::remove_file(&path).with_context(|| {
                format!("unable to remove expired pre-update backup {}", path.display())
            })?;
        }
        database::delete_backup_record(connection, &backup_id)?;
    }
    Ok(())
}

fn enforce_retention(connection: &Connection) -> Result<()> {
    for (backup_id, path) in database::retention_candidates(connection)? {
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("unable to remove expired backup {}", path.display()))?;
        }
        database::delete_backup_record(connection, &backup_id)?;
    }
    Ok(())
}

fn remove_sqlite_sidecars(database_path: &Path) {
    let database_text = database_path.to_string_lossy();
    let _ = fs::remove_file(format!("{database_text}-wal"));
    let _ = fs::remove_file(format!("{database_text}-shm"));
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    let mut output = File::create(&temp)?;
    output.write_all(&serde_json::to_vec_pretty(value)?)?;
    output.sync_all()?;
    drop(output);
    fs::rename(temp, path)?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn config(directory: &Path) -> AppConfig {
        let data_dir = directory.to_path_buf();
        let config = AppConfig {
            database_path: data_dir.join("homeserver.sqlite3"),
            logs_dir: data_dir.join("logs"),
            backups_dir: data_dir.join("backups"),
            recovery_dir: data_dir.join("recovery-packages"),
            imports_dir: data_dir.join("imports"),
            restore_dir: data_dir.join("restore"),
            staging_dir: data_dir.join("staging"),
            updates_dir: data_dir.join("updates"),
            update_staging_dir: data_dir.join("updates/staging"),
            update_rollback_dir: data_dir.join("updates/rollback"),
            update_installed_dir: data_dir.join("updates/installed"),
            update_manifest_url: "https://updates.microgifter.com/homeserver/stable/manifest.json"
                .to_owned(),
            data_dir,
            server_name: "Test HomeServer".to_owned(),
        };
        for path in [
            &config.logs_dir,
            &config.backups_dir,
            &config.recovery_dir,
            &config.imports_dir,
            &config.restore_dir,
            &config.staging_dir,
            &config.updates_dir,
            &config.update_staging_dir,
            &config.update_rollback_dir,
            &config.update_installed_dir,
        ] {
            fs::create_dir_all(path).expect("test directory");
        }
        config
    }

    #[test]
    fn recovery_package_round_trip_and_wrong_passphrase_rejection() {
        let directory = tempdir().expect("temporary directory");
        let config = config(directory.path());
        let connection = database::initialize(&config.database_path).expect("database");
        let result = create_backup(
            &connection,
            &config,
            CreateBackupRequest {
                kind: BackupKind::Recovery,
                passphrase: Some("correct horse battery staple".to_owned()),
                note: Some("test recovery".to_owned()),
            },
        )
        .expect("recovery package");

        verify_backup(
            &connection,
            &config,
            BackupReferenceRequest {
                backup_id: result.backup.backup_id.clone(),
                passphrase: Some("correct horse battery staple".to_owned()),
                confirmation: None,
            },
        )
        .expect("valid passphrase");
        assert!(verify_backup(
            &connection,
            &config,
            BackupReferenceRequest {
                backup_id: result.backup.backup_id,
                passphrase: Some("incorrect passphrase value".to_owned()),
                confirmation: None,
            }
        )
        .is_err());
    }

    #[test]
    fn pending_restore_rejects_invalid_database_before_replacement() {
        let directory = tempdir().expect("temporary directory");
        let config = config(directory.path());
        fs::write(config.pending_restore_plan_path(), b"{}" as &[u8]).expect("plan");
        fs::write(
            config.pending_restore_database_path(),
            b"not sqlite" as &[u8],
        )
        .expect("invalid database");
        assert!(apply_pending_restore(&config).is_err());
    }
    #[test]
    fn failed_restore_activation_reinstates_the_previous_database() {
        let directory = tempdir().expect("temporary directory");
        let current = directory.path().join("homeserver.sqlite3");
        let missing_pending = directory.path().join("missing.sqlite3");
        fs::write(&current, b"original database").expect("current database fixture");

        let error = activate_staged_database(
            &current,
            &missing_pending,
            directory.path(),
            "12345678-1234-1234-1234-123456789012",
        )
        .expect_err("activation should fail when the pending database is missing");

        assert!(error.to_string().contains("activate the staged"));
        assert_eq!(
            fs::read(&current).expect("previous database should be restored"),
            b"original database"
        );
    }


}
