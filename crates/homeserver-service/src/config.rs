use anyhow::{Context, Result};
#[cfg(not(windows))]
use directories::ProjectDirs;
use std::path::PathBuf;

const DEFAULT_SERVER_NAME: &str = "Microgifter HomeServer";
const MAX_SERVER_NAME_CHARS: usize = 128;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub logs_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub recovery_dir: PathBuf,
    pub restore_dir: PathBuf,
    pub staging_dir: PathBuf,
    pub imports_dir: PathBuf,
    pub server_name: String,
}

impl AppConfig {
    pub fn load() -> Result<Self> {
        let data_dir = std::env::var_os("MG_HOMESERVER_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                #[cfg(windows)]
                {
                    std::env::var_os("PROGRAMDATA")
                        .map(PathBuf::from)
                        .map(|root| root.join("Microgifter").join("HomeServer"))
                }
                #[cfg(not(windows))]
                {
                    ProjectDirs::from("com", "Microgifter", "HomeServer")
                        .map(|dirs| dirs.data_dir().to_path_buf())
                }
            })
            .context("unable to resolve the HomeServer data directory")?;

        let logs_dir = data_dir.join("logs");
        let backups_dir = data_dir.join("backups");
        let recovery_dir = data_dir.join("recovery-packages");
        let restore_dir = data_dir.join("restore");
        let staging_dir = data_dir.join("staging");
        let imports_dir = staging_dir.join("recovery-imports");
        for directory in [
            &data_dir,
            &logs_dir,
            &backups_dir,
            &recovery_dir,
            &restore_dir,
            &staging_dir,
            &imports_dir,
        ] {
            std::fs::create_dir_all(directory)
                .with_context(|| format!("unable to create {}", directory.display()))?;
        }

        let raw_server_name =
            std::env::var("MG_HOMESERVER_NAME").unwrap_or_else(|_| DEFAULT_SERVER_NAME.to_owned());
        let server_name = sanitize_server_name(&raw_server_name);

        Ok(Self {
            database_path: data_dir.join("homeserver.sqlite3"),
            data_dir,
            logs_dir,
            backups_dir,
            recovery_dir,
            restore_dir,
            staging_dir,
            imports_dir,
            server_name,
        })
    }

    pub fn pending_restore_plan_path(&self) -> PathBuf {
        self.restore_dir.join("pending-restore.json")
    }

    pub fn pending_restore_database_path(&self) -> PathBuf {
        self.restore_dir.join("pending-restore.sqlite3")
    }

    pub fn new_import_path(&self) -> PathBuf {
        self.imports_dir
            .join(format!("recovery-import-{}.mghbackup", uuid::Uuid::new_v4().simple()))
    }
}

fn sanitize_server_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_SERVER_NAME_CHARS)
        .collect();
    let cleaned = cleaned.trim();

    if cleaned.is_empty() {
        DEFAULT_SERVER_NAME.to_owned()
    } else {
        cleaned.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_name_removes_control_characters_and_whitespace() {
        assert_eq!(sanitize_server_name("  Office\nServer\t  "), "OfficeServer");
    }

    #[test]
    fn empty_server_name_uses_product_default() {
        assert_eq!(sanitize_server_name(" \n\t "), DEFAULT_SERVER_NAME);
    }

    #[test]
    fn server_name_is_length_bounded() {
        let long_name = "x".repeat(MAX_SERVER_NAME_CHARS + 20);
        assert_eq!(
            sanitize_server_name(&long_name).chars().count(),
            MAX_SERVER_NAME_CHARS
        );
    }
}
