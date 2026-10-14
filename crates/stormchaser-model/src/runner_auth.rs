//! Runner identity.
//!
//! Every runner connects to NATS with the same credential, and a message names its
//! runner only in its `runner_id` field, so the engine could not tell one runner from
//! another. Now each runner asks the engine for a key of its own when it starts
//! ([`SESSION_SUBJECT`]). The engine sends the key sealed to a public key the runner
//! made for the purpose, so another holder of the runner credential who sees the reply
//! cannot read it, and it refuses to issue a key for an id whose runner is still
//! sending heartbeats. The runner then signs everything it reports with the key
//! ([`sign_payload`], applied by [`crate::nats`] once [`set_session`] has been called),
//! and the engine checks the signature against the key it issued for that id
//! ([`verify_payload`]).
//!
//! What this does not stop: a holder of the runner credential can still pull tasks, and
//! can take over the id of a runner that has stopped sending heartbeats.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use ring::{aead, agreement, hkdf, hmac, rand};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a runner asks the engine for its key. It is a request-reply subject, outside
/// the JetStream stream: a request on a subject the stream captures is answered by the
/// stream's own acknowledgement instead of by the engine.
pub const SESSION_SUBJECT: &str = "stormchaser.rpc.runner.session";

/// The field of a runner's message that carries its signature.
pub const SIGNATURE_FIELD: &str = "runner_signature";

/// How far a signature's timestamp may be from the engine's clock.
pub const MAX_SIGNATURE_AGE_SECS: i64 = 300;

/// `STORMCHASER_RUNNER_SIGNATURES` on the engine: `required` (the default) or
/// `optional`, which lets runners that have no key yet, older ones during an upgrade,
/// report unsigned.
pub const SIGNATURES_ENV: &str = "STORMCHASER_RUNNER_SIGNATURES";

const KEY_INFO: &[u8] = b"stormchaser runner session v1";
const MAC_DOMAIN: &str = "stormchaser runner message v1";

/// A runner's request for a key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRequest {
    pub runner_id: String,
    /// The runner's one-off X25519 public key, base64.
    pub public_key: String,
}

/// The engine's answer: a key sealed to the runner's public key, or why there is none.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The engine's one-off X25519 public key, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_public_key: Option<String>,
    /// Nonce and ciphertext of the key, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_key: Option<String>,
}

/// A runner's half of the exchange: its one-off key pair, used for one request.
pub struct PendingSession {
    runner_id: String,
    private_key: agreement::EphemeralPrivateKey,
    public_key: Vec<u8>,
}

impl PendingSession {
    pub fn new(runner_id: &str) -> Result<Self> {
        let rng = rand::SystemRandom::new();
        let private_key = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
            .map_err(|_| anyhow!("could not generate a key pair"))?;
        let public_key = private_key
            .compute_public_key()
            .map_err(|_| anyhow!("could not compute the public key"))?
            .as_ref()
            .to_vec();
        Ok(Self {
            runner_id: runner_id.to_string(),
            private_key,
            public_key,
        })
    }

    pub fn request(&self) -> SessionRequest {
        SessionRequest {
            runner_id: self.runner_id.clone(),
            public_key: b64().encode(&self.public_key),
        }
    }

    /// Opens the engine's answer, giving the key.
    pub fn open(self, response: &SessionResponse) -> Result<[u8; 32]> {
        if !response.ok {
            bail!(
                "the engine refused: {}",
                response.error.as_deref().unwrap_or("no reason given")
            );
        }
        let engine_public = b64()
            .decode(
                response
                    .engine_public_key
                    .as_deref()
                    .context("no engine key")?,
            )
            .context("engine key is not base64")?;
        let sealed = b64()
            .decode(response.sealed_key.as_deref().context("no sealed key")?)
            .context("sealed key is not base64")?;
        let wrapping = agreement::agree_ephemeral(
            self.private_key,
            &agreement::UnparsedPublicKey::new(&agreement::X25519, &engine_public),
            |shared| wrapping_key(shared, &self.runner_id, &self.public_key, &engine_public),
        )
        .map_err(|_| anyhow!("key agreement failed"))??;
        open_key(&wrapping, &self.runner_id, &sealed)
    }
}

/// The engine's half: makes a key for `request` and seals it to the runner.
pub fn issue_session(request: &SessionRequest) -> Result<([u8; 32], SessionResponse)> {
    let rng = rand::SystemRandom::new();
    let runner_public = b64()
        .decode(&request.public_key)
        .context("runner key is not base64")?;

    let mut key = [0u8; 32];
    rand::SecureRandom::fill(&rng, &mut key).map_err(|_| anyhow!("no randomness"))?;

    let private_key = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
        .map_err(|_| anyhow!("could not generate a key pair"))?;
    let engine_public = private_key
        .compute_public_key()
        .map_err(|_| anyhow!("could not compute the public key"))?
        .as_ref()
        .to_vec();
    let wrapping = agreement::agree_ephemeral(
        private_key,
        &agreement::UnparsedPublicKey::new(&agreement::X25519, &runner_public),
        |shared| wrapping_key(shared, &request.runner_id, &runner_public, &engine_public),
    )
    .map_err(|_| anyhow!("key agreement failed: the runner's public key is not usable"))??;

    let mut nonce = [0u8; aead::NONCE_LEN];
    rand::SecureRandom::fill(&rng, &mut nonce).map_err(|_| anyhow!("no randomness"))?;
    let mut sealed = key.to_vec();
    wrapping
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(request.runner_id.as_bytes()),
            &mut sealed,
        )
        .map_err(|_| anyhow!("could not seal the key"))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);

    Ok((
        key,
        SessionResponse {
            ok: true,
            error: None,
            engine_public_key: Some(b64().encode(&engine_public)),
            sealed_key: Some(b64().encode(&out)),
        },
    ))
}

impl SessionResponse {
    pub fn refused(reason: &str) -> Self {
        Self {
            ok: false,
            error: Some(reason.to_string()),
            engine_public_key: None,
            sealed_key: None,
        }
    }
}

fn wrapping_key(
    shared: &[u8],
    runner_id: &str,
    runner_public: &[u8],
    engine_public: &[u8],
) -> Result<aead::LessSafeKey> {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[runner_public, engine_public].concat());
    let info = [KEY_INFO, runner_id.as_bytes()];
    let prk = salt.extract(shared);
    let okm = prk
        .expand(&info, &aead::CHACHA20_POLY1305)
        .map_err(|_| anyhow!("key derivation failed"))?;
    Ok(aead::LessSafeKey::new(aead::UnboundKey::from(okm)))
}

fn open_key(wrapping: &aead::LessSafeKey, runner_id: &str, sealed: &[u8]) -> Result<[u8; 32]> {
    if sealed.len() < aead::NONCE_LEN {
        bail!("sealed key is too short");
    }
    let (nonce, ciphertext) = sealed.split_at(aead::NONCE_LEN);
    let mut buffer = ciphertext.to_vec();
    let plain = wrapping
        .open_in_place(
            aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| anyhow!("bad nonce"))?,
            aead::Aad::from(runner_id.as_bytes()),
            &mut buffer,
        )
        .map_err(|_| anyhow!("the key was not sealed for this runner"))?;
    plain
        .try_into()
        .map_err(|_| anyhow!("the key has the wrong length"))
}

fn b64() -> &'static base64::engine::GeneralPurpose {
    &base64::engine::general_purpose::STANDARD
}

/// This process's key, once the engine has issued one. Only runners have one.
static SESSION: parking_lot::RwLock<Option<[u8; 32]>> = parking_lot::RwLock::new(None);

/// Makes this process sign what it publishes with `key` from now on.
pub fn set_session(key: [u8; 32]) {
    *SESSION.write() = Some(key);
}

/// Signs `payload` with this process's key, if it has one; otherwise leaves it alone.
pub fn sign_if_runner(payload: Value) -> Value {
    match *SESSION.read() {
        Some(key) => sign_payload(payload, &key),
        None => payload,
    }
}

/// Adds a signature over `payload` (an object) and the time.
pub fn sign_payload(mut payload: Value, key: &[u8; 32]) -> Value {
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };
    object.remove(SIGNATURE_FIELD);
    let signed_at = chrono::Utc::now().timestamp();
    let mac = mac(key, signed_at, &Value::Object(object.clone()));
    object.insert(
        SIGNATURE_FIELD.to_string(),
        serde_json::json!({ "signed_at": signed_at, "mac": b64().encode(mac) }),
    );
    payload
}

/// Whether `payload` carries a signature at all.
pub fn is_signed(payload: &Value) -> bool {
    payload.get(SIGNATURE_FIELD).is_some()
}

/// Checks the signature on `payload` against `key`.
pub fn verify_payload(payload: &Value, key: &[u8]) -> Result<()> {
    let mut object = payload
        .as_object()
        .context("the message is not an object")?
        .clone();
    let signature = object
        .remove(SIGNATURE_FIELD)
        .context("the message is not signed")?;
    let signed_at = signature["signed_at"]
        .as_i64()
        .context("the signature has no time")?;
    let given = b64()
        .decode(
            signature["mac"]
                .as_str()
                .context("the signature has no MAC")?,
        )
        .context("the signature is not base64")?;
    if (chrono::Utc::now().timestamp() - signed_at).abs() > MAX_SIGNATURE_AGE_SECS {
        bail!("the signature is too old, or from too far in the future");
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::verify(
        &key,
        &signed_bytes(signed_at, &Value::Object(object)),
        &given,
    )
    .map_err(|_| anyhow!("the signature does not match"))
}

fn mac(key: &[u8; 32], signed_at: i64, payload: &Value) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&key, &signed_bytes(signed_at, payload))
        .as_ref()
        .to_vec()
}

/// What is signed: a domain label, the time, and the payload as serde_json writes it.
/// The engine parses the text the runner wrote and writes it again, which gives the
/// same bytes: object keys come out in the same order either way.
fn signed_bytes(signed_at: i64, payload: &Value) -> Vec<u8> {
    let mut bytes = format!("{MAC_DOMAIN}\n{signed_at}\n").into_bytes();
    bytes.extend(serde_json::to_vec(payload).unwrap_or_default());
    bytes
}

/// Asks the engine for this runner's key, and signs from then on with it.
///
/// Repeats until the engine answers with a key: it refuses while another process still
/// holds `runner_id` (until that one stops, or misses its heartbeats for 30 seconds),
/// and does not answer while it is down.
pub async fn obtain_session(client: &async_nats::Client, runner_id: &str) -> Result<()> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match try_obtain_session(client, runner_id).await {
            Ok(key) => {
                set_session(key);
                tracing::info!("Received this runner's key from the engine");
                return Ok(());
            }
            Err(e) => {
                tracing::warn!(
                    "No key from the engine yet for runner {} (attempt {}): {:#}",
                    runner_id,
                    attempt,
                    e
                );
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

async fn try_obtain_session(client: &async_nats::Client, runner_id: &str) -> Result<[u8; 32]> {
    let pending = PendingSession::new(runner_id)?;
    let request = serde_json::to_vec(&pending.request())?;
    let reply = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.request(SESSION_SUBJECT, request.into()),
    )
    .await
    .map_err(|_| anyhow!("the engine did not answer"))?
    .map_err(|e| anyhow!("the request failed: {e}"))?;
    let response: SessionResponse =
        serde_json::from_slice(&reply.payload).context("the answer is not a session response")?;
    pending.open(&response)
}

/// Whether the engine accepts unsigned messages from runners that have no key.
pub fn signatures_required() -> bool {
    !matches!(
        std::env::var(SIGNATURES_ENV).as_deref(),
        Ok("optional") | Ok("OPTIONAL")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_runner_gets_the_key_the_engine_issued_and_no_one_else_can_open_it() {
        let pending = PendingSession::new("k8s-runner-1").unwrap();
        let (issued, response) = issue_session(&pending.request()).unwrap();
        // Over the wire and back
        let response: SessionResponse =
            serde_json::from_str(&serde_json::to_string(&response).unwrap()).unwrap();

        // Another runner that saw the reply, with a key pair of its own
        let eavesdropper = PendingSession::new("k8s-runner-1").unwrap();
        assert!(eavesdropper.open(&response).is_err());

        assert_eq!(pending.open(&response).unwrap(), issued);
    }

    #[test]
    fn a_key_sealed_for_one_runner_id_does_not_open_as_another() {
        let pending = PendingSession::new("runner-a").unwrap();
        let mut request = pending.request();
        request.runner_id = "runner-b".into();
        let (_, response) = issue_session(&request).unwrap();
        assert!(pending.open(&response).is_err());
    }

    #[test]
    fn signed_messages_verify_only_with_their_key_and_unchanged() {
        let key = [7u8; 32];
        let payload =
            json!({ "runner_id": "r1", "step_id": "s", "outputs": {"b": 2, "a": [1, 2.5]} });
        let signed = sign_payload(payload.clone(), &key);

        // Through text and back, as the engine receives it
        let received: Value = serde_json::from_str(&signed.to_string()).unwrap();
        verify_payload(&received, &key).unwrap();

        assert!(verify_payload(&received, &[8u8; 32]).is_err());
        let mut tampered = received.clone();
        tampered["runner_id"] = json!("r2");
        assert!(verify_payload(&tampered, &key).is_err());
        assert!(verify_payload(&payload, &key).is_err());

        let mut stale = received.clone();
        stale[SIGNATURE_FIELD]["signed_at"] =
            json!(chrono::Utc::now().timestamp() - MAX_SIGNATURE_AGE_SECS - 10);
        assert!(verify_payload(&stale, &key).is_err());
    }
}
