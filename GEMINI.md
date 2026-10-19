# Project-specific Instructions

## Domain name

Any time a domain name which is not `example.com` needs to be used in tests etc use `paninfracon.net`

## Architectural Principles

### 1. Module Responsibility

The project is divided into specialized crates to ensure a clear separation of concerns. Each crate's internal modules have specific roles. Code more than one service needs lives in `stormchaser-core`: the API, the engine and the query service depend on it and never on each other. Keep it that way: when one service needs another's code, move that code into `stormchaser-core`.

- **`stormchaser-model`**: Shared domain types, plus the infrastructure code every service needs. The aim is a pure types crate; it is not one yet. Server-only I/O (database TLS, log backends, the SSRF guard, the policy client) has moved out to `stormchaser-core` and `stormchaser-opa`; the NATS helpers and runner identity code that runners share are still here, outside `wasm32`. Keep new I/O and execution logic out of it; put shared types and small shared helpers here.
  - `workflow.rs` / `step.rs`: Run and step instance models and their status enums.
  - `dsl/`: Types that map from `*.storm` HCL definitions (`workflow.rs`, `step.rs`, `specs.rs` for built-in step specs, `connections.rs`, `limits.rs` for parser limits).
  - `auth.rs`: OPA input contexts and identity claims.
  - `connections.rs`: Connection, storage backend and artifact registry models.
  - `event.rs` / `events.rs` / `event_rules.rs`: CloudEvents envelopes, NATS event payloads and event rules.
  - `nats.rs`: NATS connection and JetStream setup shared by the services.
  - `outbox.rs`: Outbox models for publishing events in the same transaction as the state change.
  - `hcl_eval.rs` / `hcl_guard.rs`: Shared HCL expression helpers, and the nesting limits that keep workflow HCL from overflowing the stack.
  - `runner.rs` / `runner_auth.rs`: Runner models, and runner identity on NATS messages.
  - `schema_gen.rs` / `schema_cache.rs`: JSON Schema generation for the DSL, and the OCI-backed event schema cache.
  - `id.rs`: Typed ids (`RunId`, `StepInstanceId`, ...).

- **`stormchaser-dsl`**: HCL parser and schema toolset.
  - `parser/`: `StormchaserParser`, mapping HCL blocks to `stormchaser-model` types (`workflow.rs`, `step.rs`, `expr.rs`, `provision.rs`).
  - `libraries.rs`: Applies step libraries to the steps that use them (spec merge, params, retry, timeout, allow_failure).
  - `ast.rs`: Abstract Syntax Tree representation of the DSL.
  - `form.rs`: The JSON Schema of a workflow's inputs, used by run forms and the query service.
  - `hcl_schema/`: Bidirectional conversion between JSON Schema and its HCL form (`hcl_to_json.rs`, `json_to_hcl.rs`).

- **`stormchaser-engine`**: The orchestration layer managing state transitions.
  - `workflow_machine.rs` / `step_machine/`: State machines governing the lifecycle of runs and steps.
  - `handler/`: NATS event handlers. `workflow/` (queued, direct, resolve, start_pending, timeout, abandon, sops), `step/` (scheduling, dispatch, quota, built-in steps under `intrinsic/`, step events), `integrations/` (approval, email, Slack, Teams, Lambda, webhook, Jinja) and `runner.rs`.
  - `router.rs` / `server.rs`: Consume the JetStream stream and route events to handlers; `retry.rs` decides which handler errors are transient and get the event delivered again.
  - `leader.rs` / `workers.rs`: A Postgres advisory lock picks one replica to run the background workers (liveness, timeouts, resolver crash recovery, the outbox relay, task message purge, dispatch recovery).
  - `engine_instance.rs`: Per-process heartbeat, used to fail built-in steps of a dead engine.
  - `hcl_eval.rs` / `templating.rs` / `stdlib.rs`: Runtime expression evaluation, MiniJinja templating and the expression function library.
  - `wasm.rs`: WASM step execution.
  - `secrets.rs`: Vault and SOPS secrets and the Vault path allow-list.
  - `db/`: Database layer for engine state (runs, steps, outbox, quotas, runners, task messages, archive).

- **`stormchaser-api`**: REST interface.
  - `main.rs` / `lib.rs` / `setup.rs`: State and route wiring.
  - `routes/`: Route handlers (workflows and runs, steps, connections, cron, webhooks, event rules, auth, schema, MCP).
  - `hitl/`: Human-in-the-loop approvals and approval-link routes.
  - `rate_limit.rs` / `redact.rs` / `api_docs.rs`: Rate limiting, secret masking in responses, and the OpenAPI document and Swagger UI page.
  - `telemetry.rs`: Structured logging, tracing (OpenTelemetry) and metrics.
  - `db/`: API-specific database queries.

- **`stormchaser-core`**: Code shared by the API, the engine and the query service.
  - `config.rs` / `setup.rs` / `listener.rs`: The HTTP services' settings, their start-up (TLS, database, NATS, policy, log backend, identity provider) and their HTTPS or plain HTTP listener, chosen by `STORMCHASER_LISTEN_TLS`.
  - `service.rs`: The `ServiceState` trait a service's state implements for the shared HTTP code.
  - `auth/` / `access.rs`: Token verification (local HS256 and OIDC via JWKS), the policy middleware, and object ownership and authorization. The API re-exports `access` as `authz`.
  - `authz.rs` / `aws_guard.rs` / `run_limits.rs`: Policy checks on connection use, IAM role and Lambda allow-lists, and operator ceilings on run quotas.
  - `git_cache.rs` / `git_source.rs`: Cloning and caching Git sources, and reading a workflow file through a Git connection.
  - `query.rs` / `jq_sandbox.rs`: Input queries (SQL, REST, AWS Cloud Control, mock), and jq programs run in a separate, limited worker process.
  - `hitl.rs`: Sealing and opening approval links.
  - `db/`: Migrations (`migrations.rs`), the outbox inserts and the lookups more than one service makes.
  - `db_tls.rs` / `net_guard.rs` / `logging/`: Postgres connection options (`STORMCHASER_DB_SSL`, pool size), SSRF protection for outbound HTTP, and the Loki and Elasticsearch log backends.

- **`stormchaser-query`**: Separate service for workflow input schema hydration and dynamic option resolution.
  - `main.rs`: Axum server exposing `/api/v1/schema/hydrate` and `/api/v1/schema/hydrate-direct`. It uses the same configuration, token checks and policy middleware as the API, from `stormchaser-core`.
  - `hydration.rs`: Runs a workflow's input queries (SQL, API, AWS Cloud Control) as the calling user, after a policy check on the connection.

- **`stormchaser-runner-*`**: Environment-specific executors.
  - `stormchaser-runner-docker`: `container_machine/` is a typestate machine for Docker container lifecycles; `handler/` handles tasks, cancellation and orphaned containers; `reaper.rs` cleans up.
  - `stormchaser-runner-k8s`: `job_machine/` is a typestate machine for Kubernetes Job lifecycles; `task/`, `scanner.rs`, `cancel.rs` and `reaper.rs` handle tasks, adoption and cleanup.

- **`stormchaser-agent`**: Runner-side helper that runs inside step containers or as a sidecar.
  - `main.rs`: Subcommands `run` (runs the step's command, then parks storage), `park` (waits for the step container, then uploads storage and artifacts) and `unpark` (downloads and extracts storage with hash verification).
  - `artifacts.rs` / `reports.rs`: Artifact publishing and test report collection.
  - `connections.rs`: Park and unpark (tarball upload, download and extraction).
  - `confine.rs`: Keeps what the agent reads inside the step's storage volumes.

- **`stormchaser-cli`**: Primary user command-line interface (`stormchaser` binary). Split into a library and thin binary to support `docs.rs`.
  - `lib.rs` / `main.rs`: The CLI parser and command router.
  - `commands/`: Running, linting (`lint.rs`), exporting schemas (`schema.rs`), managing runs, webhooks, rules, connections, cron schedules, authentication and profiles.

- **`stormchaser-tui`**: Interactive terminal dashboard (Ratatui).
  - `app/`: Application state, API client, dialogs, key handlers and live watches.
  - `ui/`: Rendering for each tab and dialog.

- **`stormchaser-web`**: Leptos web UI. The `ssr` feature builds the Axum server, which proxies to the API; the `hydrate` feature builds the WASM frontend. Build it with `cargo leptos`.

- **`stormchaser-opa`**: Shared Open Policy Agent integration.
  - `client.rs`: `OpaClient` and the `OpaAuthorizer` trait services ask policy questions through, over HTTP or of a WASM policy.
  - `lib.rs`: Evaluates OPA policies compiled to WASM, chooses the policy a service enforces (`OPA_WASM_PATH`, else `OPA_URL`, else the built-in copy of `deploy/opa/policy.rego` in `policy/stormchaser.wasm`, rebuilt with `scripts/build-opa-policy.sh`), and loads policy data from `OPA_DATA_PATH`.

- **`stormchaser-tls`**: Shared TLS configuration.
  - `reloader.rs`: `rustls` client and server configurations, with certificate hot-reloading and optional client certificates.
  - `listener.rs` (feature `listener`): HTTPS listener for Axum services.
  - `lib.rs`: `parse_switch` for strict `true`/`false` settings.

### 2. TUI Responsiveness

- **Async First**: Use `tokio` for any I/O-heavy tasks (API calls, log and run streams, file browsing).
- **Non-blocking UI**: The main event loop must never wait on long-running processes. Use channels or shared state (`Arc<Mutex<...>>` or `Atomic` types) to update the UI from background tasks.
- **Minimal Redraws**: Optimize `terminal.draw` calls and only update affected layout chunks when possible.

### 3. Error Handling

- Use `anyhow` for application-level error handling in `main.rs` and high-level UI logic.
- Use `thiserror` (if needed) for internal library modules where specific error types are beneficial for callers.

### 4. Coding Standards

- **Low entrophy**: Make changes in small, well defined chunks, avoid changing entire files. Aggressively run the compiler to catch errors early.
- **The 500-Line Rule**: Any file exceeding 500 lines is considered "at capacity." Before adding new logic to such a file, you MUST refactor it into logical sub-modules. (Exception: Grafana dashboard JSON files are exempt from this rule). About 25 source files are over the limit today (for example `runner-docker/src/container_machine/transitions/running.rs`, `engine/src/db/steps/instances.rs`, `api/src/routes/webhook.rs`); split them before adding to them.
- **Wiring vs. Logic**: `main.rs` and `lib.rs` must act strictly as wiring facades (configuration, DI, route definitions). All business logic MUST reside in sub-modules. Some do not yet: `stormchaser-web/src/main.rs` holds its proxy and auth handlers, and the agent's and runners' `main.rs` hold command logic.
- **Domain Isolation**: Every new functional domain (e.g., a new integration, backend type, or API category) MUST start in its own file from inception.
- **Structure First**: When writing new code write method stubs first then incrementally complete the methods.
- **Idiomatic Rust**: Follow `clippy` recommendations. Do not arbitrarily disable clippy checks without confirming with the user.
- **Documentation**: All public modules and non-trivial functions should have doc comments (`///`).
- **Tests**: Every new feature must include unit tests. Use `tempfile` for any file-system-related tests to ensure isolation.
- **Long Methods**: Don't create over long methods when they are not necessary, split them down into more focussed smaller methods.
- **Long conditional branches**: In if, loop, switch, and other control structures factor out long conditional branches (100 lines or more) into functions
- **Duplicate code**: Avoid c+v style duplicate code, factor out similar methods to utility functions
- **Security**: Avoid common security errors such as path traversal, sql injection, string concat etc
- **Re-inventing the Wheel**: Before implementing any functional block ensure that there is not an existing crate for it; if there is prefer the existing crate
- **Standard Dirs**: Use operating system standard directories for e.g. caches
- **Anti-patterns**: Avoid standard anti-patterns like God Classes
- **Avoiding long names**: Where you can, prefer an import statement over a fully qualified rust path; particularly if the long path is repeated in a file
- **Stability**: Double check any changes to existing code to ensure they are both intentional and associated with the current task
- **Commit Msg**: Stage multi-line commit messages in a temporary file to avoid CLI quoting errors
- **Data Integrity**: When there is a state machine associated with a data model, ensure that the data model is only updated via the state machine. Do no update the data model directly bypassing the state machine
- **Data Management Layer**: When using SQL queries always wrap the query in a rust function, do not inline it directly in the invoking function
- **Readability**: All functions, function arguments, and variables must have meaningful human readable names, avoid patterns like `q_XX`
- **Import Specifications**: When referring to a type or function from another module, prefer the shortest form of the path, using imports if necessary
- **Type integrity**: When creating new function signitures use the narrowest type possible, and avoid generic constraints unless absolutely necessary or the function is specifically designed to be generic
- **Recording changes**: Update the changelog with brief summaries of major changes. Before releasing a new version ensure that the changelog and other documentation is complete and current
- **Workflow readability**: In the `.storm` files wrap shell commands in HCL 'here' docs and line split them appropriately to promote readability
- **Function Returns**: Don't use tuple returns from functions unless absolutely necessary, or an error type return, instead use a proper struct with named fields
- **Secret Management**: Never hardcode passwords, even for tests or development environments.
  - **WARNING - AI Test Environments**: NEVER modify or overwrite the `.env` file with static passwords (e.g. `STORMCHASER_DEV_PASSWORD=password`) to "fix" failing tests or coverage runs. This desynchronizes the Docker volumes from the expected credentials, causing cascading authentication failures across PostgreSQL and Dex. Always rely on the project's `./scripts/setup.sh` to generate and manage secure, random credentials.
  - **WARNING - SQLx Offline**: Never inject `return;` into tests when `SQLX_OFFLINE=true` is set. `SQLX_OFFLINE` is an `sqlx` compile-time flag for its `query!` macros (the code uses none today), not a runtime bypass. Bypassing tests causes spurious passes.
- **Strong Typing**: Never assemble data using string concat or low level (e.g. json types) objects, always define a struct and serialize it instead

### 5. UI/UX Guidelines

- **Beginner Friendly**: Labels and descriptions should be clear.
- **Keyboard Centric**: Design for speed. Use intuitive keybindings (e.g., `/` for search, `Tab` for focus switching, `Enter` to select).
- **Visual Feedback**: Provide clear status indicators for background tasks (e.g., "Loading runs...", "Streaming logs...").

### 6. Quality

- **Tests**: Run all tests before checkin.
- **Coverage**:
  - Before checkin, run tests in coverage mode and verify there is no significant reduction.
  - **Efficient Reporting**: Use summary-only tools (e.g., `cargo llvm-cov --summary-only`) to avoid generating or processing thousands of lines of raw coverage data unless specifically requested.
- **Format**: Run Cargo format *before* attempting to check in or run tests.
- **Improvement**: When assessing code quality for improvements do not attempt to implment new features.
- **Honoring tests**: Never shortcut tests to just return when fixing test failures
- **Work Ethic**: Don't be lazy and use shortcut solutions when there is a comprehensive fix possible
- **DEad Code**: Don't use `#[allow(dead_code)]` to suppress clippy warnings

## Agent Performance & Context Efficiency

### 1. Context Optimization

- **Minimal Output**: Prefer tools that provide concise summaries (e.g., `cargo llvm-cov --summary-only`, `grep -c`). Do not ingest thousands of lines of tool output unless deep analysis of a specific failure is required.
- **Strategic Searching**: Use `grep_search` with conservative limits (`total_max_matches`) and specific file patterns (`include`) to find code locations quickly.
- **Parallelism**: Execute multiple independent tool calls (e.g., several `read_file` or `grep_search` calls) in a single turn whenever possible.

### 2. Testing & Snapshots

- **Snapshot Updates**: When intentional UI changes occur, use `INSTA_UPDATE=always ./scripts/test-unit.sh` to update snapshots. If you rename a test function, `insta` will generate a new untracked `.snap` file; remember to `git rm` the old snapshot and `git add` the new one.
- **Test Isolation**: Ensure tests do not write to real user configuration or cache directories. Always use `tempfile` or override path fields in the `App` instance during testing.
- **UI Regression Suspicion**: Be highly suspicious if a code change that should be independent of the UI (e.g., changes in `model`, `parsing`, or `discovery`) causes a UI test or snapshot failure. Investigate whether the change inadvertently altered data structures or logic that the UI relies on before blindly updating snapshots.
- **Test stability**: Do NOT fix test errors by removing parts of the tests as that removes critical validation of the project

### 3. Technical Integrity

- **Empirical Verification**: Before applying a fix, always reproduce the bug with a new test case.
- **Idiomatic Updates**: Ensure all changes (including tests, documentation, and types) are complete and follow local conventions. Do not take shortcuts to minimize tool calls.

## 4. Tech Stack Decisons

- **TUI Framework**: `ratatui` (latest stable).
- **Web UI**: `leptos` with `axum` for server-side rendering.
- **HTTP services**: `axum`; Postgres through `sqlx`; NATS JetStream through `async-nats`.
- **Backend**: `crossterm`.
- **Serialization**: `serde` + `serde_json` for caching.
- **Graph Logic**: `petgraph` (chosen, but not used by any crate yet).
- **Text Editing**: `ratatui-textarea`.

## 5. Rendering Mermaid Diagrams

To render Mermaid diagrams using the `mermaid-cli` Docker image, you must pass the current user and group IDs to avoid permission issues (`EACCES`).

Use the following command pattern:

```bash
docker run --rm \
  -v $(pwd):/data \
  --user $(id -u):$(id -g) \
  minlag/mermaid-cli \
  -i /data/your_diagram.mmd \
  -o /data/your_diagram.png
```

## 6. Git Commit Messages

For non-trivial commit messages (those containing backticks, multiple lines, or complex characters), **always use a temporary file** instead of passing the message directly via `-m`. This avoids shell interpolation and escaping issues.

Never commit to `trunk` branch without explicit authorization

Remove any temporary refactoring or codebase exploration scripts before committing

```bash
git commit -F commit_msg.txt
```

## 7. Privilage escalation

When you need to use sudo for local privilage escalation, e.g. when cleaning up environments, use pkexec instead to get the desktop integration

## 8. High Quality Unit Tests

A high-quality unit test in this project must adhere to the following principles:

- **Isolation**: Tests must not depend on external systems (e.g., live databases, network services). Use mocks, stubs, or in-memory equivalents. For filesystem interactions, use the `tempfile` crate to ensure isolation and automatic cleanup.
- **Determinism**: Tests must consistently pass or fail. Avoid dependencies on system time, random number generators without fixed seeds, or external state.
- **Clear Structure**: Follow the Arrange-Act-Assert (or Given-When-Then) pattern to clearly separate test setup, execution, and validation.
- **Descriptive Naming**: Test names should clearly communicate what is being tested and under what conditions (e.g., `test_parse_schema_rejects_invalid_json`).
- **Single Responsibility**: Each test should verify a single logical behavior or edge case. If a test fails, it should be immediately obvious which behavior is broken.
- **Meaningful Assertions**: Assert against specific error types or structural states, not just generic `is_err()` or `is_ok()`, to ensure the failure mode is exactly as expected.
- **Fast Execution**: Unit tests should be extremely fast. Avoid operations that introduce artificial delays.

## 9. High Quality Integration Tests

A high-quality integration test in this project must adhere to the following principles:

- **Realistic Environments**: Test against real instances of dependencies (e.g., PostgreSQL, NATS, OPA) using Docker or similar containerization, rather than relying on mocks, to accurately reflect production behavior.
- **State Management**: Ensure each test starts with a known, clean state. Use isolated database schemas, unique namespaces, or robust teardown routines to prevent test pollution. Do not share mutable state across tests.
- **End-to-End Scenarios**: Focus on critical user journeys or system flows that span multiple components or services. Verify the wiring and interaction between modules.
- **Resilience to Timing**: When dealing with asynchronous systems (like message queues or eventual consistency), avoid fixed sleep durations (`thread::sleep`). Use robust polling or wait mechanisms with timeouts.
- **Observability in Failure**: Ensure that when an integration test fails, it captures sufficient context (logs, container output) to aid in debugging without needing to rerun the test manually.
- **Data Integrity**: Validate side-effects and persistent state changes, not just API responses. Ensure the system reaches the correct terminal state.
