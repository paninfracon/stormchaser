# stormchaser-engine

The orchestration engine (`stormchaser-engine` binary). It consumes events from NATS JetStream and drives runs and steps through their state machines in PostgreSQL. It:

- resolves runs: fetches and caches Git sources, decrypts SOPS secrets, reads Vault secrets, runs input queries and checks policy;
- schedules steps, enforces run quotas and operator ceilings, dispatches container steps to runners, and retries failed steps as their `retry` block says;
- runs built-in steps itself (`RestApi`, `SqlExecute`, `JQ`, `JinjaRender`, `WebhookInvoke`, `Wasm`, `LambdaInvoke`, email, Slack and Teams messages, Terraform steps) and evaluates HCL expressions and MiniJinja templates;
- publishes its events through an outbox relay, and delivers an event again when its handler fails for a transient reason.

Several replicas can run together. A Postgres advisory lock picks one to run the background workers (liveness, timeouts, resolver crash recovery, the outbox relay, dispatch recovery). Optional integrations (AWS, email, Vault, Slack, Teams) are Cargo features; see `docs/cargo-features.md`.

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
