# Stormchaser Project Status & Future Work

*Status as of 2026-10-19, version 1.5.1 plus the changes under `[Unreleased]` in `CHANGELOG.md`.*

## 1. Summary

The core works end to end: workflows are parsed from the HCL-based DSL, runs are resolved from Git or from workflow text, and steps run on the Docker and Kubernetes runners or inside the engine. Approvals, webhooks, event rules, cron triggers, the CLI, TUI and web UI all work against the same API.

Since 1.5.1 the work has been on reliability, security and correctness rather than new features:

* **Reliability.** Events and step results go through a transactional outbox and are deduplicated by the stream. Events whose handler fails for a transient reason are delivered again with backoff. Redelivered events no longer run a run twice. Lost dispatches are dispatched again. One engine replica, chosen through a Postgres advisory lock, runs the background workers; each engine records a heartbeat, and the built-in steps of a dead engine are failed (or, for `JQ` and `JinjaRender`, run again).
* **Security.** Services serve HTTPS and connect to PostgreSQL with `verify-full` by default. There is always an authorization policy: with no OPA server, the services enforce a built-in WASM copy of the shipped policy, which refuses what it does not recognise and checks ownership of every run, connection, webhook, rule and schedule. Allow-lists cover `env()`, Vault paths, IAM roles, internal hosts and privileged steps. NATS requires credentials, with separate users per service. Approval links are sealed with their own key and expire. Steps get resource limits by default.
* **Features that were documented but did not work now do.** Step `retry` blocks, `allow_failure`, step library parameters, and spec merging between a step and its library.
* **Dependencies.** All dependencies upgraded (Axum 0.8, Leptos 0.8, SQLx 0.9, kube 4, MiniJinja 3, jaq 3). The minimum Rust version is 1.96.

## 2. Implementation Overview

| Category | Status | Details |
| :--- | :--- | :--- |
| **Language** | Implemented | Rust workspace of 13 crates. |
| **Data Store** | Implemented | PostgreSQL for state, history and the outbox; NATS JetStream for events. |
| **DSL** | Implemented | HCL-based DSL parsed with `hcl-rs`; HCL and MiniJinja expressions; step libraries with parameters; includes; offline linting against a JSON Schema. |
| **Execution** | Implemented | Event-driven state machines for runs and steps. Several engine replicas with shard assignment and a leader for background workers. Per-run quotas under operator ceilings, per-step resource limits, run and step timeouts, step retries. |
| **Auth** | Implemented | OIDC login (Dex for the quickstart) and API-signed tokens; OPA policy on every request and object, with a built-in policy as the fallback. |
| **File System** | Implemented | Shared File System (SFS) parked as tarballs in S3-compatible storage with SHA-256 checks; artifacts to S3 or OCI registries; PVC and host-path optimisations. |
| **Steps** | Implemented | `RunContainer`, `RunK8sJob`, `GitCheckout`, `RestApi`, `SqlExecute`, `JQ`, `JinjaRender`, `WebhookInvoke`, `Wasm`, `LambdaInvoke`, `EmailSend` (SMTP or SES), `TestReportEmail`, `SlackMessage`, `TeamsMessage`, Terraform plan/apply/approval, `Approval`, `Wait`. The `Parallel` and `Sequential` group types are planned. |
| **Interfaces** | Implemented | CLI, TUI, Leptos web UI, REST API with OpenAPI, MCP server. |
| **Deployment** | Implemented | docker-compose quickstart; Helm charts (umbrella, orchestration, both runners, telemetry). |

## 3. Known Gaps

These are documented or modelled but do not work yet:

* **`on_failure` and `finally` blocks** are parsed but the engine does not run them.
* **Storage I/O limits** (`max_throughput`, `max_iops` in `docs/workflow_dsl.md`) are not enforced.
* **`locked` affinity**, and reuse of a warm container for `shared` affinity. A `shared` group pins its steps to one runner; each step still gets its own container.
* **`is_retry()` and `retry_count()`** expression functions.
* **AWS Secrets Manager** and other secret backends besides SOPS and Vault.
* **Approving from Slack or Teams.** The message steps send messages only.

## 4. Planned Features

### Core Workflow & Engine Capabilities

* **Step Optimization:** Run multiple small steps (e.g. Python scripts) within a single container to reduce overhead.
* **Step Memoization/Caching:** Skip execution if inputs and code have not changed.
* **Global and per-workflow concurrency limits**, beyond the per-run quotas.

### Advanced Step Types & Integrations

* **ChatOps approvals:** Approvals and status updates directly in Slack or Microsoft Teams.
* **Continuous Verification:** Steps with health metric monitoring and automatic rollbacks.
* **Sensors (Polling):** Background polling of external systems (Jira, GitHub, databases) that emits events.
* **Environment and Service abstractions:** For cross-context workflow reuse.

### Execution Runner Enhancements

* **Native WASM on Kubernetes:** Integrate [kwasm](https://kwasm.sh/) for executing Wasm steps natively within the Kubernetes runner. Today `Wasm` steps run inside the engine.

### Reliability & Developer Experience

* **Built-in Mermaid Rendering:** Diagrams are rendered by hand with the `mermaid-cli` container; rendering workflows in the application is planned.
* **Dynamic Schema Resolution via OCI Event Streams:** Consume event streams from the OCI schema registry to update the schema cache at once, instead of waiting for the background sync interval.

## 5. Blocked / Pending Upstream

* **TUI HCL Editor:** Syntax-highlighted editing of `.storm` files in the TUI with `ratatui-code-editor` is **blocked**. That crate hardcodes its supported languages and has no public API to inject a custom `tree_sitter::Language` (such as `tree_sitter_hcl::language()`).

## 6. Maintenance Tasks

* **Planned Features Tracking:** As features move from planned to implemented, update this document, [Features](features.md) and the DSL documentation together.
* **The 500-line rule:** About 25 source files are over the limit set in `GEMINI.md`; split them before adding to them.
* **Unused dependencies:** The engine's `rustls-native-certs` feature pulls in a crate no code uses.
* **Upstream Blocks:** Watch `ratatui-code-editor` for tree-sitter language injection.
