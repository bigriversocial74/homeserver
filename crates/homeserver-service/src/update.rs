use crate::config::AppConfig;
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{Duration as ChronoDuration, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use microgifter_homeserver_core::{
    SignedUpdateManifest, UpdateApplicationPlan, UpdateApplicationResult, UpdateManifestPayload,
    UpdateRecord, UpdateState, PRODUCT_NAME, UPDATE_KEY_ID, UPDATE_MANIFEST_SCHEMA_VERSION,
};
use semver::Version;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use url::Url;

const PINNED_UPDATE_PUBLIC_KEY_BASE64: &str = "nzuIihsgbLnkpjq217CZZm6v8eD9YKBMrLOOTC3jeRc=";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;
const MIN_INSTALLER_BYTES: u64 = 1_000_000;
const MAX_RELEASE_NOTES_CHARS: usize = 20_000;
const UPDATER_RESOURCE_NAME: &str = "microgifter-homeserver-updater.exe";

#[derive(Debug, Clone)]
pub struct VerifiedUpdate {
    pub update_id: String,
    pub manifest: SignedUpdateManifest,
}

pub async fn fetch_and_verify_manifest(
    config: &AppConfig,
    current_version: &str,
) -> Result<VerifiedUpdate> {
    let manifest_url = secure_https_url(&config.update_manifest_url, "update manifest")?;
    let response = secure_client(Duration::from_secs(20))?
        .get(manifest_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("unable to request the HomeServer update manifest")?
        .error_for_status()
        .context("HomeServer update manifest request was rejected")?;

    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_MANIFEST_BYTES as u64,
            "update manifest exceeds the size limit"
        );
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("unable to read the update manifest response")?;
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_MANIFEST_BYTES,
            "update manifest exceeds the size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    ensure!(!bytes.is_empty(), "update manifest is empty");
    let manifest: SignedUpdateManifest =
        serde_json::from_slice(&bytes).context("update manifest JSON is invalid")?;
    verify_manifest(&manifest, current_version)
}

pub fn verify_manifest(
    manifest: &SignedUpdateManifest,
    current_version: &str,
) -> Result<VerifiedUpdate> {
    let public_key = decode_pinned_public_key()?;
    verify_manifest_with_key(manifest, current_version, &public_key)
}

fn verify_manifest_with_key(
    manifest: &SignedUpdateManifest,
    current_version: &str,
    public_key: &VerifyingKey,
) -> Result<VerifiedUpdate> {
    ensure!(
        manifest.key_id == UPDATE_KEY_ID,
        "update manifest signing key is not trusted"
    );
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(&manifest.signature)
        .context("update manifest signature encoding is invalid")?;
    let signature = Signature::from_slice(&signature_bytes)
        .context("update manifest signature length is invalid")?;
    let canonical_payload = serde_json::to_vec(&manifest.payload)?;
    public_key
        .verify(&canonical_payload, &signature)
        .context("update manifest signature verification failed")?;
    validate_payload(&manifest.payload, current_version)?;

    let update_id = format!(
        "update:{}:{}",
        manifest.payload.version,
        &manifest.payload.installer.sha256[..16]
    );
    Ok(VerifiedUpdate {
        update_id,
        manifest: manifest.clone(),
    })
}

fn validate_payload(payload: &UpdateManifestPayload, current_version: &str) -> Result<()> {
    ensure!(
        payload.schema_version == UPDATE_MANIFEST_SCHEMA_VERSION,
        "unsupported update manifest schema"
    );
    ensure!(
        payload.product == PRODUCT_NAME,
        "update manifest product is invalid"
    );
    ensure!(
        payload.channel == microgifter_homeserver_core::UpdateChannel::Stable,
        "only stable HomeServer updates are supported"
    );
    let current =
        Version::parse(current_version).context("current HomeServer version is invalid")?;
    let target = Version::parse(&payload.version).context("update target version is invalid")?;
    ensure!(
        target.pre.is_empty(),
        "stable update cannot contain a prerelease version"
    );
    if let Some(minimum) = &payload.minimum_version {
        let minimum = Version::parse(minimum).context("minimum update version is invalid")?;
        ensure!(
            current >= minimum,
            "this update requires a newer HomeServer upgrade baseline"
        );
    }
    ensure!(
        payload.published_at_utc <= Utc::now() + ChronoDuration::minutes(10),
        "update manifest publication time is in the future"
    );
    ensure!(
        payload.release_notes.chars().count() <= MAX_RELEASE_NOTES_CHARS,
        "update release notes exceed the size limit"
    );
    ensure!(
        payload.installer.file_name == "Microgifter-HomeServer-Setup.exe",
        "update installer filename is invalid"
    );
    ensure!(
        (MIN_INSTALLER_BYTES..=MAX_INSTALLER_BYTES).contains(&payload.installer.size_bytes),
        "update installer size is outside the supported range"
    );
    ensure!(
        valid_sha256(&payload.installer.sha256),
        "update installer SHA-256 is invalid"
    );
    ensure!(
        valid_thumbprint(&payload.installer.authenticode_thumbprint),
        "update Authenticode thumbprint is invalid"
    );
    secure_https_url(&payload.installer.url, "update installer")?;
    Ok(())
}

pub fn manifest_is_newer(verified: &VerifiedUpdate, current_version: &str) -> Result<bool> {
    let current = Version::parse(current_version)?;
    let target = Version::parse(&verified.manifest.payload.version)?;
    Ok(target > current)
}

pub async fn download_and_verify_installer(
    config: &AppConfig,
    record: &UpdateRecord,
    manifest: &SignedUpdateManifest,
) -> Result<PathBuf> {
    ensure!(
        record.state == UpdateState::Available || record.state == UpdateState::Downloading,
        "update is not available for download"
    );
    ensure!(
        record.version == manifest.payload.version,
        "update record and manifest version do not match"
    );
    let url = secure_https_url(&manifest.payload.installer.url, "update installer")?;
    let destination = config.update_staging_dir.join(format!(
        "{}-{}",
        record.update_id.replace(':', "-"),
        manifest.payload.installer.file_name
    ));
    let temporary = destination.with_extension("part");
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }

    let response = secure_client(Duration::from_secs(15 * 60))?
        .get(url)
        .header(reqwest::header::ACCEPT, "application/octet-stream")
        .send()
        .await
        .context("unable to download the HomeServer update installer")?
        .error_for_status()
        .context("HomeServer update installer request was rejected")?;
    if let Some(length) = response.content_length() {
        ensure!(
            length == manifest.payload.installer.size_bytes,
            "update installer response size does not match the signed manifest"
        );
    }

    let mut output = tokio::fs::File::create(&temporary)
        .await
        .context("unable to create staged update installer")?;
    let mut stream = response.bytes_stream();
    let mut size = 0_u64;
    let mut hasher = Sha256::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("unable to read update installer response")?;
        size = size
            .checked_add(chunk.len() as u64)
            .context("update installer size overflow")?;
        ensure!(
            size <= manifest.payload.installer.size_bytes && size <= MAX_INSTALLER_BYTES,
            "update installer exceeds the signed size"
        );
        tokio::io::AsyncWriteExt::write_all(&mut output, &chunk).await?;
        hasher.update(&chunk);
    }
    tokio::io::AsyncWriteExt::sync_all(&mut output).await?;
    drop(output);

    let hash = hex::encode(hasher.finalize());
    ensure!(
        size == manifest.payload.installer.size_bytes,
        "update installer is truncated"
    );
    ensure!(
        hash.eq_ignore_ascii_case(&manifest.payload.installer.sha256),
        "update installer SHA-256 does not match the signed manifest"
    );
    verify_authenticode(
        &temporary,
        &manifest.payload.installer.authenticode_thumbprint,
    )?;
    if destination.exists() {
        fs::remove_file(&destination)?;
    }
    fs::rename(&temporary, &destination)?;
    Ok(destination)
}

pub fn consume_application_result(config: &AppConfig) -> Result<Option<UpdateApplicationResult>> {
    let path = config.update_result_path();
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(&path)?;
    ensure!(
        metadata.len() > 2 && metadata.len() <= 64 * 1024,
        "update result size is invalid"
    );
    let result: UpdateApplicationResult = serde_json::from_slice(&fs::read(&path)?)
        .context("update application result is invalid")?;
    ensure!(
        result.schema_version == UPDATE_MANIFEST_SCHEMA_VERSION,
        "unsupported update result schema"
    );
    fs::remove_file(path)?;
    Ok(Some(result))
}

fn secure_client(timeout: Duration) -> Result<reqwest::Client> {
    let redirect = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 5 {
            return attempt.error("too many update redirects");
        }
        if attempt.url().scheme() != "https" {
            return attempt.error("update redirect downgraded from HTTPS");
        }
        attempt.follow()
    });
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(redirect)
        .user_agent(format!(
            "Microgifter-HomeServer/{}",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("unable to create the HomeServer update client")
}

fn secure_https_url(value: &str, label: &str) -> Result<Url> {
    let url = Url::parse(value).with_context(|| format!("{label} URL is invalid"))?;
    ensure!(url.scheme() == "https", "{label} URL must use HTTPS");
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "{label} URL cannot contain credentials"
    );
    ensure!(url.host_str().is_some(), "{label} URL host is missing");
    Ok(url)
}

fn decode_pinned_public_key() -> Result<VerifyingKey> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(PINNED_UPDATE_PUBLIC_KEY_BASE64)
        .context("pinned update public key is invalid")?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("pinned update public key length is invalid"))?;
    VerifyingKey::from_bytes(&bytes).context("pinned update public key is invalid")
}

fn verify_file(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<()> {
    let metadata = fs::metadata(path)?;
    ensure!(metadata.is_file(), "staged installer is not a regular file");
    ensure!(
        metadata.len() == expected_size,
        "staged installer size does not match the signed manifest"
    );
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    ensure!(
        hex::encode(hasher.finalize()).eq_ignore_ascii_case(expected_sha256),
        "staged installer SHA-256 does not match the signed manifest"
    );
    Ok(())
}

#[cfg(windows)]
pub fn verify_authenticode(path: &Path, expected_thumbprint: &str) -> Result<()> {
    let script = r#"$signature = Get-AuthenticodeSignature -LiteralPath $env:MG_UPDATE_FILE
if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate) { exit 20 }
[Console]::Out.Write($signature.SignerCertificate.Thumbprint)"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("MG_UPDATE_FILE", path)
        .output()
        .context("unable to execute Windows Authenticode verification")?;
    ensure!(
        output.status.success(),
        "installer does not have a valid trusted Authenticode signature"
    );
    let actual = String::from_utf8(output.stdout)?.trim().replace(' ', "");
    ensure!(
        actual.eq_ignore_ascii_case(expected_thumbprint),
        "installer Authenticode signer does not match the signed manifest"
    );
    Ok(())
}

#[cfg(not(windows))]
pub fn verify_authenticode(_path: &Path, _expected_thumbprint: &str) -> Result<()> {
    bail!("Authenticode verification is only supported on Windows")
}

#[cfg(windows)]
fn launch_detached(updater: &Path, plan_path: &Path) -> Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
    Command::new(updater)
        .arg("apply")
        .arg(plan_path)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .context("unable to launch the HomeServer updater helper")?;
    Ok(())
}

#[cfg(not(windows))]
fn launch_detached(_updater: &Path, _plan_path: &Path) -> Result<()> {
    bail!("HomeServer update application is only supported on Windows")
}

fn atomic_write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn valid_thumbprint(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|character| character.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use microgifter_homeserver_core::UpdateChannel;

    fn signed_manifest(version: &str) -> (SignedUpdateManifest, VerifyingKey) {
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let payload = UpdateManifestPayload {
            schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
            product: PRODUCT_NAME.to_owned(),
            channel: UpdateChannel::Stable,
            version: version.to_owned(),
            minimum_version: Some("0.1.0".to_owned()),
            published_at_utc: Utc::now(),
            release_notes: "Signed update test".to_owned(),
            installer: microgifter_homeserver_core::UpdateInstallerContract {
                url: "https://updates.microgifter.com/HomeServer.exe".to_owned(),
                file_name: "Microgifter-HomeServer-Setup.exe".to_owned(),
                size_bytes: 5_000_000,
                sha256: "a".repeat(64),
                authenticode_thumbprint: "B".repeat(40),
            },
        };
        let signature = key.sign(&serde_json::to_vec(&payload).unwrap());
        (
            SignedUpdateManifest {
                key_id: UPDATE_KEY_ID.to_owned(),
                payload,
                signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
            },
            key.verifying_key(),
        )
    }

    #[test]
    fn valid_signed_manifest_is_accepted() {
        let (manifest, public_key) = signed_manifest("0.2.0");
        let verified = verify_manifest_with_key(&manifest, "0.1.0", &public_key).unwrap();
        assert_eq!(verified.manifest.payload.version, "0.2.0");
        assert!(manifest_is_newer(&verified, "0.1.0").unwrap());
    }

    #[test]
    fn tampered_manifest_is_rejected() {
        let (mut manifest, public_key) = signed_manifest("0.2.0");
        manifest.payload.version = "9.9.9".to_owned();
        assert!(verify_manifest_with_key(&manifest, "0.1.0", &public_key).is_err());
    }

    #[test]
    fn insecure_installer_url_is_rejected_even_when_signed() {
        let (mut manifest, public_key) = signed_manifest("0.2.0");
        manifest.payload.installer.url = "http://updates.microgifter.com/HomeServer.exe".to_owned();
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        manifest.signature = URL_SAFE_NO_PAD.encode(
            key.sign(&serde_json::to_vec(&manifest.payload).unwrap())
                .to_bytes(),
        );
        assert!(verify_manifest_with_key(&manifest, "0.1.0", &public_key).is_err());
    }
}
