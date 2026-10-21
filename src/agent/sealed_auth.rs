//! Copilot's connection-bound HPKE extension, as exercised by the pinned SDK.
use anyhow::{Context, Result, ensure};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use hpke::{Deserializable, Serializable};
use rand_core::{OsRng, RngCore};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn token(
    initialize: &Value,
    root: &Value,
    resource: &str,
    token: &str,
) -> Result<String> {
    let required = initialize["copilot.encryptionRequired"]
        .as_array()
        .is_some_and(|items| items.iter().any(|v| v == "auth-token"));
    let keys = root["copilot.encryptionKeys"].as_array();
    let key = keys.and_then(|keys| {
        keys.iter().find(|k| {
            k["use"] == "auth-token" && k["algorithm"] == "hpke-x25519-hkdf-sha256-aes256gcm"
        })
    });
    if !required && key.is_none() {
        return Ok(token.to_owned());
    }
    let key = key.context("AHP requires encryption but has no compatible authentication key")?;
    let binding = &initialize["copilot.authChallenge"];
    let challenge = binding["challenge"]
        .as_str()
        .context("AHP authentication challenge is missing")?;
    ensure!(
        binding["required"] == true
            && binding["responseMaxAgeSeconds"]
                .as_u64()
                .is_some_and(|v| v > 0)
            && challenge.len() == 32
            && challenge
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "Invalid AHP authentication challenge"
    );
    let public = STANDARD
        .decode(
            key["publicKey"]
                .as_str()
                .context("Missing AHP public key")?,
        )
        .context("Invalid AHP public key encoding")?;
    ensure!(public.len() == 32, "Invalid AHP public key length");
    let key_id = key["keyId"]
        .as_str()
        .context("Missing AHP key identifier")?;
    ensure!(
        key_id == URL_SAFE_NO_PAD.encode(&Sha256::digest(&public)[..8]),
        "AHP public key identifier mismatch"
    );
    let mut nonce = [0_u8; 16];
    OsRng.fill_bytes(&mut nonce);
    let nonce: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    let plaintext = serde_json::to_vec(
        &json!({"cty":"text","ctx":{"purpose":"auth-token","resource":resource,"connection":{"challenge":challenge,"nonce":nonce,"issuedAt":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()}},"value":token}),
    )?;
    type Kem = hpke::kem::X25519HkdfSha256;
    let recipient = <Kem as hpke::Kem>::PublicKey::from_bytes(&public)
        .map_err(|_| anyhow::anyhow!("Invalid AHP encryption key"))?;
    let (enc, mut sender) = hpke::setup_sender::<
        hpke::aead::AesGcm256,
        hpke::kdf::HkdfSha256,
        Kem,
        _,
    >(&hpke::OpModeS::Base, &recipient, &[], &mut OsRng)
    .map_err(|_| anyhow::anyhow!("AHP encryption setup failed"))?;
    let mut ciphertext = enc.to_bytes().to_vec();
    ciphertext.extend(
        sender
            .seal(&plaintext, &[])
            .map_err(|_| anyhow::anyhow!("AHP authentication encryption failed"))?,
    );
    Ok(format!(
        "copilot-sealed.v1.{key_id}.{}",
        URL_SAFE_NO_PAD.encode(ciphertext)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_token_roundtrip_and_invalid_metadata() {
        type Kem = hpke::kem::X25519HkdfSha256;
        let (secret, public) = <Kem as hpke::Kem>::gen_keypair(&mut OsRng);
        let key_id = URL_SAFE_NO_PAD.encode(&Sha256::digest(public.to_bytes())[..8]);
        let initialize = json!({"copilot.encryptionRequired":["auth-token"],"copilot.authChallenge":{"required":true,"responseMaxAgeSeconds":300,"challenge":"00112233445566778899aabbccddeeff"}});
        let mut root = json!({"copilot.encryptionKeys":[{"use":"auth-token","algorithm":"hpke-x25519-hkdf-sha256-aes256gcm","publicKey":STANDARD.encode(public.to_bytes()),"keyId":key_id}]});
        let sealed = token(&initialize, &root, "https://api.github.com", "test-secret").unwrap();
        assert!(!sealed.contains("test-secret"));
        let bytes = URL_SAFE_NO_PAD
            .decode(sealed.rsplit('.').next().unwrap())
            .unwrap();
        let enc = <Kem as hpke::Kem>::EncappedKey::from_bytes(&bytes[..32]).unwrap();
        let mut receiver =
            hpke::setup_receiver::<hpke::aead::AesGcm256, hpke::kdf::HkdfSha256, Kem>(
                &hpke::OpModeR::Base,
                &secret,
                &enc,
                &[],
            )
            .unwrap();
        let clear: Value =
            serde_json::from_slice(&receiver.open(&bytes[32..], &[]).unwrap()).unwrap();
        assert_eq!(clear["value"], "test-secret");
        assert_eq!(
            clear["ctx"]["connection"]["challenge"],
            initialize["copilot.authChallenge"]["challenge"]
        );
        assert_eq!(clear["ctx"]["resource"], "https://api.github.com");
        assert_eq!(
            clear["ctx"]["connection"]["nonce"].as_str().unwrap().len(),
            32
        );
        assert!(
            token(
                &json!({"copilot.encryptionRequired":["auth-token"]}),
                &root,
                "resource",
                "secret"
            )
            .is_err()
        );
        root["copilot.encryptionKeys"][0]["keyId"] = json!("wrong");
        assert!(token(&initialize, &root, "resource", "secret").is_err());
        assert!(token(&initialize, &json!({}), "resource", "secret").is_err());
        assert_eq!(
            token(&json!({}), &json!({}), "resource", "secret").unwrap(),
            "secret"
        );
    }
}
