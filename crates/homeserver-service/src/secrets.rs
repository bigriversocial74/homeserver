use anyhow::{Context, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

const CREDENTIAL_SERVICE: &str = "MicrogifterHomeServer";

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceSecrets {
    pub device_token: String,
    pub signing_key_base64: String,
}

impl Drop for DeviceSecrets {
    fn drop(&mut self) {
        self.device_token.zeroize();
        self.signing_key_base64.zeroize();
    }
}

fn entry(installation_id: &str) -> Result<Entry> {
    Entry::new(CREDENTIAL_SERVICE, installation_id)
        .context("unable to open the HomeServer operating-system credential vault")
}

pub fn save(installation_id: &str, secrets: &DeviceSecrets) -> Result<()> {
    let payload = serde_json::to_string(secrets)?;
    entry(installation_id)?
        .set_password(&payload)
        .context("unable to save HomeServer cloud credentials")
}

pub fn load(installation_id: &str) -> Result<DeviceSecrets> {
    let payload = entry(installation_id)?
        .get_password()
        .context("HomeServer cloud credentials are unavailable")?;
    serde_json::from_str(&payload).context("HomeServer cloud credentials are invalid")
}

pub fn delete(installation_id: &str) -> Result<()> {
    match entry(installation_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to delete HomeServer cloud credentials"),
    }
}
