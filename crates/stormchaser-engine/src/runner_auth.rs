//! The engine's side of runner identity; see [`stormchaser_model::runner_auth`].

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::runner_auth::{
    issue_session, signatures_required, verify_payload_at, SessionRequest, SessionResponse,
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

/// Checks that a message naming a runner, received on `subject`, was signed with that
/// runner's key for that subject, at about `published_at` (Unix seconds: when the
/// server received it). Returns why it is refused, if it is, or an error if that could
/// not be decided (the database could not be read), in which case the message should be
/// looked at again later rather than dropped.
pub async fn refusal(
    pool: &PgPool,
    sender: Sender,
    subject: &str,
    payload: &Value,
    published_at: i64,
) -> Result<Option<String>> {
    let runner_id = payload.get("runner_id").and_then(Value::as_str);
    let key = match runner_id {
        Some(id) => crate::db::get_runner_session_key(pool, id)
            .await
            .with_context(|| format!("could not look up the key of runner {id}"))?,
        None => None,
    };

    // The engine's own verdict that a step's runner is gone names that runner, which may
    // still have a key, but the engine cannot sign as it. The failed-step handler checks
    // independently that the runner really is lost before it acts on the verdict, so
    // anyone sending it gains nothing the engine would not do anyway.
    if sender == Sender::RunnerOrControlPlane
        && payload.get("error").and_then(Value::as_str) == Some("lost_zombie")
    {
        return Ok(None);
    }

    if let (Some(key), Some(id)) = (&key, runner_id) {
        return Ok(verify_payload_at(payload, key, subject, published_at)
            .err()
            .map(|e| format!("not signed by runner {id}: {e:#}")));
    }

    // No key on record for whoever this claims to be
    let names_a_runner = match (sender, runner_id) {
        (Sender::Runner, _) => true,
        // The engine and the API name no registered runner when they finish a step
        (Sender::RunnerOrControlPlane, Some(id)) => crate::db::runner_is_known(pool, id)
            .await
            .with_context(|| format!("could not look up runner {id}"))?,
        (Sender::RunnerOrControlPlane, None) => false,
    };
    if !names_a_runner {
        return Ok(None);
    }
    if !signatures_required() && runner_id.is_some() {
        tracing::warn!(
            "Accepting an unsigned message from runner {:?}, which has no key ({}=optional)",
            runner_id,
            stormchaser_model::runner_auth::SIGNATURES_ENV
        );
        return Ok(None);
    }
    Ok(Some(format!(
        "runner {:?} has no key from this engine; runners must be upgraded with it, or set {}=optional while they are",
        runner_id,
        stormchaser_model::runner_auth::SIGNATURES_ENV
    )))
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
    if is_reserved_runner_id(id) {
        tracing::warn!(
            "Refused a key for runner {}: the id is the engine's own",
            id
        );
        return SessionResponse::refused("this runner id is reserved for the engine");
    }
    let (key, response) = match issue_session(request) {
        Ok(issued) => issued,
        Err(e) => return SessionResponse::refused(&format!("{e:#}")),
    };
    match crate::db::claim_runner_session(pool, id, &key).await {
        Ok(true) => {
            tracing::info!("Issued a key to runner {}", id);
            response
        }
        Ok(false) => {
            tracing::warn!(
                "Refused a key for runner {}: a runner with that id is still running, or was given a key moments ago",
                id
            );
            SessionResponse::refused(
                "a runner with this id is still running, or was given a key in the last 30 seconds; it must stop, or miss its heartbeats for 30 seconds, before another may take its id",
            )
        }
        Err(e) => {
            tracing::error!("Could not store the key of runner {}: {:?}", id, e);
            SessionResponse::refused("the engine could not store the key")
        }
    }
}

/// The runner ids the engine names itself by when it reports on steps it runs (SQL and
/// REST API steps). No runner may hold a key for one: the router would then demand that
/// runner's signature on the engine's own reports, and refuse them.
pub const RESERVED_RUNNER_IDS: &[&str] = &["intrinsic-sql", "engine-intrinsic"];

pub fn is_reserved_runner_id(id: &str) -> bool {
    RESERVED_RUNNER_IDS.contains(&id)
}
