use crate::config::AppConfig;
use anyhow::{ensure, Context, Result};
use rand::{rngs::OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::{Path, PathBuf}};
use zeroize::Zeroize;

const BACKUP_KEY_BYTES: usize = 32;
const MAX_PROTECTED_KEY_BYTES: usize = 16 * 1024;

pub fn load_or_create(config: &AppConfig, installation_id: &str) -> Result<[u8; BACKUP_KEY_BYTES]> {
    let path = key_path(config);
    if path.exists() {
        return load(config, installation_id);
    }

    let mut key = [0_u8; BACKUP_KEY_BYTES];
    OsRng.fill_bytes(&mut key);
    let protected = protect(&key, installation_id)?;
    ensure!(
        protected.len() <= MAX_PROTECTED_KEY_BYTES,
        "protected HomeServer backup key is unexpectedly large"
    );
    write_atomic(&path, &protected)?;
    Ok(key)
}

pub fn load(config: &AppConfig, installation_id: &str) -> Result<[u8; BACKUP_KEY_BYTES]> {
    let path = key_path(config);
    recover_interrupted_replace(&path)?;
    let protected =
        fs::read(path).context("HomeServer backup encryption key is unavailable")?;
    ensure!(
        !protected.is_empty() && protected.len() <= MAX_PROTECTED_KEY_BYTES,
        "HomeServer backup encryption key is invalid"
    );
    let decrypted = unprotect(&protected, installation_id)?;
    ensure!(
        decrypted.len() == BACKUP_KEY_BYTES,
        "HomeServer backup encryption key is invalid"
    );
    let mut key = [0_u8; BACKUP_KEY_BYTES];
    key.copy_from_slice(&decrypted);
    Ok(key)
}

fn key_path(config: &AppConfig) -> PathBuf {
    config.data_dir.join("secrets").join("backup-key.dpapi")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path
        .parent()
        .context("HomeServer backup key directory is unavailable")?;
    fs::create_dir_all(directory)?;
    recover_interrupted_replace(path)?;
    let temporary = path.with_extension("dpapi.tmp");
    let backup = path.with_extension("dpapi.replace-backup");
    let mut output = fs::File::create(&temporary)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);

    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    if path.exists() {
        fs::rename(path, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    if backup.exists() {
        fs::remove_file(backup)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn recover_interrupted_replace(path: &Path) -> Result<()> {
    let backup = path.with_extension("dpapi.replace-backup");
    if !path.exists() && backup.exists() {
        fs::rename(backup, path)?;
    } else if path.exists() && backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
}

fn entropy(installation_id: &str) -> [u8; 32] {
    Sha256::digest(format!("MicrogifterHomeServerBackup:{installation_id}").as_bytes()).into()
}

#[cfg(windows)]
fn protect(value: &[u8], installation_id: &str) -> Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
            CRYPT_INTEGER_BLOB,
        },
    };

    let mut value = value.to_vec();
    let mut entropy = entropy(installation_id).to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: value.len() as u32,
        pbData: value.as_mut_ptr(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };

    let succeeded = unsafe {
        CryptProtectData(
            &input,
            null(),
            &entropy_blob,
            null_mut(),
            null(),
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    value.zeroize();
    entropy.zeroize();
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error())
            .context("Windows DPAPI could not protect the HomeServer backup key");
    }

    let protected = unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(output.pbData.cast());
        bytes
    };
    Ok(protected)
}

#[cfg(windows)]
fn unprotect(value: &[u8], installation_id: &str) -> Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };

    let mut value = value.to_vec();
    let mut entropy = entropy(installation_id).to_vec();
    let input = CRYPT_INTEGER_BLOB {
        cbData: value.len() as u32,
        pbData: value.as_mut_ptr(),
    };
    let entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: entropy.len() as u32,
        pbData: entropy.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };

    let succeeded = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            &entropy_blob,
            null_mut(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    value.zeroize();
    entropy.zeroize();
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error())
            .context("Windows DPAPI could not decrypt the HomeServer backup key");
    }

    let decrypted = unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(output.pbData.cast());
        bytes
    };
    Ok(decrypted)
}

#[cfg(not(windows))]
fn protect(_value: &[u8], _installation_id: &str) -> Result<Vec<u8>> {
    anyhow::bail!("machine-scoped HomeServer backup keys are only supported on Windows")
}

#[cfg(not(windows))]
fn unprotect(_value: &[u8], _installation_id: &str) -> Result<Vec<u8>> {
    anyhow::bail!("machine-scoped HomeServer backup keys are only supported on Windows")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_is_stable_and_installation_specific() {
        assert_eq!(entropy("one"), entropy("one"));
        assert_ne!(entropy("one"), entropy("two"));
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_never_obscures_keys_as_plaintext() {
        assert!(protect(&[7_u8; BACKUP_KEY_BYTES], "test").is_err());
        assert!(unprotect(&[7_u8; BACKUP_KEY_BYTES], "test").is_err());
    }
}
