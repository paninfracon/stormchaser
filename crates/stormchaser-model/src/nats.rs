use anyhow::Result;
use async_nats::jetstream;
use async_nats::HeaderMap;
use cloudevents::{EventBuilder, EventBuilderV10};
use serde_json::Value;

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
        .id(uuid::Uuid::new_v4().to_string())
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
