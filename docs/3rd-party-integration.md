# 3rd Party Event Integration

Stormchaser's event bus is designed to be a robust, language-agnostic integration layer. This document outlines how 3rd party applications can subscribe to, validate, and publish events interacting with the Stormchaser engine.

## Architecture Overview

Stormchaser utilizes **NATS JetStream** as its central nervous system. Every state transition in the engine (e.g., a workflow starting, a step failing) emits an event.

To ensure stability and interoperability, Stormchaser's eventing system adheres to two major standards:

1. **CloudEvents**: All messages are wrapped in a standard [CloudEvents](https://cloudevents.io/) envelope.
2. **JSON Schema**: The internal `data` payload of every CloudEvent is strictly validated against a versioned JSON Schema.

## Connecting and Consuming Events

### Subject-Based Routing

Stormchaser uses subject-based routing with version tokens. This allows you to filter exactly what events your application receives without parsing the payloads.

**Pattern:** `stormchaser.<version>.<domain>.<action>`
**Example:** `stormchaser.v1.run.completed`

You can use NATS wildcards to listen to broader categories:

* `stormchaser.v1.run.>` : Listen to all workflow run events (queued, running, completed, failed, aborted).
* `stormchaser.v1.step.>` : Listen to all step events.

### The Message Payload

When your application receives a message, it will be a JSON-serialized CloudEvent.

```json
{
  "specversion": "1.0",
  "id": "a1b2c3d4-e5f6-7a8b-9c0d-1e2f3a4b5c6d",
  "source": "/stormchaser",
  "type": "stormchaser.v1.run.completed",
  "time": "2026-05-04T12:00:00Z",
  "datacontenttype": "application/json",
  "data": {
    "run_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
    "event_type": "workflow_completed",
    "timestamp": "2026-05-04T12:00:00Z"
  }
}
```

In addition to the CloudEvent payload, Stormchaser attaches metadata to the **NATS Message Headers**:

* `Content-Type`: `application/cloudevents+json`
* `Nats-Msg-Schema-Version`: The version of the schema (e.g., `1.0`).

## Implementation Strategies

### 1. For Rust Applications (Native)

If your 3rd party application is written in Rust, the easiest approach is to add the `stormchaser-model` crate as a dependency.

This provides immediate access to:

* **Strongly Typed Events:** `stormchaser_model::events::WorkflowCompletedEvent`, `StepFailedEvent`, etc.
* **CloudEvents Helper:** `stormchaser_model::nats::publish_cloudevent` to safely push events back into the bus.

**Example Consumer:**

```rust
use cloudevents::Event;
use stormchaser_model::events::WorkflowCompletedEvent;
use futures::StreamExt;

// Assuming `subscriber` is an async_nats::Subscriber listening to "stormchaser.v1.run.completed"
while let Some(msg) = subscriber.next().await {
    // 1. Unwrap the CloudEvent envelope
    let ce: Event = serde_json::from_slice(&msg.payload)?;

    // 2. Extract and deserialize the strong type
    if let Some(cloudevents::Data::Json(v)) = ce.data() {
        let event: WorkflowCompletedEvent = serde_json::from_value(v.clone())?;
        println!("Workflow {} completed!", event.run_id);
    }
}
```

### 2. For Non-Rust Applications (Python, Go, Node.js, Java)

Stormchaser's reliance on industry standards makes integrating non-Rust applications completely seamless.

#### Step 1: Use a CloudEvents SDK

Instead of manually parsing the JSON and extracting headers, use one of the official [CloudEvents SDKs](https://github.com/cloudevents/sdk-go) (available for Go, Python, Java, C#, Node.js, etc.). These SDKs handle the envelope validation, header extraction, and payload unwrapping for you.

#### Step 2: Generate Native Models from JSON Schema

Stormchaser exports the exact shape of its events as standard JSON Schema Draft 7 documents. You can find these in the `schemas/` directory of the Stormchaser repository (or fetch them from your configured OCI Schema Registry).

You can use tools like [Quicktype](https://quicktype.io/) to automatically generate native code for your application:

* **TypeScript:** Generate native Interfaces and Type Guards.
* **Python:** Generate Pydantic models for automatic validation.
* **Go:** Generate native `struct` definitions with correct JSON tags.
* **Java/C#:** Generate heavily typed class models.

By generating your data models directly from the JSON Schemas, you ensure your application is always perfectly aligned with the Stormchaser engine's data structures, preventing runtime serialization errors.

## Future Enhancements

Stormchaser plans to support dynamic schema resolution via OCI Event Streams. In the future, 3rd party applications utilizing the `SchemaCache` mechanism will be able to instantly invalidate and refresh their local schema models via live events from an OCI container repository.
