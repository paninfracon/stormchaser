# stormchaser-api

The Stormchaser REST API (`stormchaser-api` binary), built on Axum. It:

- accepts runs (from a repository, from workflow text, from webhooks, event rules and cron triggers) and queues their starting events through the outbox in the run's own transaction;
- serves runs, steps, logs, artifacts and test reports, and manages connections, webhooks, event rules and cron schedules;
- authenticates callers with OIDC (through JWKS) or its own HS256 tokens, and asks OPA about every request and every object, enforcing a built-in copy of the shipped policy when no OPA server or WASM policy is configured;
- handles approvals, including sealed, expiring approval links;
- serves HTTPS by default (`STORMCHASER_LISTEN_TLS=false` turns it off), an OpenAPI document and Swagger UI page only with `STORMCHASER_API_DOCS=true`, and an MCP server (Cargo feature `mcp`, on by default).

The `stormchaser-query` service reuses its configuration, authentication and policy middleware.

---

## About Stormchaser

Stormchaser is a distributed workflow engine for event-driven and human-triggered workflows, written in Rust. Workflows are written in an HCL-based DSL; the engine keeps state in PostgreSQL and passes events over NATS JetStream.

For more information, full documentation, and examples, please visit the [Stormchaser GitHub Repository](https://github.com/paninfracon/stormchaser) and read the [Main Project README](https://github.com/paninfracon/stormchaser/blob/trunk/README.md).
