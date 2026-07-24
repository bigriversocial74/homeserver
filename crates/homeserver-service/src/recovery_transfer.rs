use crate::{backup, config::AppConfig, database};
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use microgifter_homeserver_core::{
    BackupActionResult, BackupKind, BackupReferenceRequest, BackupState,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const PACKAGE_MAGIC: &[u8; 8] = b"MGHSBK03";
const PACKAGE_VERSION: u32 = 3;
const MAX_PACKAGE_BYTES: u64 = 320 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
struct ImportedHeader {
    format_version: u32,
    backup_id: String,
    kind: BackupKind,
    encryption: String,
    created_at_utc: DateTime<Utc>,
    archive_sha256: String,
    database_sha256: String,
}

pub struct ExportPackage {
    pub path: PathBuf,
    pub file_name: String,
    pub size_bytes: u64,
}

pub fn import_recovery_package(
    connection: &Connection,
    config: &AppConfig,
    temporary_path: &Path,
    passphrase: String,
) -> Result<BackupActionResult> {
    let package_size = fs::metadata(temporary_path)
        .context("uploaded recovery package is unavailable")?
        .len();
    ensure!(
        package_size > 12 && package_size <= MAX_PACKAGE_BYTES,
        "recovery package size is invalid"
    );
    let header = read_header(temporary_path)?;
    ensure!(
        header.format_version == PACKAGE_VERSION,
        "recovery package format is unsupported"
    );
    ensure!(
        header.kind == BackupKind::Recovery,
        "only passphrase-protected recovery packages can be imported"
    );
    ensure!(
        header.encryption == "passphrase_argon2id_aes256gcm",
        "imported package is not a portable recovery package"
    );
    Uuid::parse_str(&header.backup_id).context("recovery package identity is invalid")?;
    validate_sha256(&header.archive_sha256, "archive")?;
    validate_sha256(&header.database_sha256, "database")?;

    let file_name = format!(
        "microgifter-homeserver-recovery-{}-{}.mghbackup",
        header.created_at_utc.format("%Y%m%dT%H%M%SZ"),
        &header.backup_id[..8]
    );
    let destination = config.recovery_dir.join(&file_name);
    let existing = connection
        .query_row(
            "SELECT archive_sha256,database_sha256,storage_path FROM backup_records WHERE backup_id=?1",
            params![header.backup_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;

    let is_new_record = existing.is_none();
    let mut imported_new_file = false;
    if let Some((archive_sha256, database_sha256, storage_path)) = existing {
        ensure!(
            archive_sha256.as_deref() == Some(header.archive_sha256.as_str())
                && database_sha256.as_deref() == Some(header.database_sha256.as_str()),
            "recovery package identity conflicts with a different catalog record"
        );
        let existing_path = PathBuf::from(storage_path);
        if !existing_path.exists() {
            move_replace(temporary_path, &destination)?;
            imported_new_file = true;
            connection.execute(
                "UPDATE backup_records SET storage_path=?1,file_name=?2,size_bytes=?3,state='ready',failure_code=NULL,updated_at_utc=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE backup_id=?4",
                params![destination.to_string_lossy(), file_name, package_size as i64, header.backup_id],
            )?;
        } else {
            fs::remove_file(temporary_path)?;
        }
    } else {
        move_replace(temporary_path, &destination)?;
        imported_new_file = true;
        let insert_result = connection.execute(
            "INSERT INTO backup_records (backup_id,kind,encryption,state,file_name,storage_path,size_bytes,archive_sha256,database_sha256,note,created_at_utc,updated_at_utc) VALUES (?1,'recovery',?2,'ready',?3,?4,?5,?6,?7,'Imported portable recovery package',?8,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![
                header.backup_id,
                header.encryption,
                file_name,
                destination.to_string_lossy(),
                package_size as i64,
                header.archive_sha256,
                header.database_sha256,
                header.created_at_utc.to_rfc3339(),
            ],
        );
        if let Err(error) = insert_result {
            let _ = fs::remove_file(&destination);
            return Err(error.into());
        }
    }

    let result = backup::verify_backup(
        connection,
        config,
        BackupReferenceRequest {
            backup_id: header.backup_id.clone(),
            passphrase: Some(passphrase),
            confirmation: None,
        },
    );
    match result {
        Ok(mut result) => {
            result.message = "Portable recovery package imported and fully verified.".to_owned();
            Ok(result)
        }
        Err(error) => {
            if is_new_record {
                let _ = connection.execute(
                    "DELETE FROM backup_records WHERE backup_id=?1",
                    params![header.backup_id],
                );
                if imported_new_file {
                    let _ = fs::remove_file(&destination);
                }
            } else {
                let _ = database::mark_backup_failed(
                    connection,
                    &header.backup_id,
                    "recovery_import_verification_failed",
                );
                if imported_new_file {
                    let _ = fs::remove_file(&destination);
                }
            }
            Err(error)
        }
    }
}

pub fn package_for_export(
    connection: &Connection,
    config: &AppConfig,
    backup_id: &str,
) -> Result<ExportPackage> {
    Uuid::parse_str(backup_id).context("backup identity is invalid")?;
    let record = database::backup_by_id(connection, backup_id)?;
    ensure!(
        record.kind == BackupKind::Recovery,
        "only portable recovery packages can be exported"
    );
    ensure!(
        matches!(
            record.state,
            BackupState::Ready
                | BackupState::Verified
                | BackupState::RestoreStaged
                | BackupState::Restored
        ),
        "recovery package is not ready for export"
    );
    let path = PathBuf::from(&record.storage_path);
    let canonical_path = path
        .canonicalize()
        .context("recovery package is unavailable")?;
    let canonical_root = config.recovery_dir.canonicalize()?;
    ensure!(
        canonical_path.starts_with(&canonical_root),
        "recovery package is outside managed HomeServer storage"
    );
    let size_bytes = fs::metadata(&canonical_path)?.len();
    ensure!(
        size_bytes > 12 && size_bytes <= MAX_PACKAGE_BYTES,
        "recovery package size is invalid"
    );
    Ok(ExportPackage {
        path: canonical_path,
        file_name: record.file_name,
        size_bytes,
    })
}

fn read_header(path: &Path) -> Result<ImportedHeader> {
    let mut input = File::open(path)?;
    let mut prefix = [0_u8; 12];
    input.read_exact(&mut prefix)?;
    ensure!(
        &prefix[..8] == PACKAGE_MAGIC,
        "recovery package magic is invalid"
    );
    let header_length = u32::from_be_bytes(prefix[8..12].try_into()?) as usize;
    ensure!(
        header_length > 0 && header_length <= MAX_HEADER_BYTES,
        "recovery package header length is invalid"
    );
    let package_size = fs::metadata(path)?.len() as usize;
    ensure!(
        package_size > 12 + header_length,
        "recovery package is truncated"
    );
    let mut header = vec![0_u8; header_length];
    input.read_exact(&mut header)?;
    serde_json::from_slice(&header).context("recovery package header is invalid")
}

fn move_replace(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, destination)?;
            File::options().write(true).open(destination)?.sync_all()?;
            fs::remove_file(source)?;
            Ok(())
        }
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("recovery package {label} hash is invalid");
    }
    Ok(())
}
