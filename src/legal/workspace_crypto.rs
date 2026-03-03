use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MATTER_ENVELOPE_FORMAT: &str = "clawyer-matter-enc-v1";
const MATTER_ENVELOPE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MatterEncryptedEnvelope {
    format: String,
    version: u32,
    algorithm: String,
    salt_b64: String,
    ciphertext_b64: String,
    ciphertext_sha256: String,
    plaintext_sha256: String,
    matter_id: String,
    encrypted_at: String,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn matter_id_for_path(path: &str, matter_root: &str) -> Option<String> {
    let path_parts = path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let root_parts = matter_root
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if root_parts.is_empty() || path_parts.len() <= root_parts.len() {
        return None;
    }
    if !path_parts[..root_parts.len()]
        .iter()
        .zip(root_parts.iter())
        .all(|(a, b)| a == b)
    {
        return None;
    }
    let candidate = crate::legal::policy::sanitize_matter_id(path_parts[root_parts.len()]);
    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

pub fn is_encrypted_payload(content: &str) -> bool {
    let Ok(envelope) = serde_json::from_str::<MatterEncryptedEnvelope>(content) else {
        return false;
    };
    envelope.format == MATTER_ENVELOPE_FORMAT && envelope.version == MATTER_ENVELOPE_VERSION
}

pub fn encrypt_matter_content(
    crypto: &crate::secrets::SecretsCrypto,
    matter_id: &str,
    plaintext: &str,
) -> Result<String, String> {
    let (ciphertext, salt) = crypto
        .encrypt(plaintext.as_bytes())
        .map_err(|e| format!("matter encrypt failed: {e}"))?;
    let envelope = MatterEncryptedEnvelope {
        format: MATTER_ENVELOPE_FORMAT.to_string(),
        version: MATTER_ENVELOPE_VERSION,
        algorithm: "aes-256-gcm-hkdf-sha256".to_string(),
        salt_b64: BASE64.encode(salt),
        ciphertext_b64: BASE64.encode(&ciphertext),
        ciphertext_sha256: sha256_hex(&ciphertext),
        plaintext_sha256: sha256_hex(plaintext.as_bytes()),
        matter_id: matter_id.to_string(),
        encrypted_at: Utc::now().to_rfc3339(),
    };

    serde_json::to_string(&envelope).map_err(|e| format!("matter envelope encode failed: {e}"))
}

pub fn decrypt_matter_content(
    crypto: &crate::secrets::SecretsCrypto,
    expected_matter_id: &str,
    content: &str,
) -> Result<Option<String>, String> {
    let Ok(envelope) = serde_json::from_str::<MatterEncryptedEnvelope>(content) else {
        return Ok(None);
    };
    if envelope.format != MATTER_ENVELOPE_FORMAT {
        return Ok(None);
    }
    if envelope.version != MATTER_ENVELOPE_VERSION {
        return Err(format!(
            "unsupported matter envelope version {}",
            envelope.version
        ));
    }

    let normalized_expected = crate::legal::policy::sanitize_matter_id(expected_matter_id);
    let normalized_embedded = crate::legal::policy::sanitize_matter_id(&envelope.matter_id);
    if normalized_expected.is_empty() || normalized_embedded != normalized_expected {
        return Err("matter envelope scope mismatch".to_string());
    }

    let salt = BASE64
        .decode(&envelope.salt_b64)
        .map_err(|e| format!("invalid matter envelope salt: {e}"))?;
    let ciphertext = BASE64
        .decode(&envelope.ciphertext_b64)
        .map_err(|e| format!("invalid matter envelope ciphertext: {e}"))?;

    if sha256_hex(&ciphertext) != envelope.ciphertext_sha256 {
        return Err("matter envelope ciphertext checksum mismatch".to_string());
    }

    let plaintext = crypto
        .decrypt(&ciphertext, &salt)
        .map_err(|e| format!("matter decrypt failed: {e}"))?;

    let text = plaintext.expose().to_string();
    if sha256_hex(text.as_bytes()) != envelope.plaintext_sha256 {
        return Err("matter envelope plaintext checksum mismatch".to_string());
    }

    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::{
        decrypt_matter_content, encrypt_matter_content, is_encrypted_payload, matter_id_for_path,
    };

    fn test_crypto() -> crate::secrets::SecretsCrypto {
        crate::secrets::SecretsCrypto::new(SecretString::from(
            "0123456789abcdef0123456789abcdef".to_string(),
        ))
        .expect("crypto")
    }

    #[test]
    fn encrypt_roundtrip() {
        let crypto = test_crypto();
        let payload = encrypt_matter_content(&crypto, "demo-matter", "facts")
            .expect("encrypt should succeed");
        assert!(is_encrypted_payload(&payload));
        let decrypted = decrypt_matter_content(&crypto, "demo-matter", &payload)
            .expect("decrypt should succeed");
        assert_eq!(decrypted.as_deref(), Some("facts"));
    }

    #[test]
    fn decrypt_rejects_scope_mismatch() {
        let crypto = test_crypto();
        let payload = encrypt_matter_content(&crypto, "demo-matter", "facts")
            .expect("encrypt should succeed");
        let err = decrypt_matter_content(&crypto, "other-matter", &payload)
            .expect_err("mismatch should fail");
        assert!(err.contains("scope mismatch"));
    }

    #[test]
    fn matter_path_scope_extracts_matter_id() {
        assert_eq!(
            matter_id_for_path("matters/demo/facts.md", "matters"),
            Some("demo".to_string())
        );
        assert_eq!(
            matter_id_for_path("casefiles/demo/facts.md", "matters"),
            None
        );
    }
}
