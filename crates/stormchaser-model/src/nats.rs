use crate::id::EventId;
use anyhow::Result;
use async_nats::jetstream;
use async_nats::HeaderMap;
use cloudevents::{AttributesReader, EventBuilder, EventBuilderV10};
use schemars::schema::RootSchema;
use serde_json::Value;
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

/// Helper function to wrap a JSON payload in a CloudEvent and publish it to NATS with schema headers.
pub async fn publish_cloudevent(
    js: &jetstream::Context,
    subject: &str,
    event_type: &str,
    source: &str,
    data: Value,
    schema_version: Option<&str>,
    schema_id: Option<&str>,
) -> Result<()> {
    let event = EventBuilderV10::new()
        .id(EventId::new_v4().into_inner().to_string())
        .ty(event_type)
        .source(source)
        .time(chrono::Utc::now())
        .data("application/json", data)
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build CloudEvent: {}", e))?;

    let payload = serde_json::to_string(&event)?;

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/cloudevents+json");
    if let Some(v) = schema_version {
        headers.insert("Nats-Msg-Schema-Version", v);
    }
    if let Some(id) = schema_id {
        headers.insert("Schema-ID", id);
    }

    js.publish_with_headers(subject.to_string(), headers, payload.into())
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
}
