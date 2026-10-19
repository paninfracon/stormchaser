# Stormchaser Project Considerations

This document records ideas and open questions raised while reviewing the architecture and DSL documentation, and whether each has been acted on. For what is planned, see [Project Status](project_status.md).

## 1. Architectural & Operational Risks

### Duplicate webhook deliveries

**Done.** GitHub and other senders deliver a webhook again when they time out or retry. The API records each accepted delivery in PostgreSQL and acknowledges a repeat without acting on it again (`STORMCHASER_WEBHOOK_DEDUPE_HOURS` sets how long a delivery is remembered). For GitHub the key is a hash of the signed body, since the `X-GitHub-Delivery` header is not covered by the signature; for `signed` (Standard Webhooks) sources it is the `webhook-id`. Separately, the engine's outbox relay sets `Nats-Msg-Id` on every event it publishes, so JetStream drops a second copy within its 2-minute window.

### Diagnosability

**Open.** A `trace <run id>` command in the CLI could pull a run's state transition history from PostgreSQL and its logs from Loki through the API, and render a timeline in the terminal, showing the HCL that failed and the container's exit code. Today `stormchaser runs get`, `runs watch` and `runs logs` show these separately, and a failed run records its reason. Correlation ids (`run_id`, `step_id`) should be on all OpenTelemetry log messages too.

### OPA bootstrap

**Done.** The services evaluate OPA policies compiled to WASM in process (`stormchaser-opa`, Wasmtime). With no `OPA_URL` or `OPA_WASM_PATH`, they enforce a built-in copy of the shipped policy (`deploy/opa/policy.rego`), with settings from `OPA_DATA_PATH`. An external OPA server (`OPA_URL`) or a custom WASM policy (`OPA_WASM_PATH`) can be used instead. See [OPA RBAC](opa-rbac.md).

## 2. DSL & Feature Omissions

* `on_failure` and `finally` blocks are documented in [Workflow DSL](workflow_dsl.md) and parsed, but the engine does not run them.
* Storage `limits` (`max_throughput`, `max_iops`) are documented but not enforced.

## 3. Documentation Errors & Inconsistencies

### Examples

**Open.** The documentation could be embedded in the CLI: `stormchaser explain step <type>` would print the step's spec schema and a working example. Today `stormchaser schema` exports the whole DSL schema, and the step pages are in [`docs/steps/`](steps/).

## 4. Security Risks

See [Security](security.md) for the security controls in place and what deployments must provide.
