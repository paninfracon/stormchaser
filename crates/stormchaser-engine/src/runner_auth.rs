//! The engine's side of runner identity; see [`stormchaser_model::runner_auth`].

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::runner_auth::{
    issue_session, signatures_required, verify_payload, SessionRequest, SessionResponse,
};

/// Who may send a message on a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sender {
    /// Only a runner, which must sign it.
    Runner,
    /// A runner (which must sign it) or the engine or API, for steps they finish
    /// themselves. Those are held to the step's completion token instead.
    RunnerOrControlPlane,
}

/// Who may send on `subject` (normalized, without a shard), if it is a runner's.
pub fn sender_of(subject: &str) -> Option<Sender> {
    match subject {
        "stormchaser.v1.runner.register"
        | "stormchaser.v1.runner.heartbeat"
        | "stormchaser.v1.runner.offline"
        | "stormchaser.v1.step.initializing"
        | "stormchaser.v1.step.running"
        | "stormchaser.v1.step.unpacking_sfs"
        | "stormchaser.v1.step.packing_sfs"
        | stormchaser_model::nats::ARTIFACT_AUTH_SUBJECT => Some(Sender::Runner),
        "stormchaser.v1.step.completed" | "stormchaser.v1.step.failed" => {
            Some(Sender::RunnerOrControlPlane)
        }
        _ => None,
    }
}

/// Checks that a message naming a runner was signed with that runner's key. Returns
/// why it is refused, if it is.
pub async fn refusal(pool: &PgPool, sender: Sender, payload: &Value) -> Option<String> {
    let runner_id = payload.get("runner_id").and_then(Value::as_str);
    let key = match runner_id {
        Some(id) => match crate::db::get_runner_session_key(pool, id).await {
            Ok(key) => key,
            Err(e) => return Some(format!("could not look up the key of runner {id}: {e}")),
        },
        None => None,
    };

    if let (Some(key), Some(id)) = (&key, runner_id) {
        return verify_payload(payload, key)
            .err()
            .map(|e| format!("not signed by runner {id}: {e:#}"));
    }

    // No key on record for whoever this claims to be
    let names_a_runner = match (sender, runner_id) {
        (Sender::Runner, _) => true,
        // The engine and the API name no registered runner when they finish a step
        (Sender::RunnerOrControlPlane, Some(id)) => {
            crate::db::runner_is_known(pool, id).await.unwrap_or(true)
        }
        (Sender::RunnerOrControlPlane, None) => false,
    };
    if !names_a_runner {
        return None;
    }
    if !signatures_required() && runner_id.is_some() {
        tracing::warn!(
            "Accepting an unsigned message from runner {:?}, which has no key ({}=optional)",
            runner_id,
            stormchaser_model::runner_auth::SIGNATURES_ENV
        );
        return None;
    }
    Some(format!(
        "runner {:?} has no key from this engine; runners must be upgraded with it, or set {}=optional while they are",
        runner_id,
        stormchaser_model::runner_auth::SIGNATURES_ENV
    ))
}

/// Answers a runner's request for its key.
pub async fn handle_session_request(
    payload: &[u8],
    pool: PgPool,
    nats_client: async_nats::Client,
    reply: Option<String>,
) -> Result<()> {
    let reply = reply.context("a key request needs a reply subject")?;
    let response = match serde_json::from_slice::<SessionRequest>(payload) {
        Ok(request) => session_for(&pool, &request).await,
        Err(e) => SessionResponse::refused(&format!("not a key request: {e}")),
    };
    nats_client
        .publish(reply, serde_json::to_vec(&response)?.into())
        .await?;
    Ok(())
}

async fn session_for(pool: &PgPool, request: &SessionRequest) -> SessionResponse {
    let id = request.runner_id.as_str();
    if id.is_empty() || id.len() > 253 {
        return SessionResponse::refused("the runner id is empty or too long");
    }
    match crate::db::runner_session_is_live(pool, id).await {
        Ok(true) => {
            tracing::warn!(
                "Refused a key for runner {}: a runner with that id is still sending heartbeats",
                id
            );
            return SessionResponse::refused(
                "a runner with this id is still running; it must stop, or miss its heartbeats for 30 seconds, before another may take its id",
            );
        }
        Ok(false) => {}
        Err(e) => {
            tracing::error!("Could not check runner {}: {:?}", id, e);
            return SessionResponse::refused("the engine could not check this runner id");
        }
    }
    let (key, response) = match issue_session(request) {
        Ok(issued) => issued,
        Err(e) => return SessionResponse::refused(&format!("{e:#}")),
    };
    if let Err(e) = crate::db::store_runner_session(pool, id, &key).await {
        tracing::error!("Could not store the key of runner {}: {:?}", id, e);
        return SessionResponse::refused("the engine could not store the key");
    }
    tracing::info!("Issued a key to runner {}", id);
    response
}
