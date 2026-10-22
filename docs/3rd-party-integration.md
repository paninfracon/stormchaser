# 3rd Party Event Integration

Stormchaser publishes its events on NATS JetStream, so other applications can
watch runs and steps without calling the API. This document describes the
subjects, the message format, and how to consume them.

## Architecture Overview

Every event goes to one JetStream stream, `stormchaser`, which captures
`stormchaser.v1.>` (see [NATS](nats.md)). Messages are kept for 168 hours by
default.

Two standards apply:

1. **CloudEvents**: every message is a JSON [CloudEvents](https://cloudevents.io/)
   1.0 envelope.
2. **JSON Schema**: the `data` payload of most events is described by a JSON
   Schema (draft-07), exported in the `schemas/` directory.

Schema validation happens only in the engine, for the events it consumes. It
checks payloads against schemas it generates from its own types at start-up,
not against the `schemas/` files, and only for the subjects it has a schema for.
A payload that does not match is logged and dropped. Events on other subjects
are accepted unchecked.

A subscriber needs a NATS identity of its own. Give it subscribe permission on
the subjects below and on its inbox, and no publish permission beyond what
JetStream consumers need. See [NATS production requirements](nats.md#production-requirements).

## Connecting and Consuming Events

### Subject-Based Routing

**Pattern:** `stormchaser.v1.<shard>.<domain>.<action>`
**Example:** `stormchaser.v1.global.run.queued`

`<shard>` is `global`, or a shard number computed from the run id
(`0` unless `STORMCHASER_TOTAL_SHARDS` is set above 1). Runners publish step
events on the run's shard number; the API and engine mostly use `global`. Match
it with `*`:

* `stormchaser.v1.*.run.>`: run events.
  * `run.queued`: a run was submitted.
  * `run.start_pending`: the engine is preparing to start a run.
  * `run.running`: the run started.
  * `run.completed`: the run finished successfully.
  * `run.failed`: the run failed.
  * `run.aborted`: the run was aborted (for example, its timeout passed).
* `stormchaser.v1.*.step.>`: step events.
  * `step.scheduled.<type>`: the engine dispatched a step to a runner, for
    example `stormchaser.v1.0.step.scheduled.runcontainer`. These messages
    carry the step's resolved spec, including secrets; do not give a third
    party read access to them.
  * `step.initializing`: a runner is preparing the step.
  * `step.unpacking_sfs`, `step.packing_sfs`: the step's shared storage is
    being restored or saved.
  * `step.running`: the step's container started.
  * `step.completed`: the step succeeded.
  * `step.failed`: the step failed.
* `stormchaser.v1.global.runner.>`: runner lifecycle.
  * `runner.heartbeat`, `runner.offline`.

Some subjects have no shard token:

* `stormchaser.v1.run.direct`: a run submitted as workflow text, for the engine
  to parse.
* `stormchaser.v1.runner.register`: a runner registered. Runners publish this
  with a plain NATS publish and no headers, but the stream still stores it.
* `stormchaser.v1.runner.cancel`: the engine asks runners to cancel a step.
* `stormchaser.v1.runner.<kind>.<runner id>`: a task sent to one runner (a step
  with `shared` affinity, see [Execution Affinity](affinity.md)). Like
  `step.scheduled.*`, these carry secrets.

To get everything except task messages, subscribe to `stormchaser.v1.*.run.>`,
`stormchaser.v1.*.runner.>` and the step subjects you need by name.

### The Message Payload

Each message is a JSON-serialized CloudEvent:

```json
{
  "specversion": "1.0",
  "id": "a1b2c3d4-e5f6-4a8b-9c0d-1e2f3a4b5c6d",
  "source": "/stormchaser/api",
  "type": "WorkflowQueuedEvent",
  "time": "2026-05-04T12:00:00Z",
  "datacontenttype": "application/json",
  "data": {
    "run_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
    "event_type": "WorkflowQueuedEvent",
    "timestamp": "2026-05-04T12:00:00Z",
    "status": "queued"
  }
}
```

* `type` and `data.event_type` are the event's type name (`WorkflowQueuedEvent`,
  `StepFailedEvent`, ...), not the subject. The runner registration message is
  the exception: its `type` is `stormchaser.v1.runner.register`.
* `source` is `/stormchaser`, `/stormchaser/api` or `stormchaser-engine`,
  depending on who published the event.
* `id` is a random UUID. Events relayed from the database outbox are also sent
  with a `Nats-Msg-Id` header, so the stream keeps one copy if the relay sends
  one twice.
* Events a runner publishes carry a signature in `data` once the engine has
  given the runner a key.
* Payload fields differ by event. `WorkflowQueuedEvent`, for example, may also
  carry `dsl`, `inputs`, `initiating_user`, `sops_file` and `sops_role_arn`.

The NATS message headers are:

* `Content-Type`: `application/cloudevents+json`
* `ce-schemaid`: the schema version (for example `1.0`), when the publisher set
  one.
* `Nats-Msg-Id`: on events relayed from the outbox.

## Implementation Strategies

### 1. For Rust Applications (Native)

Add the `stormchaser-model` crate as a dependency. It provides:

* **Typed events:** `stormchaser_model::events::WorkflowQueuedEvent`,
  `WorkflowCompletedEvent`, `StepFailedEvent`, and the rest.
* **Subjects:** `stormchaser_model::nats::NatsSubject`, which builds the subject
  strings above.
* **Publishing:** `stormchaser_model::nats::publish_cloudevent`, which wraps a
  payload in a CloudEvent and publishes it on JetStream with the headers above.
  The engine acts on what it receives, so only publish events you mean it to
  act on, and only with a NATS identity allowed to.

**Example consumer:**

```rust
use cloudevents::{AttributesReader, Event};
use stormchaser_model::events::WorkflowQueuedEvent;
use futures::StreamExt;

// `subscriber` is an async_nats::Subscriber on "stormchaser.v1.*.run.queued"
while let Some(msg) = subscriber.next().await {
    // 1. Unwrap the CloudEvent envelope
    let ce: Event = serde_json::from_slice(&msg.payload)?;
    if ce.ty() != "WorkflowQueuedEvent" {
        continue;
    }

    // 2. Deserialize the typed payload
    if let Some(cloudevents::Data::Json(v)) = ce.data() {
        let event: WorkflowQueuedEvent = serde_json::from_value(v.clone())?;
        println!("Workflow {} queued!", event.run_id);
    }
}
```

### 2. For Non-Rust Applications (Python, Go, Node.js, Java)

#### Step 1: Use a CloudEvents SDK

Use one of the [CloudEvents SDKs](https://github.com/cloudevents) (Go, Python,
Java, C#, Node.js and others) to parse the envelope.

#### Step 2: Generate Models from JSON Schema

The `schemas/` directory holds a draft-07 JSON Schema for each event type
(`WorkflowQueuedEvent.json`, `StepCompletedEvent.json`, ...). Tools such as
[Quicktype](https://quicktype.io/) generate TypeScript, Python (Pydantic), Go
and Java/C# models from them.

Not every event has a file there: there is none, for example, for
`WorkflowDirectEvent` or the SFS step events.

## Future Enhancements

Planned: fetching schemas from an OCI registry. `stormchaser_model::schema_cache::SchemaCache`
has a background sync from a registry, but nothing in Stormchaser uses it yet.
