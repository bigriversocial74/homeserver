use anyhow::{Context, Result};
#[cfg(not(windows))]
use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
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

        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("unable to create {}", data_dir.display()))?;

        let server_name = std::env::var("MG_HOMESERVER_NAME")
            .unwrap_or_else(|_| "Microgifter HomeServer".to_owned());

        Ok(Self {
            database_path: data_dir.join("homeserver.sqlite3"),
            data_dir,
            server_name,
        })
    }
}
