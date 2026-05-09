use crate::id::EventId;
use anyhow::Result;
use async_nats::jetstream;
use async_nats::HeaderMap;
use cloudevents::{AttributesReader, EventBuilder, EventBuilderV10};
use schemars::schema::RootSchema;
use serde_json::Value;
use std::borrow::Cow;
use tracing::error;

/// Validates a JSON value against a compiled JSON Schema.
///
/// Returns `Ok(())` if the value is valid. If the schema is `None`, validation is
/// skipped (permissive). If the schema is provided and validation fails, the
/// mismatch is logged and an error is returned.
pub fn validate_against_schema(data: &Value, schema: Option<&RootSchema>) -> Result<()> {
    let schema = match schema {
        Some(s) => s,
        None => return Ok(()),
    };

    let schema_value = serde_json::to_value(schema)?;
    let compiled = jsonschema::validator_for(&schema_value)
        .map_err(|e| anyhow::anyhow!("Failed to compile schema: {}", e))?;

    let errors: Vec<String> = compiled
        .iter_errors(data)
        .map(|e| format!("{}", e))
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
pub fn extract_and_validate(
    event: &cloudevents::Event,
    schema: Option<&RootSchema>,
) -> Result<Value> {
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
    RunQueued,
    RunDirect,
    RunRunning,
    RunCompleted,
    RunFailed,
    RunAborted,
    RunnerRegister,
    RunnerHeartbeat,
    RunnerOffline,
    StepScheduled(String),
    StepRunning,
    StepCompleted,
    StepFailed,
    StepQuery,
    StepUnpackingSfs,
    StepPackingSfs,
    RunStartPending,
    Custom(String),
}

impl NatsSubject {
    pub fn as_str(&self) -> Cow<'static, str> {
        match self {
            NatsSubject::RunQueued => Cow::Borrowed("stormchaser.v1.run.queued"),
            NatsSubject::RunStartPending => Cow::Borrowed("stormchaser.v1.run.start_pending"),
            NatsSubject::RunDirect => Cow::Borrowed("stormchaser.v1.run.direct"),
            NatsSubject::RunRunning => Cow::Borrowed("stormchaser.v1.run.running"),
            NatsSubject::RunCompleted => Cow::Borrowed("stormchaser.v1.run.completed"),
            NatsSubject::RunFailed => Cow::Borrowed("stormchaser.v1.run.failed"),
            NatsSubject::RunAborted => Cow::Borrowed("stormchaser.v1.run.aborted"),
            NatsSubject::RunnerRegister => Cow::Borrowed("stormchaser.v1.runner.register"),
            NatsSubject::RunnerHeartbeat => Cow::Borrowed("stormchaser.v1.runner.heartbeat"),
            NatsSubject::RunnerOffline => Cow::Borrowed("stormchaser.v1.runner.offline"),
            NatsSubject::StepScheduled(ty) => Cow::Owned(format!(
                "stormchaser.v1.step.scheduled.{}",
                ty.to_lowercase()
            )),
            NatsSubject::StepRunning => Cow::Borrowed("stormchaser.v1.step.running"),
            NatsSubject::StepCompleted => Cow::Borrowed("stormchaser.v1.step.completed"),
            NatsSubject::StepFailed => Cow::Borrowed("stormchaser.v1.step.failed"),
            NatsSubject::StepQuery => Cow::Borrowed("stormchaser.v1.step.query"),
            NatsSubject::StepUnpackingSfs => Cow::Borrowed("stormchaser.v1.step.unpacking_sfs"),
            NatsSubject::StepPackingSfs => Cow::Borrowed("stormchaser.v1.step.packing_sfs"),
            NatsSubject::Custom(s) => Cow::Owned(s.clone()),
        }
    }
}

use crate::events::{EventSource, EventType, SchemaId, SchemaVersion};

/// Pure function to construct a CloudEvent payload and its corresponding NATS headers.
pub fn build_cloudevent_and_headers(
    event_type: EventType,
    source: EventSource,
    data: Value,
    schema_version: Option<SchemaVersion>,
    schema_id: Option<SchemaId>,
) -> Result<(String, HeaderMap)> {
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
    let (payload, headers) =
        build_cloudevent_and_headers(event_type, source, data, schema_version, schema_id)?;

    js.publish_with_headers(subject.as_str().into_owned(), headers, payload.into())
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, schemars::JsonSchema)]
    struct TestEvent {
        id: u32,
        name: String,
    }

    #[test]
    fn test_validate_against_schema_valid() {
        let schema = schema_for!(TestEvent);
        let data = serde_json::json!({"id": 1, "name": "test"});
        assert!(validate_against_schema(&data, Some(&schema)).is_ok());
    }

    #[test]
    fn test_validate_against_schema_invalid() {
        let schema = schema_for!(TestEvent);
        // `id` must be a number, not a string
        let data = serde_json::json!({"id": "not-a-number", "name": "test"});
        assert!(validate_against_schema(&data, Some(&schema)).is_err());
    }

    #[test]
    fn test_validate_against_schema_none_is_permissive() {
        let data = serde_json::json!({"anything": true});
        assert!(validate_against_schema(&data, None).is_ok());
    }

    #[test]
    fn test_build_cloudevent_and_headers_includes_schema_headers() {
        let event_type = EventType::Workflow(crate::events::WorkflowEventType::Queued);
        let source = EventSource::System;
        let data = serde_json::json!({"test": true});
        let schema_version = Some(SchemaVersion::new("1.0".to_string()));
        let schema_id = Some(SchemaId::new("test-schema-id".to_string()));

        let (payload, headers) =
            build_cloudevent_and_headers(event_type, source, data, schema_version, schema_id)
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
