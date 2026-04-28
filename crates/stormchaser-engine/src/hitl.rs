use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub fn generate_approval_token(
    run_id: Uuid,
    step_id: Uuid,
    action: &str,
    secret: &str,
) -> Result<String> {
    // 1. Derive key from secret
    let mut hasher = Sha256::new();
    hasher.update(secret);
    let key_bytes = hasher.finalize();
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    // 2. Prepare payload
    let payload = json!({
        "run_id": run_id,
        "step_id": step_id,
        "action": action,
        "inputs": {},
    });
    let plaintext = serde_json::to_vec(&payload)?;

    // 3. Encrypt
    let mut nonce_bytes = [0u8; 12];
    // In a real app we'd use a CSPRNG, but for this utility we can use Uuid bytes or similar if needed.
    // Actually, let's use a simple deterministic nonce for now if we don't want to add rand,
    // but better to add rand for security.
    // Given the constraints, let's just use the first 12 bytes of a new Uuid.
    let nonce_uuid = Uuid::new_v4();
    nonce_bytes.copy_from_slice(&nonce_uuid.as_bytes()[..12]);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {:?}", e))?;

    // 4. Combine nonce + ciphertext
    let mut combined = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    // 5. Encode base64
    Ok(URL_SAFE_NO_PAD.encode(combined))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn test_generate_approval_token() {
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        let secret = "test-secret";

        let token = generate_approval_token(run_id, step_id, "approve", secret).unwrap();
        assert!(!token.is_empty());

        // Verify we can decrypt it back (simulating API logic)
        let mut hasher = Sha256::new();
        hasher.update(secret);
        let key_bytes = hasher.finalize();
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let decoded = URL_SAFE_NO_PAD.decode(token).unwrap();
        let (nonce_bytes, ciphertext) = decoded.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher.decrypt(nonce, ciphertext).unwrap();

        let payload: Value = serde_json::from_slice(&plaintext).unwrap();
        assert_eq!(payload["run_id"], run_id.to_string());
        assert_eq!(payload["step_id"], step_id.to_string());
        assert_eq!(payload["action"], "approve");
    }
}
