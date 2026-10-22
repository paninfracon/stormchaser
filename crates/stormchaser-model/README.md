# stormchaser-model

Types and shared code used across the Stormchaser workspace: workflow and step models, the DSL's types, events and CloudEvents envelopes, connections, auth contexts, typed ids and JSON Schema generation.

It is not a pure types crate. Outside `wasm32` it also holds infrastructure the services share: PostgreSQL connection options (TLS, pool size), NATS connection setup, the outbox models, SSRF-guarded HTTP clients, Loki and Elasticsearch log backends, runner identity, and the OCI-backed event schema cache. It depends on `tokio`, `sqlx`, `async-nats` and `reqwest` for these. The `wasm32` build, used by the web UI, leaves them out.

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
