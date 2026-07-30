use anyhow::{ensure, Context, Result};
use base64::{
    engine::general_purpose::STANDARD, engine::general_purpose::URL_SAFE_NO_PAD, Engine as _,
};
use chrono::Utc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) struct SignedSnapshotEvidence<'a> {
    pub public_key_base64: &'a str,
    pub expected_key_id: &'a str,
    pub algorithm: &'a str,
    pub key_id: &'a str,
    pub signed_document: &'a str,
    pub signature: &'a str,
    pub signed_document_hash: &'a str,
    pub schema: &'a str,
    pub account_id: i64,
    pub device_public_id: &'a str,
    pub max_revision: u64,
    pub snapshot_hash: &'a str,
    pub settings: Value,
}

pub(super) fn verify(evidence: SignedSnapshotEvidence<'_>) -> Result<()> {
    ensure!(
        evidence.algorithm.eq_ignore_ascii_case("Ed25519"),
        "VP3 federated settings signature algorithm is invalid"
    );
    ensure!(
        evidence.key_id == evidence.expected_key_id,
        "VP3 federated settings signing key is not trusted"
    );
    let public_key = STANDARD
        .decode(evidence.public_key_base64)
        .context("VP3 federated settings public key encoding is invalid")?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("VP3 federated settings public key length is invalid"))?;
    let verifier = VerifyingKey::from_bytes(&public_key)
        .context("VP3 federated settings public key is invalid")?;
    let document = URL_SAFE_NO_PAD
        .decode(evidence.signed_document)
        .context("VP3 federated settings document encoding is invalid")?;
    let signature = URL_SAFE_NO_PAD
        .decode(evidence.signature)
        .context("VP3 federated settings signature encoding is invalid")?;
    let signature = Signature::from_slice(&signature)
        .context("VP3 federated settings signature length is invalid")?;
    verifier
        .verify(&document, &signature)
        .context("VP3 federated settings signature verification failed")?;
    ensure!(
        hex::encode(Sha256::digest(&document)).eq_ignore_ascii_case(evidence.signed_document_hash),
        "VP3 federated settings signed document hash is invalid"
    );

    let claims: Value = serde_json::from_slice(&document)
        .context("VP3 federated settings signed document is invalid")?;
    let now = Utc::now().timestamp();
    let issued_at = claims
        .get("iat")
        .and_then(Value::as_i64)
        .context("VP3 federated settings issued-at claim is missing")?;
    let expires_at = claims
        .get("exp")
        .and_then(Value::as_i64)
        .context("VP3 federated settings expiration claim is missing")?;
    ensure!(
        issued_at <= now + 600,
        "VP3 federated settings snapshot was issued in the future"
    );
    ensure!(
        expires_at > now,
        "VP3 federated settings snapshot has expired"
    );
    ensure!(
        (60..=900).contains(&(expires_at - issued_at)),
        "VP3 federated settings snapshot lifetime is invalid"
    );

    let expected = json!({
        "schema": evidence.schema,
        "account_id": evidence.account_id,
        "device_public_id": evidence.device_public_id,
        "max_revision": evidence.max_revision,
        "snapshot_hash": evidence.snapshot_hash,
        "settings": evidence.settings,
        "iat": issued_at,
        "exp": expires_at,
    });
    ensure!(
        claims == expected,
        "VP3 federated settings wrapper does not match its signed document"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn valid_snapshot_signature_is_accepted_and_tampering_is_rejected() {
        let secret = [7_u8; 32];
        let signing = SigningKey::from_bytes(&secret);
        let now = Utc::now().timestamp();
        let claims = json!({
            "schema": "vp3.federated-settings.v1",
            "account_id": 7,
            "device_public_id": "HS-TEST",
            "max_revision": 3,
            "snapshot_hash": "a".repeat(64),
            "settings": [],
            "iat": now,
            "exp": now + 600,
        });
        let document = serde_json::to_vec(&claims).unwrap();
        let signature = signing.sign(&document);
        let encoded_document = URL_SAFE_NO_PAD.encode(&document);
        let encoded_signature = URL_SAFE_NO_PAD.encode(signature.to_bytes());
        let public_key = STANDARD.encode(signing.verifying_key().to_bytes());
        let document_hash = hex::encode(Sha256::digest(&document));
        let evidence = SignedSnapshotEvidence {
            public_key_base64: &public_key,
            expected_key_id: "settings-key-v1",
            algorithm: "Ed25519",
            key_id: "settings-key-v1",
            signed_document: &encoded_document,
            signature: &encoded_signature,
            signed_document_hash: &document_hash,
            schema: "vp3.federated-settings.v1",
            account_id: 7,
            device_public_id: "HS-TEST",
            max_revision: 3,
            snapshot_hash: &"a".repeat(64),
            settings: json!([]),
        };
        assert!(verify(evidence).is_ok());

        let tampered = SignedSnapshotEvidence {
            public_key_base64: &public_key,
            expected_key_id: "settings-key-v1",
            algorithm: "Ed25519",
            key_id: "settings-key-v1",
            signed_document: &encoded_document,
            signature: &encoded_signature,
            signed_document_hash: &document_hash,
            schema: "vp3.federated-settings.v1",
            account_id: 8,
            device_public_id: "HS-TEST",
            max_revision: 3,
            snapshot_hash: &"a".repeat(64),
            settings: json!([]),
        };
        assert!(verify(tampered).is_err());
    }
}
