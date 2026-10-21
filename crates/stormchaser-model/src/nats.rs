#![cfg(not(target_arch = "wasm32"))]
use crate::id::EventId;
use crate::RunId;
use anyhow::{Context, Result};
use async_nats::jetstream;
use async_nats::HeaderMap;
use cloudevents::{AttributesReader, EventBuilder, EventBuilderV10};
use schemars::Schema;
use serde_json::Value;
use std::borrow::Cow;
use tracing::{error, warn};

/// Applies NATS credentials and TLS settings from the environment to `options`.
///
/// Every component connects through this so that an operator can turn on NATS
/// authentication in one place. Credentials come from, in order of preference:
/// `NATS_CREDS_FILE` (a `.creds` file), `NATS_USER` + `NATS_PASSWORD`, or `NATS_TOKEN`.
/// `NATS_CA_FILE` adds a root certificate and `NATS_TLS_REQUIRED=true` refuses to talk
/// to a server without TLS.
///
/// Replies (request answers, publish acknowledgements, pull-consumer deliveries) come
/// to an inbox under [`inbox_prefix`], so the server can let each user read only its
/// own: with every user on the shared `_INBOX.>`, any of them could read the engine's
/// deliveries and the answers the engine sends to others.
pub async fn apply_auth_from_env(
    mut options: async_nats::ConnectOptions,
) -> Result<async_nats::ConnectOptions> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());

    if let Some(path) = var("NATS_CREDS_FILE") {
        options = options
            .credentials_file(&path)
            .await
            .with_context(|| format!("Failed to read NATS credentials file {}", path))?;
    } else if let (Some(user), Some(password)) = (var("NATS_USER"), var("NATS_PASSWORD")) {
        options = options.user_and_password(user, password);
    } else if let Some(token) = var("NATS_TOKEN") {
        options = options.token(token);
    } else {
        warn!(
            "Connecting to NATS without credentials; set NATS_CREDS_FILE, NATS_USER/NATS_PASSWORD or NATS_TOKEN"
        );
    }

    options = options.custom_inbox_prefix(inbox_prefix());

    if let Some(ca) = var("NATS_CA_FILE") {
        options = options.add_root_certificates(ca.into());
    }
    if var("NATS_TLS_REQUIRED").is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1") {
        options = options.require_tls(true);
    }

    Ok(options)
}

/// Where this process's replies arrive: `NATS_INBOX_PREFIX` if set, otherwise
/// `_INBOX_<NATS_USER>` for a user and password login, otherwise NATS's own `_INBOX`.
/// The server configuration lets each user subscribe to its own prefix only.
pub fn inbox_prefix() -> String {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    if let Some(prefix) = var("NATS_INBOX_PREFIX") {
        return prefix;
    }
    match var("NATS_USER") {
        Some(user) => format!("_INBOX_{user}"),
        None => "_INBOX".to_string(),
    }
}

/// Connects to NATS with the credentials and TLS settings found in the environment.
///
/// Use this instead of `async_nats::connect`, which would connect anonymously and be
/// refused by a server that requires authentication.
pub async fn connect(addrs: impl async_nats::ToServerAddrs) -> Result<async_nats::Client> {
    let options = apply_auth_from_env(async_nats::ConnectOptions::new()).await?;
    Ok(async_nats::connect_with_options(addrs, options).await?)
}

pub fn get_total_shards() -> u32 {
    std::env::var("STORMCHASER_TOTAL_SHARDS")
        .unwrap_or_else(|_| "1".to_string())
        .parse()
        .unwrap_or(1)
}

pub fn compute_shard_id(run_id: &RunId) -> u32 {
    let total_shards = get_total_shards();
    if total_shards <= 1 {
        return 0;
    }
    crc32fast::hash(run_id.to_string().as_bytes()) % total_shards
}

/// Validates a JSON value against a compiled JSON Schema.
///
/// Returns `Ok(())` if the value is valid. If the schema is `None`, validation is
/// skipped (permissive). If the schema is provided and validation fails, the
/// mismatch is logged and an error is returned.
pub fn validate_against_schema(data: &Value, schema: Option<&Schema>) -> Result<()> {
    let schema = match schema {
        Some(s) => s,
        None => return Ok(()),
    };

    let schema_value = serde_json::to_value(schema)?;
    let compiled = jsonschema::validator_for(&schema_value)
        .map_err(|e| anyhow::anyhow!("Failed to compile schema: {}", e))?;

    let errors: Vec<String> = compiled
        .iter_errors(data)
        // Masked: the default message quotes the offending value, which may be a secret
        .map(|e| format!("{} at '{}'", e.masked(), e.instance_path()))
        .collect();

    if errors.is_empty() {
        Ok(())
    } else {
        let msg = errors.join("; ");
        error!("CloudEvent payload failed schema validation: {}", msg);
        Err(anyhow::anyhow!("Schema validation failed: {}", msg))
    }
}

/// Extracts JSON data from a CloudEvent and validates it against an optional schema.
///
/// Returns `Ok(value)` when the payload is valid (or no schema is provided).
/// Returns `Err` when the CloudEvent contains no JSON data, or when a schema
/// is available but the payload does not conform to it.
pub fn extract_and_validate(event: &cloudevents::Event, schema: Option<&Schema>) -> Result<Value> {
    let data = match event.data() {
        Some(cloudevents::Data::Json(v)) => v.clone(),
        Some(_) => anyhow::bail!("CloudEvent data is not JSON"),
        None => anyhow::bail!("CloudEvent has no data"),
    };

    if let Some(s) = schema {
        if let Err(e) = validate_against_schema(&data, Some(s)) {
            tracing::error!(
                "Rejecting CloudEvent data for event type '{}': schema mismatch: {}",
                event.ty(),
                e
            );
            return Err(e);
        }
    }

    Ok(data)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NatsSubject {
    RunQueued(Option<u32>),
    RunDirect(Option<u32>),
    RunRunning(Option<u32>),
    RunCompleted(Option<u32>),
    RunFailed(Option<u32>),
    RunAborted(Option<u32>),
    RunnerRegister,
    RunnerHeartbeat,
    RunnerOffline,
    /// Engine to every runner: stop the steps of a run. Runners may read it, not publish it.
    RunnerCancel,
    StepScheduled(String, Option<u32>),
    StepInitializing(Option<u32>),
    StepRunning(Option<u32>),
    StepCompleted(Option<u32>),
    StepFailed(Option<u32>),
    StepQuery(Option<u32>),
    StepUnpackingSfs(Option<u32>),
    StepPackingSfs(Option<u32>),
    RunStartPending(Option<u32>),
    Custom(String),
}

impl NatsSubject {
    pub fn as_str(&self) -> Cow<'static, str> {
        let get_shard_str = |shard: &Option<u32>| -> String {
            match shard {
                Some(id) => id.to_string(),
                None => "global".to_string(),
            }
        };

        match self {
            NatsSubject::RunQueued(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.run.queued",
                get_shard_str(shard)
            )),
            NatsSubject::RunStartPending(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.run.start_pending",
                get_shard_str(shard)
            )),
            NatsSubject::RunDirect(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.run.direct",
                get_shard_str(shard)
            )),
            NatsSubject::RunRunning(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.run.running",
                get_shard_str(shard)
            )),
            NatsSubject::RunCompleted(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.run.completed",
                get_shard_str(shard)
            )),
            NatsSubject::RunFailed(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.run.failed",
                get_shard_str(shard)
            )),
            NatsSubject::RunAborted(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.run.aborted",
                get_shard_str(shard)
            )),
            NatsSubject::RunnerRegister => Cow::Borrowed("stormchaser.v1.global.runner.register"),
            NatsSubject::RunnerHeartbeat => Cow::Borrowed("stormchaser.v1.global.runner.heartbeat"),
            NatsSubject::RunnerOffline => Cow::Borrowed("stormchaser.v1.global.runner.offline"),
            NatsSubject::RunnerCancel => Cow::Borrowed(crate::runner::CANCEL_SUBJECT),
            NatsSubject::StepScheduled(ty, shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.scheduled.{}",
                get_shard_str(shard),
                ty.to_lowercase()
            )),
            NatsSubject::StepInitializing(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.initializing",
                get_shard_str(shard)
            )),
            NatsSubject::StepRunning(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.running",
                get_shard_str(shard)
            )),
            NatsSubject::StepCompleted(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.completed",
                get_shard_str(shard)
            )),
            NatsSubject::StepFailed(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.failed",
                get_shard_str(shard)
            )),
            NatsSubject::StepQuery(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.query",
                get_shard_str(shard)
            )),
            NatsSubject::StepUnpackingSfs(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.unpacking_sfs",
                get_shard_str(shard)
            )),
            NatsSubject::StepPackingSfs(shard) => Cow::Owned(format!(
                "stormchaser.v1.{}.step.packing_sfs",
                get_shard_str(shard)
            )),
            NatsSubject::Custom(s) => Cow::Owned(s.clone()),
        }
    }
}

use crate::events::{EventSource, EventType, SchemaId, SchemaVersion};

/// Pure function to construct a CloudEvent payload, to be published on `subject`, and
/// its corresponding NATS headers.
pub fn build_cloudevent_and_headers(
    subject: &str,
    event_type: EventType,
    source: EventSource,
    data: Value,
    schema_version: Option<SchemaVersion>,
    schema_id: Option<SchemaId>,
) -> Result<(String, HeaderMap)> {
    // A runner signs what it reports, once the engine has given it a key
    let data = crate::runner_auth::sign_if_runner(data, subject);
    let mut builder = EventBuilderV10::new()
        .id(EventId::new_v4().into_inner().to_string())
        .ty(event_type.as_str())
        .source(source.as_str())
        .time(chrono::Utc::now())
        .data(crate::APPLICATION_JSON, data);

    if let Some(id) = &schema_id {
        builder = builder.extension("dataschema", id.clone().into_inner());
    }

    let event = builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build CloudEvent: {}", e))?;

    let payload = serde_json::to_string(&event)?;

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/cloudevents+json");
    if let Some(v) = schema_version {
        headers.insert("ce-schemaid", v.into_inner().as_str());
    }

    Ok((payload, headers))
}

/// The subject runners ask the engine on for the registry credentials of a step's
/// artifact pushes.
pub const ARTIFACT_AUTH_SUBJECT: &str = "stormchaser.rpc.step.artifact_auth";

/// Where a runner asks whether a step it found is still wanted.
pub const STEP_QUERY_SUBJECT: &str = "stormchaser.rpc.step.query";

/// The subjects the `stormchaser` stream captures. Request-reply subjects
/// (`stormchaser.rpc.>`) must stay outside it: the stream acknowledges every message it
/// stores to the message's reply subject, so a request it captured would be answered
/// by that acknowledgement instead of by the engine.
pub const STREAM_SUBJECTS: &str = "stormchaser.v1.>";

/// Where a runner puts the registry credentials inside its parking helper, and the agent
/// reads them. Step volumes can never be mounted over this path.
pub const HELPER_REGISTRY_CONFIG_PATH: &str = "/stormchaser/auth/config.json";

/// Asks the engine for the Docker client configuration a step's helper needs to push the
/// step's OCI artifacts.
///
/// The engine keeps registry usernames and passwords to itself and answers with a token
/// scoped to the run's own repositories, minted now so it is still valid for the push.
pub async fn request_artifact_auth(
    client: &async_nats::Client,
    run_id: uuid::Uuid,
    step_id: uuid::Uuid,
    runner_id: Option<&str>,
) -> Result<Value> {
    let event = EventBuilderV10::new()
        .id(uuid::Uuid::new_v4().to_string())
        .ty(ARTIFACT_AUTH_SUBJECT)
        .source(EventSource::System.as_str())
        .time(chrono::Utc::now())
        .data(
            crate::APPLICATION_JSON,
            crate::runner_auth::sign_if_runner(
                serde_json::json!({ "run_id": run_id, "step_id": step_id, "runner_id": runner_id }),
                ARTIFACT_AUTH_SUBJECT,
            ),
        )
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build CloudEvent: {}", e))?;

    let reply = client
        .request(ARTIFACT_AUTH_SUBJECT, serde_json::to_vec(&event)?.into())
        .await
        .map_err(|e| anyhow::anyhow!("no answer from the engine: {}", e))?;
    let answer: Value = serde_json::from_slice(&reply.payload)?;

    if answer["ok"].as_bool() != Some(true) {
        anyhow::bail!(
            "the engine refused: {}",
            answer["error"].as_str().unwrap_or("no reason given")
        );
    }
    // Sealed with this runner's key; plain only for a runner the engine gave no key
    if let Some(sealed) = answer["sealed_docker_config"].as_str() {
        let context = artifact_auth_context(runner_id.unwrap_or_default(), step_id);
        let plain = crate::runner_auth::open_from_engine(&context, sealed)?;
        return Ok(serde_json::from_slice(&plain)?);
    }
    Ok(answer["docker_config"].clone())
}

/// What the engine's answer to an artifact credentials request is bound to.
pub fn artifact_auth_context(runner_id: &str, step_id: uuid::Uuid) -> String {
    format!("{ARTIFACT_AUTH_SUBJECT}\n{runner_id}\n{step_id}")
}

/// Tells the engine that `runner_id` is now the runner executing a step.
///
/// Sent when a step starts, and again by a runner that picks up a step someone else
/// started (after a restart, or when a task is redelivered), so that the engine follows
/// the step to the runner that actually holds it.
pub async fn announce_step_running(
    client: &async_nats::Client,
    run_id: uuid::Uuid,
    step_id: uuid::Uuid,
    attempt: Option<i32>,
    runner_id: &str,
) -> Result<()> {
    let run_id = crate::RunId::new(run_id);
    let event = crate::events::StepRunningEvent {
        run_id,
        step_id: crate::StepInstanceId::new(step_id),
        attempt,
        event_type: EventType::Step(crate::events::StepEventType::Running),
        runner_id: Some(runner_id.to_string()),
        timestamp: chrono::Utc::now(),
    };
    publish_cloudevent(
        &jetstream::new(client.clone()),
        NatsSubject::StepRunning(Some(compute_shard_id(&run_id))),
        EventType::Step(crate::events::StepEventType::Running),
        EventSource::System,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
}

/// Tells the engine that `runner_id` is unpacking (`packing` false) or packing the
/// storage of a step. Signed like every other report, so the engine can check it comes
/// from the runner that holds the step.
pub async fn announce_step_storage(
    client: &async_nats::Client,
    run_id: uuid::Uuid,
    step_id: uuid::Uuid,
    attempt: Option<i32>,
    runner_id: &str,
    packing: bool,
) -> Result<()> {
    let run_id = crate::RunId::new(run_id);
    let shard = Some(compute_shard_id(&run_id));
    let (subject, event_type, status) = if packing {
        (
            NatsSubject::StepPackingSfs(shard),
            crate::events::StepEventType::PackingSfs,
            crate::step::StepStatus::PackingSfs,
        )
    } else {
        (
            NatsSubject::StepUnpackingSfs(shard),
            crate::events::StepEventType::UnpackingSfs,
            crate::step::StepStatus::UnpackingSfs,
        )
    };
    publish_cloudevent(
        &jetstream::new(client.clone()),
        subject,
        EventType::Step(event_type),
        EventSource::System,
        serde_json::json!({
            "run_id": run_id,
            "step_id": step_id,
            "attempt": attempt,
            "runner_id": runner_id,
            "status": status,
            "timestamp": chrono::Utc::now(),
        }),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
}

/// Name of the JetStream stream every Stormchaser component shares.
pub const STREAM_NAME: &str = "stormchaser";

/// How long the stream keeps a message when nothing erases it sooner.
///
/// Set in hours by `STORMCHASER_NATS_STREAM_MAX_AGE_HOURS` (default 168, one week);
/// `0` means no limit. Task messages hold step secrets, so the stream should never be
/// left to grow without bound.
pub fn stream_max_age() -> std::time::Duration {
    let hours = std::env::var("STORMCHASER_NATS_STREAM_MAX_AGE_HOURS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(168);
    std::time::Duration::from_secs(hours.saturating_mul(3600))
}

/// Waits until the shared stream exists, and returns it.
///
/// Runners only look the stream up. Creating it is the engine's job (or the operator's),
/// so a runner's credential cannot create it first with settings of its own choosing,
/// such as a stream whose messages can never be deleted.
pub async fn wait_for_stream(js: &jetstream::Context) -> jetstream::stream::Stream {
    let mut delay = std::time::Duration::from_secs(1);
    loop {
        match js.get_stream(STREAM_NAME).await {
            Ok(stream) => return stream,
            Err(e) => {
                tracing::warn!(
                    "JetStream stream '{}' is not available yet ({}); the engine creates it. \
                     Retrying in {}s",
                    STREAM_NAME,
                    e,
                    delay.as_secs()
                );
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(std::time::Duration::from_secs(30));
            }
        }
    }
}

/// Publishes like [`publish_cloudevent`] and returns the message's sequence in the stream.
///
/// `None` means the message was sent but the stream did not confirm storing it.
pub async fn publish_cloudevent_with_sequence(
    js: &jetstream::Context,
    subject: NatsSubject,
    event_type: EventType,
    source: EventSource,
    data: Value,
    schema_version: Option<SchemaVersion>,
    schema_id: Option<SchemaId>,
) -> Result<Option<u64>> {
    let subject = subject.as_str().into_owned();
    let (payload, headers) = build_cloudevent_and_headers(
        &subject,
        event_type,
        source,
        data,
        schema_version,
        schema_id,
    )?;

    let ack = js
        .publish_with_headers(subject, headers, payload.into())
        .await?;

    Ok(ack.await.ok().map(|ack| ack.sequence))
}

/// Helper function to wrap a JSON payload in a CloudEvent and publish it to NATS with schema headers.
pub async fn publish_cloudevent(
    js: &jetstream::Context,
    subject: NatsSubject,
    event_type: EventType,
    source: EventSource,
    data: Value,
    schema_version: Option<SchemaVersion>,
    schema_id: Option<SchemaId>,
) -> Result<()> {
    let subject = subject.as_str().into_owned();
    let (payload, headers) = build_cloudevent_and_headers(
        &subject,
        event_type,
        source,
        data,
        schema_version,
        schema_id,
    )?;

    js.publish_with_headers(subject, headers, payload.into())
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, schemars::JsonSchema)]
    struct TestEvent {
        id: u32,
        name: String,
    }

    #[test]
    fn test_validate_against_schema_valid() {
        let schema = crate::schema_gen::draft07_schema_for::<TestEvent>();
        let data = serde_json::json!({"id": 1, "name": "test"});
        validate_against_schema(&data, Some(&schema)).unwrap();
    }

    #[test]
    fn test_validate_against_schema_invalid() {
        let schema = crate::schema_gen::draft07_schema_for::<TestEvent>();
        // `id` must be a number, not a string
        let data = serde_json::json!({"id": "not-a-number", "name": "test"});
        validate_against_schema(&data, Some(&schema)).unwrap_err();
    }

    #[test]
    fn test_validate_against_schema_none_is_permissive() {
        let data = serde_json::json!({"anything": true});
        validate_against_schema(&data, None).unwrap();
    }

    #[test]
    fn test_build_cloudevent_and_headers_includes_schema_headers() {
        let event_type = EventType::Workflow(crate::events::WorkflowEventType::Queued);
        let source = EventSource::System;
        let data = serde_json::json!({"test": true});
        let schema_version = Some(SchemaVersion::new("1.0".to_string()));
        let schema_id = Some(SchemaId::new("test-schema-id".to_string()));

        let (payload, headers) = build_cloudevent_and_headers(
            "stormchaser.v1.global.run.queued",
            event_type,
            source,
            data,
            schema_version,
            schema_id,
        )
        .expect("Failed to build cloudevent and headers");

        assert_eq!(
            headers.get("ce-schemaid").map(|v| v.as_str()).unwrap_or(""),
            "1.0"
        );
        assert_eq!(
            headers.get("Content-Type").unwrap().as_str(),
            "application/cloudevents+json"
        );

        // Verify payload is valid JSON and contains the event data
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("Payload is not valid JSON");
        assert_eq!(parsed["dataschema"], "test-schema-id");
        assert_eq!(parsed["type"], "WorkflowQueuedEvent");
        assert_eq!(parsed["source"], "/stormchaser");
        assert_eq!(parsed["data"]["test"], true);
    }
}
