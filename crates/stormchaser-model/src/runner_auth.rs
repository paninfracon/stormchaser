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

/// How far a signature's timestamp may be from when the message was published.
pub const MAX_SIGNATURE_AGE_SECS: i64 = 300;

/// `STORMCHASER_RUNNER_SIGNATURES` on the engine: `required` (the default) or
/// `optional`, which lets runners that have no key yet, older ones during an upgrade,
/// report unsigned.
pub const SIGNATURES_ENV: &str = "STORMCHASER_RUNNER_SIGNATURES";

const KEY_INFO: &[u8] = b"stormchaser runner session v1";
const MAC_DOMAIN: &str = "stormchaser runner message v2";

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

/// Signs `payload`, to be published on `subject`, with this process's key, if it has
/// one; otherwise leaves it alone.
pub fn sign_if_runner(payload: Value, subject: &str) -> Value {
    match *SESSION.read() {
        Some(key) => sign_payload(payload, &key, subject),
        None => payload,
    }
}

/// `subject` without its shard (`stormchaser.v1.<shard>.step.completed` becomes
/// `stormchaser.v1.step.completed`), which is the form a signature covers and the form
/// the engine routes on.
pub fn normalize_subject(subject: &str) -> String {
    let parts: Vec<&str> = subject.split('.').collect();
    if parts.len() > 3 && (parts[2] == "global" || parts[2].parse::<u32>().is_ok()) {
        format!("{}.{}.{}", parts[0], parts[1], parts[3..].join("."))
    } else {
        subject.to_string()
    }
}

/// Adds a signature over `payload` (an object), the subject it is published on, and the
/// time. The subject is covered so that a signed report cannot be published again on
/// another subject, where it would mean something else (a step's report replayed as
/// the runner going offline, say).
pub fn sign_payload(mut payload: Value, key: &[u8; 32], subject: &str) -> Value {
    let Some(object) = payload.as_object_mut() else {
        return payload;
    };
    object.remove(SIGNATURE_FIELD);
    let signed_at = chrono::Utc::now().timestamp();
    let mac = mac(key, subject, signed_at, &Value::Object(object.clone()));
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

/// Checks the signature on `payload`, received on `subject`, against `key`.
pub fn verify_payload(payload: &Value, key: &[u8], subject: &str) -> Result<()> {
    verify_payload_at(payload, key, subject, chrono::Utc::now().timestamp())
}

/// [`verify_payload`], holding the signature's time to `published_at` (Unix seconds)
/// rather than to now. A message JetStream kept while the engine was down is checked
/// against when the server received it, so it is not refused for having waited; one
/// published again later gets a new time from the server, and is refused.
pub fn verify_payload_at(
    payload: &Value,
    key: &[u8],
    subject: &str,
    published_at: i64,
) -> Result<()> {
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
    if (published_at - signed_at).abs() > MAX_SIGNATURE_AGE_SECS {
        bail!("the signature is too old, or from too far in the future");
    }
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::verify(
        &key,
        &signed_bytes(subject, signed_at, &Value::Object(object)),
        &given,
    )
    .map_err(|_| anyhow!("the signature does not match"))
}

fn mac(key: &[u8; 32], subject: &str, signed_at: i64, payload: &Value) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&key, &signed_bytes(subject, signed_at, payload))
        .as_ref()
        .to_vec()
}

/// What is signed: a domain label, the subject without its shard, the time, and the
/// payload as serde_json writes it. The engine parses the text the runner wrote and
/// writes it again, which gives the same bytes: object keys come out in the same order
/// either way.
fn signed_bytes(subject: &str, signed_at: i64, payload: &Value) -> Vec<u8> {
    let subject = normalize_subject(subject);
    let mut bytes = format!("{MAC_DOMAIN}\n{subject}\n{signed_at}\n").into_bytes();
    bytes.extend(serde_json::to_vec(payload).unwrap_or_default());
    bytes
}

const REPLY_INFO: &[u8] = b"stormchaser runner reply v1";

/// The key that seals the engine's answers to a runner, derived from the runner's key
/// so that the same bytes are never used both to sign and to encrypt.
fn reply_key(session_key: &[u8]) -> Result<aead::LessSafeKey> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]).extract(session_key);
    let okm = prk
        .expand(&[REPLY_INFO], &aead::CHACHA20_POLY1305)
        .map_err(|_| anyhow!("key derivation failed"))?;
    Ok(aead::LessSafeKey::new(aead::UnboundKey::from(okm)))
}

/// Seals `plain` for the runner whose key is `session_key`. `context` (the runner and
/// step it is about, say) is bound to the result: it opens only with the same context.
///
/// Runners share one NATS user, and so one inbox prefix, so another runner can see an
/// answer meant for this one; sealed, it cannot read it.
pub fn seal_for_runner(session_key: &[u8], context: &str, plain: &[u8]) -> Result<String> {
    let key = reply_key(session_key)?;
    let mut nonce = [0u8; aead::NONCE_LEN];
    rand::SecureRandom::fill(&rand::SystemRandom::new(), &mut nonce)
        .map_err(|_| anyhow!("no randomness"))?;
    let mut sealed = plain.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(context.as_bytes()),
        &mut sealed,
    )
    .map_err(|_| anyhow!("could not seal the answer"))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Ok(b64().encode(out))
}

/// Opens what [`seal_for_runner`] sealed, with this process's key.
pub fn open_from_engine(context: &str, sealed: &str) -> Result<Vec<u8>> {
    let session_key = (*SESSION.read()).context("this process has no key to open the answer")?;
    open_with(&session_key, context, sealed)
}

fn open_with(session_key: &[u8], context: &str, sealed: &str) -> Result<Vec<u8>> {
    let key = reply_key(session_key)?;
    let sealed = b64().decode(sealed).context("the answer is not base64")?;
    if sealed.len() < aead::NONCE_LEN {
        bail!("the answer is too short");
    }
    let (nonce, ciphertext) = sealed.split_at(aead::NONCE_LEN);
    let mut buffer = ciphertext.to_vec();
    let plain = key
        .open_in_place(
            aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| anyhow!("bad nonce"))?,
            aead::Aad::from(context.as_bytes()),
            &mut buffer,
        )
        .map_err(|_| anyhow!("the answer was not sealed for this runner"))?;
    Ok(plain.to_vec())
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
        let subject = "stormchaser.v1.3.step.completed";
        let signed = sign_payload(payload.clone(), &key, subject);

        // Through text and back, as the engine receives it
        let received: Value = serde_json::from_str(&signed.to_string()).unwrap();
        verify_payload(&received, &key, subject).unwrap();
        // The engine checks the subject without its shard
        verify_payload(&received, &key, "stormchaser.v1.step.completed").unwrap();

        assert!(verify_payload(&received, &[8u8; 32], subject).is_err());
        let mut tampered = received.clone();
        tampered["runner_id"] = json!("r2");
        assert!(verify_payload(&tampered, &key, subject).is_err());
        assert!(verify_payload(&payload, &key, subject).is_err());

        let mut stale = received.clone();
        stale[SIGNATURE_FIELD]["signed_at"] =
            json!(chrono::Utc::now().timestamp() - MAX_SIGNATURE_AGE_SECS - 10);
        assert!(verify_payload(&stale, &key, subject).is_err());
    }

    #[test]
    fn a_signature_is_timed_against_when_its_message_was_published() {
        let key = [7u8; 32];
        let subject = "stormchaser.v1.step.completed";
        let signed = sign_payload(json!({ "runner_id": "r1" }), &key, subject);
        let signed_at = signed[SIGNATURE_FIELD]["signed_at"].as_i64().unwrap();

        // Kept by the stream an hour and delivered now: still good
        verify_payload_at(&signed, &key, subject, signed_at + 1).unwrap();
        // Published again an hour later: refused
        assert!(verify_payload_at(&signed, &key, subject, signed_at + 3600).is_err());
    }

    #[test]
    fn a_signed_message_does_not_verify_on_another_subject() {
        let key = [7u8; 32];
        let signed = sign_payload(
            json!({ "runner_id": "r1", "run_id": "x", "step_id": "y" }),
            &key,
            "stormchaser.v1.0.step.running",
        );
        assert!(verify_payload(&signed, &key, "stormchaser.v1.global.runner.offline").is_err());
        assert!(verify_payload(&signed, &key, "stormchaser.v1.step.completed").is_err());
    }

    #[test]
    fn a_sealed_answer_opens_only_with_its_key_and_context() {
        let sealed = seal_for_runner(&[3u8; 32], "r1/step", b"secret").unwrap();
        assert_eq!(
            open_with(&[3u8; 32], "r1/step", &sealed).unwrap(),
            b"secret"
        );
        assert!(open_with(&[4u8; 32], "r1/step", &sealed).is_err());
        assert!(open_with(&[3u8; 32], "r2/step", &sealed).is_err());
    }

    #[test]
    fn subjects_lose_their_shard() {
        assert_eq!(
            normalize_subject("stormchaser.v1.12.step.completed"),
            "stormchaser.v1.step.completed"
        );
        assert_eq!(
            normalize_subject("stormchaser.v1.global.runner.offline"),
            "stormchaser.v1.runner.offline"
        );
        assert_eq!(
            normalize_subject("stormchaser.rpc.step.artifact_auth"),
            "stormchaser.rpc.step.artifact_auth"
        );
    }
}
