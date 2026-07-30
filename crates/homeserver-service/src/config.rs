use anyhow::{ensure, Context, Result};
#[cfg(not(windows))]
use directories::ProjectDirs;
use std::path::PathBuf;
use url::Url;

const DEFAULT_SERVER_NAME: &str = "Microgifter HomeServer";
const DEFAULT_UPDATE_MANIFEST_URL: &str =
    "https://updates.microgifter.com/homeserver/stable/manifest.json";
const DEFAULT_VP3_BASE_URL: &str = "https://vp3.me";
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
    pub updates_dir: PathBuf,
    pub update_staging_dir: PathBuf,
    pub update_rollback_dir: PathBuf,
    pub update_installed_dir: PathBuf,
    pub update_manifest_url: String,
    pub vp3_base_url: String,
    pub vp3_lease_public_key_base64: String,
    pub vp3_lease_key_id: String,
    pub vp3_release_public_key_base64: String,
    pub vp3_release_key_id: String,
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
        let updates_dir = data_dir.join("updates");
        let update_staging_dir = updates_dir.join("staging");
        let update_rollback_dir = updates_dir.join("rollback");
        let update_installed_dir = updates_dir.join("installed");
        for directory in [
            &data_dir,
            &logs_dir,
            &backups_dir,
            &recovery_dir,
            &restore_dir,
            &staging_dir,
            &imports_dir,
            &updates_dir,
            &update_staging_dir,
            &update_rollback_dir,
            &update_installed_dir,
        ] {
            std::fs::create_dir_all(directory)
                .with_context(|| format!("unable to create {}", directory.display()))?;
        }

        let raw_server_name =
            std::env::var("MG_HOMESERVER_NAME").unwrap_or_else(|_| DEFAULT_SERVER_NAME.to_owned());
        let server_name = sanitize_server_name(&raw_server_name);
        let update_manifest_url = std::env::var("MG_HOMESERVER_UPDATE_MANIFEST_URL")
            .unwrap_or_else(|_| DEFAULT_UPDATE_MANIFEST_URL.to_owned());
        validate_https_url(&update_manifest_url, "HomeServer update manifest")?;
        let vp3_base_url = std::env::var("MG_HOMESERVER_VP3_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_VP3_BASE_URL.to_owned());
        validate_https_url(&vp3_base_url, "VP3 software authority")?;
        let vp3_base_url = vp3_base_url.trim_end_matches('/').to_owned();

        Ok(Self {
            database_path: data_dir.join("homeserver.sqlite3"),
            data_dir,
            logs_dir,
            backups_dir,
            recovery_dir,
            restore_dir,
            staging_dir,
            imports_dir,
            updates_dir,
            update_staging_dir,
            update_rollback_dir,
            update_installed_dir,
            update_manifest_url,
            vp3_base_url,
            vp3_lease_public_key_base64: std::env::var("MG_HOMESERVER_VP3_LEASE_PUBLIC_KEY_BASE64")
                .unwrap_or_default(),
            vp3_lease_key_id: std::env::var("MG_HOMESERVER_VP3_LEASE_KEY_ID")
                .unwrap_or_else(|_| "homeserver-lease-ed25519-v1".to_owned()),
            vp3_release_public_key_base64: std::env::var(
                "MG_HOMESERVER_VP3_RELEASE_PUBLIC_KEY_BASE64",
            )
            .unwrap_or_default(),
            vp3_release_key_id: std::env::var("MG_HOMESERVER_VP3_RELEASE_KEY_ID")
                .unwrap_or_else(|_| "release-ed25519-v1".to_owned()),
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
        self.imports_dir.join(format!(
            "recovery-import-{}.mghbackup",
            uuid::Uuid::new_v4().simple()
        ))
    }

    pub fn update_plan_path(&self) -> PathBuf {
        self.update_staging_dir.join("pending-update.json")
    }

    pub fn update_result_path(&self) -> PathBuf {
        self.updates_dir.join("last-update-result.json")
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

fn validate_https_url(value: &str, label: &str) -> Result<()> {
    let url = Url::parse(value).with_context(|| format!("{label} URL is invalid"))?;
    ensure!(url.scheme() == "https", "{label} URL must use HTTPS");
    ensure!(url.host_str().is_some(), "{label} URL host is missing");
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "{label} URL cannot contain credentials"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "{label} base URL cannot contain a query or fragment"
    );
    Ok(())
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

    #[test]
    fn authority_urls_must_use_https() {
        assert!(validate_https_url(DEFAULT_UPDATE_MANIFEST_URL, "manifest").is_ok());
        assert!(validate_https_url(DEFAULT_VP3_BASE_URL, "VP3").is_ok());
        assert!(validate_https_url("http://vp3.me", "VP3").is_err());
        assert!(validate_https_url("https://user:secret@example.com", "VP3").is_err());
        assert!(validate_https_url("https://vp3.me?override=1", "VP3").is_err());
    }
}
