use anyhow::{bail, ensure, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey, Verifier};
use microgifter_homeserver_core::{
    SignedUpdateManifest, UpdateChannel, UpdateInstallerContract, UpdateManifestPayload,
    PRODUCT_NAME, UPDATE_MANIFEST_SCHEMA_VERSION,
};
use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

const PRIVATE_KEY_ENV: &str = "MG_HOMESERVER_RELEASE_PRIVATE_KEY_BASE64";
const PUBLIC_KEY_ENV: &str = "MG_HOMESERVER_RELEASE_PUBLIC_KEY_BASE64";
const KEY_ID_ENV: &str = "MG_HOMESERVER_RELEASE_KEY_ID";

#[derive(Debug)]
struct Arguments {
    version: String,
    minimum_version: String,
    installer: PathBuf,
    installer_url: String,
    authenticode_thumbprint: String,
    release_notes: PathBuf,
    output: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("HomeServer release manifest failure: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let arguments = parse_arguments()?;
    let private_key = required_env(PRIVATE_KEY_ENV)?;
    let public_key = required_env(PUBLIC_KEY_ENV)?;
    let key_id = required_env(KEY_ID_ENV)?;

    ensure!(
        !key_id.trim().is_empty(),
        "release manifest key ID cannot be empty"
    );
    ensure!(
        arguments.version.parse::<semver::Version>().is_ok(),
        "release version is invalid"
    );
    ensure!(
        arguments.minimum_version.parse::<semver::Version>().is_ok(),
        "minimum release version is invalid"
    );
    ensure!(
        arguments.installer_url.starts_with("https://"),
        "release installer URL must use HTTPS"
    );

    let signing_key = decode_signing_key(&private_key)?;
    let expected_public_key = STANDARD
        .decode(public_key.trim())
        .context("release public key is not valid base64")?;
    ensure!(
        expected_public_key.as_slice() == signing_key.verifying_key().as_bytes(),
        "release private key does not match the configured public key"
    );

    let installer_bytes = fs::read(&arguments.installer).with_context(|| {
        format!(
            "unable to read release installer {}",
            arguments.installer.display()
        )
    })?;
    ensure!(
        installer_bytes.len() >= 1_000_000,
        "release installer is unexpectedly small"
    );
    let installer_sha256 = hex::encode(Sha256::digest(&installer_bytes));
    let release_notes = fs::read_to_string(&arguments.release_notes).with_context(|| {
        format!(
            "unable to read release notes {}",
            arguments.release_notes.display()
        )
    })?;
    ensure!(
        !release_notes.trim().is_empty(),
        "release notes cannot be empty"
    );
    ensure!(
        release_notes.chars().count() <= 20_000,
        "release notes exceed the HomeServer manifest limit"
    );

    let thumbprint = arguments
        .authenticode_thumbprint
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    ensure!(
        matches!(thumbprint.len(), 40 | 64)
            && thumbprint
                .chars()
                .all(|character| character.is_ascii_hexdigit()),
        "release Authenticode thumbprint is invalid"
    );

    let payload = UpdateManifestPayload {
        schema_version: UPDATE_MANIFEST_SCHEMA_VERSION,
        product: PRODUCT_NAME.to_owned(),
        channel: UpdateChannel::Stable,
        version: arguments.version,
        minimum_version: Some(arguments.minimum_version),
        published_at_utc: Utc::now(),
        release_notes,
        installer: UpdateInstallerContract {
            url: arguments.installer_url,
            file_name: "Microgifter-HomeServer-Setup.exe".to_owned(),
            size_bytes: installer_bytes.len() as u64,
            sha256: installer_sha256,
            authenticode_thumbprint: thumbprint,
        },
    };

    let canonical_payload = serde_json::to_vec(&payload)?;
    let signature = signing_key.sign(&canonical_payload);
    signing_key
        .verifying_key()
        .verify(&canonical_payload, &signature)
        .context("generated release manifest signature did not verify")?;

    let manifest = SignedUpdateManifest {
        key_id,
        payload,
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    };
    let output = serde_json::to_vec_pretty(&manifest)?;
    if let Some(parent) = arguments.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments.output, output).with_context(|| {
        format!(
            "unable to write release manifest {}",
            arguments.output.display()
        )
    })?;
    println!(
        "Created signed HomeServer update manifest at {}",
        arguments.output.display()
    );
    Ok(())
}

fn decode_signing_key(value: &str) -> Result<SigningKey> {
    let bytes = STANDARD
        .decode(value.trim())
        .context("release private key is not valid base64")?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("release private key must contain exactly 32 bytes"))?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn required_env(name: &str) -> Result<String> {
    let value = env::var(name).with_context(|| format!("required environment variable {name} is missing"))?;
    ensure!(!value.trim().is_empty(), "required environment variable {name} is empty");
    Ok(value)
}

fn parse_arguments() -> Result<Arguments> {
    let mut arguments = env::args().skip(1);
    let mut version = None;
    let mut minimum_version = None;
    let mut installer = None;
    let mut installer_url = None;
    let mut authenticode_thumbprint = None;
    let mut release_notes = None;
    let mut output = None;

    while let Some(argument) = arguments.next() {
        let value = arguments
            .next()
            .with_context(|| format!("missing value for {argument}"))?;
        match argument.as_str() {
            "--version" => version = Some(value),
            "--minimum-version" => minimum_version = Some(value),
            "--installer" => installer = Some(PathBuf::from(value)),
            "--installer-url" => installer_url = Some(value),
            "--authenticode-thumbprint" => authenticode_thumbprint = Some(value),
            "--release-notes" => release_notes = Some(PathBuf::from(value)),
            "--output" => output = Some(PathBuf::from(value)),
            _ => bail!("unsupported release manifest argument: {argument}"),
        }
    }

    Ok(Arguments {
        version: version.context("--version is required")?,
        minimum_version: minimum_version.context("--minimum-version is required")?,
        installer: installer.context("--installer is required")?,
        installer_url: installer_url.context("--installer-url is required")?,
        authenticode_thumbprint: authenticode_thumbprint
            .context("--authenticode-thumbprint is required")?,
        release_notes: release_notes.context("--release-notes is required")?,
        output: output.context("--output is required")?,
    })
}
