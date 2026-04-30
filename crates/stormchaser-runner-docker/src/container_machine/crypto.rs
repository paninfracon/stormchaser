use aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::Aes256Gcm;
use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};

/// Encrypts the provided string data using the provided key string with AES-256-GCM.
pub fn encrypt_state(data: &str, key_str: &str) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(key_str.as_bytes());
    let key_bytes = hasher.finalize();

    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid key length: {}", e))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, data.as_bytes())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);

    Ok(general_purpose::STANDARD.encode(combined))
}

/// Decrypts the provided base64 encoded string data using the provided key string with AES-256-GCM.
pub fn decrypt_state(encoded: &str, key_str: &str) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(key_str.as_bytes());
    let key_bytes = hasher.finalize();

    let combined = general_purpose::STANDARD.decode(encoded)?;
    if combined.len() < 12 {
        return Err(anyhow::anyhow!("Invalid encrypted data"));
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = aead::Nonce::<Aes256Gcm>::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid key length: {}", e))?;

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("Decryption failed: {}", e))?;

    Ok(String::from_utf8(plaintext)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encryption_decryption() {
        let secret = "my_secret_data";
        let key = "12345678901234567890123456789012"; // 32 bytes
        let encrypted = encrypt_state(secret, key).unwrap();
        assert_ne!(encrypted, secret);
        let decrypted = decrypt_state(&encrypted, key).unwrap();
        assert_eq!(decrypted, secret);
    }
}
