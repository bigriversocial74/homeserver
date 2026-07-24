use anyhow::{bail, Context, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
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

pub fn self_test(installation_id: &str) -> Result<()> {
    let diagnostic_user = format!("{installation_id}:diagnostic:{}", Uuid::new_v4().simple());
    let diagnostic_entry = Entry::new(CREDENTIAL_SERVICE, &diagnostic_user)
        .context("unable to open a diagnostic operating-system credential entry")?;
    let mut secret = format!("homeserver-vault-test:{}", Uuid::new_v4().simple());

    let result = (|| -> Result<()> {
        diagnostic_entry
            .set_password(&secret)
            .context("unable to write a diagnostic operating-system credential")?;
        let stored = diagnostic_entry
            .get_password()
            .context("unable to read the diagnostic operating-system credential")?;
        if stored != secret {
            bail!("operating-system credential vault returned mismatched data");
        }
        Ok(())
    })();

    let delete_result = match diagnostic_entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error).context("unable to delete the diagnostic credential"),
    };
    secret.zeroize();
    result.and(delete_result)
}
