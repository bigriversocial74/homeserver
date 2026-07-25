use sha2::{Digest, Sha256};
use std::{
    error::Error as StdError,
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroize;

const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_PROTECTED_BYTES: usize = 256 * 1024;

#[derive(Debug)]
pub enum Error {
    NoEntry,
    Invalid(String),
    PlatformFailure(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoEntry => formatter.write_str("credential entry does not exist"),
            Self::Invalid(message) | Self::PlatformFailure(message) => formatter.write_str(message),
        }
    }
}

impl StdError for Error {}

#[derive(Debug, Clone)]
pub struct Entry {
    service: String,
    username: String,
}

impl Entry {
    pub fn new(service: &str, username: &str) -> Result<Self, Error> {
        let service = validate_component("service", service)?;
        let username = validate_component("username", username)?;
        Ok(Self { service, username })
    }

    pub fn set_password(&self, password: &str) -> Result<(), Error> {
        if password.len() > MAX_SECRET_BYTES {
            return Err(Error::Invalid(
                "credential payload exceeds the HomeServer size limit".to_owned(),
            ));
        }

        let mut protected = protect(password.as_bytes(), &self.entropy())?;
        if protected.is_empty() || protected.len() > MAX_PROTECTED_BYTES {
            protected.zeroize();
            return Err(Error::PlatformFailure(
                "protected credential payload is invalid".to_owned(),
            ));
        }

        let result = write_atomic(&self.path()?, &protected);
        protected.zeroize();
        result
    }

    pub fn get_password(&self) -> Result<String, Error> {
        let path = self.path()?;
        recover_interrupted_replace(&path)?;
        let mut protected = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NoEntry)
            }
            Err(error) => {
                return Err(platform_error(
                    "unable to read the HomeServer credential file",
                    error,
                ))
            }
        };

        if protected.is_empty() || protected.len() > MAX_PROTECTED_BYTES {
            protected.zeroize();
            return Err(Error::PlatformFailure(
                "HomeServer credential file is invalid".to_owned(),
            ));
        }

        let decrypted = unprotect(&protected, &self.entropy());
        protected.zeroize();
        let decrypted = decrypted?;
        String::from_utf8(decrypted).map_err(|error| {
            let mut bytes = error.into_bytes();
            bytes.zeroize();
            Error::PlatformFailure("HomeServer credential data is not valid UTF-8".to_owned())
        })
    }

    pub fn delete_credential(&self) -> Result<(), Error> {
        let path = self.path()?;
        let backup = replacement_backup_path(&path);
        let mut removed = false;
        for candidate in [&path, &backup] {
            match fs::remove_file(candidate) {
                Ok(()) => removed = true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(platform_error(
                        "unable to delete the HomeServer credential file",
                        error,
                    ))
                }
            }
        }
        if removed {
            Ok(())
        } else {
            Err(Error::NoEntry)
        }
    }

    fn entropy(&self) -> [u8; 32] {
        Sha256::digest(
            format!(
                "MicrogifterHomeServerCredential:{}\0{}",
                self.service, self.username
            )
            .as_bytes(),
        )
        .into()
    }

    fn path(&self) -> Result<PathBuf, Error> {
        let digest = Sha256::digest(format!("{}\0{}", self.service, self.username).as_bytes());
        let filename = format!("credential-{}.dpapi", lower_hex(&digest));
        Ok(data_root()?.join("secrets").join("cloud").join(filename))
    }
}

fn validate_component(label: &str, value: &str) -> Result<String, Error> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(Error::Invalid(format!(
            "credential {label} is empty, too long, or contains control characters"
        )));
    }
    Ok(value.to_owned())
}

fn data_root() -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os("MG_HOMESERVER_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }

    #[cfg(windows)]
    {
        std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Microgifter").join("HomeServer"))
            .ok_or_else(|| {
                Error::PlatformFailure(
                    "Windows ProgramData is unavailable for HomeServer credentials".to_owned(),
                )
            })
    }

    #[cfg(not(windows))]
    {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
            return Ok(PathBuf::from(path).join("Microgifter").join("HomeServer"));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| {
                home.join(".local")
                    .join("share")
                    .join("Microgifter")
                    .join("HomeServer")
            })
            .ok_or_else(|| {
                Error::PlatformFailure(
                    "HomeServer data directory is unavailable for credentials".to_owned(),
                )
            })
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let directory = path.parent().ok_or_else(|| {
        Error::PlatformFailure("HomeServer credential directory is unavailable".to_owned())
    })?;
    fs::create_dir_all(directory).map_err(|error| {
        platform_error(
            "unable to create the HomeServer credential directory",
            error,
        )
    })?;

    recover_interrupted_replace(path)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temporary = path.with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    let backup = replacement_backup_path(path);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut output = options
        .open(&temporary)
        .map_err(|error| platform_error("unable to create a credential staging file", error))?;
    if let Err(error) = output.write_all(bytes).and_then(|()| output.sync_all()) {
        drop(output);
        let _ = fs::remove_file(&temporary);
        return Err(platform_error(
            "unable to write the HomeServer credential file",
            error,
        ));
    }
    drop(output);

    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            platform_error("unable to clear a stale credential replacement backup", error)
        })?;
    }
    if path.exists() {
        fs::rename(path, &backup).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            platform_error("unable to preserve the current HomeServer credential", error)
        })?;
    }

    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(platform_error(
            "unable to activate the HomeServer credential file",
            error,
        ));
    }
    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            platform_error("unable to remove the credential replacement backup", error)
        })?;
    }
    Ok(())
}

fn replacement_backup_path(path: &Path) -> PathBuf {
    path.with_extension("replace-backup")
}

fn recover_interrupted_replace(path: &Path) -> Result<(), Error> {
    let backup = replacement_backup_path(path);
    if !path.exists() && backup.exists() {
        fs::rename(&backup, path).map_err(|error| {
            platform_error("unable to recover an interrupted credential replacement", error)
        })?;
    } else if path.exists() && backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            platform_error("unable to clear an obsolete credential replacement backup", error)
        })?;
    }
    Ok(())
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn platform_error(context: &str, error: impl fmt::Display) -> Error {
    Error::PlatformFailure(format!("{context}: {error}"))
}

#[cfg(windows)]
fn protect(value: &[u8], entropy: &[u8; 32]) -> Result<Vec<u8>, Error> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
            CRYPT_INTEGER_BLOB,
        },
    };

    let mut value = value.to_vec();
    let mut entropy = entropy.to_vec();
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
        return Err(platform_error(
            "Windows DPAPI could not protect the HomeServer cloud credential",
            std::io::Error::last_os_error(),
        ));
    }

    let protected = unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(output.pbData.cast());
        bytes
    };
    Ok(protected)
}

#[cfg(windows)]
fn unprotect(value: &[u8], entropy: &[u8; 32]) -> Result<Vec<u8>, Error> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };

    let mut value = value.to_vec();
    let mut entropy = entropy.to_vec();
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
        return Err(platform_error(
            "Windows DPAPI could not decrypt the HomeServer cloud credential",
            std::io::Error::last_os_error(),
        ));
    }

    let decrypted = unsafe {
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(output.pbData.cast());
        bytes
    };
    Ok(decrypted)
}

#[cfg(not(windows))]
fn protect(_value: &[u8], _entropy: &[u8; 32]) -> Result<Vec<u8>, Error> {
    Err(Error::PlatformFailure(
        "machine-scoped HomeServer credentials are only supported on Windows".to_owned(),
    ))
}

#[cfg(not(windows))]
fn unprotect(_value: &[u8], _entropy: &[u8; 32]) -> Result<Vec<u8>, Error> {
    Err(Error::PlatformFailure(
        "machine-scoped HomeServer credentials are only supported on Windows".to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_names_are_stable_and_isolated() {
        let first = Entry::new("MicrogifterHomeServer", "installation-one").unwrap();
        let same = Entry::new("MicrogifterHomeServer", "installation-one").unwrap();
        let second = Entry::new("MicrogifterHomeServer", "installation-two").unwrap();
        assert_eq!(first.entropy(), same.entropy());
        assert_ne!(first.entropy(), second.entropy());
    }

    #[test]
    fn invalid_components_are_rejected() {
        assert!(Entry::new("", "user").is_err());
        assert!(Entry::new("service", "bad\nuser").is_err());
    }

    #[test]
    fn interrupted_replacement_uses_a_separate_recovery_path() {
        let path = PathBuf::from("credential-example.dpapi");
        assert_ne!(replacement_backup_path(&path), path);
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_platform_never_falls_back_to_plaintext() {
        assert!(protect(b"secret", &[0_u8; 32]).is_err());
        assert!(unprotect(b"secret", &[0_u8; 32]).is_err());
    }
}
