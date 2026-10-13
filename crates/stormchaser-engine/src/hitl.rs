//! Approval links: the "approve" and "reject" links an approval email carries.
//!
//! A link is a capability. Whoever holds it can approve or reject one step without
//! logging in, so it is sealed with a key of its own, names the step and the action,
//! records who it was sent to, and expires.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use stormchaser_model::{RunId, StepInstanceId};
use uuid::Uuid;

/// Environment variable holding the key approval links are sealed with. The engine and
/// the API must be given the same value. There is no default: without it the engine
/// sends no links and the API accepts none.
pub const APPROVAL_LINK_KEY_ENV: &str = "STORMCHASER_APPROVAL_LINK_KEY";

/// Shortest key accepted.
pub const MIN_APPROVAL_LINK_KEY_LEN: usize = 32;

/// Environment variable setting how long a link stays valid, in hours (default 72).
pub const APPROVAL_LINK_TTL_ENV: &str = "STORMCHASER_APPROVAL_LINK_TTL_HOURS";

/// The configured link key, or `None` when it is unset or too short to use.
pub fn approval_link_key() -> Option<String> {
    let key = std::env::var(APPROVAL_LINK_KEY_ENV).ok()?;
    if key.len() < MIN_APPROVAL_LINK_KEY_LEN {
        tracing::error!(
            "{} is shorter than {} characters; approval links are disabled",
            APPROVAL_LINK_KEY_ENV,
            MIN_APPROVAL_LINK_KEY_LEN
        );
        return None;
    }
    Some(key)
}

/// How long a newly issued link stays valid.
pub fn approval_link_ttl() -> chrono::Duration {
    let hours = std::env::var(APPROVAL_LINK_TTL_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|hours| *hours > 0)
        .unwrap_or(72);
    chrono::Duration::hours(hours)
}

/// What an approval link says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalLink {
    /// The run the step belongs to.
    pub run_id: RunId,
    /// The step to approve or reject.
    pub step_id: StepInstanceId,
    /// `approve` or `reject`.
    pub action: String,
    /// Outputs to record for the step when it is approved.
    #[serde(default)]
    pub inputs: Value,
    /// When the link stops working (Unix seconds). Links without it are refused.
    pub exp: i64,
    /// Who the link was sent to. Recorded with the decision, and used as the approver
    /// in policy when there is exactly one.
    #[serde(default)]
    pub approvers: Vec<String>,
}

/// Why a link was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum LinkError {
    /// Not a link this deployment issued: wrong key, tampered with, or not a link.
    Invalid,
    /// A genuine link whose time has passed.
    Expired,
}

fn cipher(key: &str) -> Aes256Gcm {
    let mut hasher = Sha256::new();
    hasher.update(b"stormchaser-approval-link-v1");
    hasher.update(key.as_bytes());
    Aes256Gcm::new(&hasher.finalize())
}

/// Seals a link into the token that goes in its URL.
pub fn seal_approval_link(link: &ApprovalLink, key: &str) -> Result<String> {
    // 96 unpredictable bits from two random UUIDs
    let mut seed = Sha256::new();
    seed.update(Uuid::new_v4().as_bytes());
    seed.update(Uuid::new_v4().as_bytes());
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&seed.finalize()[..12]);
    let nonce = Nonce::from(nonce_bytes);

    let ciphertext = cipher(key)
        .encrypt(&nonce, serde_json::to_vec(link)?.as_ref())
        .map_err(|e| anyhow::anyhow!("Encryption failed: {:?}", e))?;

    let mut combined = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);
    Ok(URL_SAFE_NO_PAD.encode(combined))
}

/// Opens a token, checking that it was sealed with `key` and has not expired at `now`.
pub fn open_approval_link(
    token: &str,
    key: &str,
    now: DateTime<Utc>,
) -> Result<ApprovalLink, LinkError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| LinkError::Invalid)?;
    if decoded.len() <= 12 {
        return Err(LinkError::Invalid);
    }
    let (nonce_bytes, ciphertext) = decoded.split_at(12);
    let nonce = Nonce::try_from(nonce_bytes).map_err(|_| LinkError::Invalid)?;
    let plaintext = cipher(key)
        .decrypt(&nonce, ciphertext)
        .map_err(|_| LinkError::Invalid)?;
    let link: ApprovalLink = serde_json::from_slice(&plaintext).map_err(|_| LinkError::Invalid)?;

    if link.action != "approve" && link.action != "reject" {
        return Err(LinkError::Invalid);
    }
    if now.timestamp() >= link.exp {
        return Err(LinkError::Expired);
    }
    Ok(link)
}

/// Issues the token for a link to approve or reject a step, valid for the configured time.
pub fn generate_approval_token(
    run_id: RunId,
    step_id: StepInstanceId,
    action: &str,
    approvers: &[String],
    key: &str,
) -> Result<String> {
    seal_approval_link(
        &ApprovalLink {
            run_id,
            step_id,
            action: action.to_string(),
            inputs: serde_json::json!({}),
            exp: (Utc::now() + approval_link_ttl()).timestamp(),
            approvers: approvers.to_vec(),
        },
        key,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "test-only-approval-link-key-aaaaaa";

    fn link(exp: i64) -> ApprovalLink {
        ApprovalLink {
            run_id: RunId::new_v4(),
            step_id: StepInstanceId::new_v4(),
            action: "approve".to_string(),
            inputs: serde_json::json!({"decision": "yes"}),
            exp,
            approvers: vec!["lead@example.com".to_string()],
        }
    }

    #[test]
    fn a_sealed_link_opens_with_the_same_key() {
        let now = Utc::now();
        let original = link(now.timestamp() + 60);
        let token = seal_approval_link(&original, KEY).unwrap();
        assert_eq!(open_approval_link(&token, KEY, now), Ok(original));
    }

    #[test]
    fn generated_links_expire() {
        let now = Utc::now();
        let token = generate_approval_token(
            RunId::new_v4(),
            StepInstanceId::new_v4(),
            "reject",
            &[],
            KEY,
        )
        .unwrap();
        assert!(open_approval_link(&token, KEY, now).is_ok());
        let later = now + approval_link_ttl() + chrono::Duration::minutes(1);
        assert_eq!(
            open_approval_link(&token, KEY, later),
            Err(LinkError::Expired)
        );
    }

    #[test]
    fn links_from_another_key_or_tampered_with_are_refused() {
        let now = Utc::now();
        let token = seal_approval_link(&link(now.timestamp() + 60), KEY).unwrap();

        let other_key = "test-only-approval-link-key-bbbbbb";
        assert_eq!(
            open_approval_link(&token, other_key, now),
            Err(LinkError::Invalid)
        );

        let mut bytes = URL_SAFE_NO_PAD.decode(&token).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        let tampered = URL_SAFE_NO_PAD.encode(bytes);
        assert_eq!(
            open_approval_link(&tampered, KEY, now),
            Err(LinkError::Invalid)
        );

        assert_eq!(
            open_approval_link("not-a-token", KEY, now),
            Err(LinkError::Invalid)
        );
        assert_eq!(open_approval_link("", KEY, now), Err(LinkError::Invalid));
    }

    #[test]
    fn a_link_with_no_expiry_or_an_unknown_action_is_refused() {
        let now = Utc::now();
        let seal_raw = |payload: Value| {
            let nonce = Nonce::from([7u8; 12]);
            let ciphertext = cipher(KEY)
                .encrypt(&nonce, serde_json::to_vec(&payload).unwrap().as_ref())
                .unwrap();
            let mut combined = vec![7u8; 12];
            combined.extend_from_slice(&ciphertext);
            URL_SAFE_NO_PAD.encode(combined)
        };

        // The shape issued before links expired
        let old = seal_raw(serde_json::json!({
            "run_id": RunId::new_v4(), "step_id": StepInstanceId::new_v4(),
            "action": "approve", "inputs": {}
        }));
        assert_eq!(open_approval_link(&old, KEY, now), Err(LinkError::Invalid));

        let odd = seal_raw(serde_json::json!({
            "run_id": RunId::new_v4(), "step_id": StepInstanceId::new_v4(),
            "action": "delete", "inputs": {}, "exp": now.timestamp() + 60
        }));
        assert_eq!(open_approval_link(&odd, KEY, now), Err(LinkError::Invalid));
    }

    #[test]
    fn two_links_for_the_same_step_differ() {
        let l = link(Utc::now().timestamp() + 60);
        assert_ne!(
            seal_approval_link(&l, KEY).unwrap(),
            seal_approval_link(&l, KEY).unwrap()
        );
    }
}
