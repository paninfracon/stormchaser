# Project-specific Instructions

## Domain name

Any time a domain name which is not `example.com` needs to be used in tests etc use `paninfracon.net`

## Architectural Principles

### 1. Module Responsibility

The project is divided into specialized crates to ensure a clear separation of concerns. Each crate's internal modules have specific roles:

- **`stormchaser-model`**: Foundational domain types. Must remain pure and free of execution logic.
  - `workflow.rs` / `step.rs`: Core execution state models and state machine data.
  - `dsl.rs`: Specification types that map directly from `*.storm` HCL definitions.
  - `auth.rs`: OPA contexts, identity claims, and authorization types.
  - `storage.rs` / `artifact.rs`: Models for storage backends and artifact metadata.
  - `event.rs` / `event_rules.rs`: Definitions for system events and rule-based triggers.
  - `schema_gen.rs`: Centralized generation logic for building Extensible JSON Schemas representing the DSL.

- **`stormchaser-dsl`**: Specialized HCL parser and schema toolset.
  - `lib.rs`: `StormchaserParser` implementation for mapping HCL blocks to `stormchaser-model` types.
  - `ast.rs`: Abstract Syntax Tree representation of the DSL.
  - `hcl_schema.rs`: Bidirectional serialization logic for mapping JSON Schema to and from the functional HCL data format.

- **`stormchaser-engine`**: The core orchestration layer managing state transitions.
  - `workflow_machine.rs` / `step_machine.rs`: State machines governing the lifecycle of workflows and individual steps.
  - `hcl_eval.rs`: Runtime expression evaluation and variable interpolation.
  - `git_cache.rs`: Handles cloning, caching, and resolution of Git-based workflow sources.
  - `wasm.rs`: Logic for executing WASM-based steps.
  - `handler.rs`: NATS message handling and event dispatching.
  - `db.rs`: Database interaction layer for persisting engine state.

- **`stormchaser-api`**: REST interface for external interaction.
  - `main.rs`: Axum server configuration and route definitions.
  - `auth/`: OPA-backed authorization logic and middleware.
  - `hitl.rs`: Management of Human-In-The-Loop approval flows.
  - `telemetry.rs`: Structured logging, tracing (OpenTelemetry), and metrics.
  - `db.rs`: API-specific database queries.
  - `routes/schema.rs`: Serves the live DSL JSON schema for offline validation support.

- **`stormchaser-runner-*`**: Environment-specific executors.
  - `stormchaser-runner-docker`: Uses `container_machine.rs` to manage Docker container lifecycles.
  - `stormchaser-runner-k8s`: Uses `job_machine.rs` to manage Kubernetes Job lifecycles.

- **`stormchaser-agent`**: Runner-side execution wrapper.
  - Executes user commands within the runner environment.
  - `main.rs`: Implements storage "parking" (packaging/uploading) and "unparking" (downloading/extracting) for SFS (Stormchaser File System).
  - Collects and uploads artifacts and test reports.

- **`stormchaser-cli`**: Primary user command-line interface. Split into a library and thin binary to support `docs.rs`.
  - `lib.rs` / `main.rs`: Exposes the CLI parser and command router.
  - `commands/`: Implements commands for running, linting (`lint.rs`), exporting schemas (`schema.rs`), managing runs, webhooks, rules, and authentication.

- **`stormchaser-tui`**: Interactive terminal dashboard.
  - `main.rs`: Ratatui-based UI for real-time monitoring of workflow state and logs.

- **`stormchaser-opa`**: Shared Open Policy Agent integration.
  - `lib.rs`: Client and data structures for policy evaluation.

- **`stormchaser-tls`**: Shared security configuration.
  - `lib.rs`: Standardized mTLS and TLS setup for secure inter-service communication.

### 2. TUI Responsiveness

- **Async First**: Use `tokio` for any I/O-heavy tasks (PATH scanning, man page parsing, command execution).
- **Non-blocking UI**: The main event loop must never wait on long-running processes. Use channels or shared state (`Arc<Mutex<...>>` or `Atomic` types) to update the UI from background tasks.
- **Minimal Redraws**: Optimize `terminal.draw` calls and only update affected layout chunks when possible.

### 3. Error Handling

- Use `anyhow` for application-level error handling in `main.rs` and high-level UI logic.
- Use `thiserror` (if needed) for internal library modules where specific error types are beneficial for callers.

### 4. Coding Standards

- **Low entrophy**: Make changes in small, well defined chunks, avoid changing entire files. Aggressively run the compiler to catch errors early.
- **The 500-Line Rule**: Any file exceeding 500 lines is considered "at capacity." Before adding new logic to such a file, you MUST refactor it into logical sub-modules.
- **Wiring vs. Logic**: `main.rs` and `lib.rs` must act strictly as wiring facades (configuration, DI, route definitions). All business logic MUST reside in sub-modules.
- **Domain Isolation**: Every new functional domain (e.g., a new integration, backend type, or API category) MUST start in its own file from inception.
- **Structure First**: When writing new code write method stubs first then incrementally complete the methods.
- **Idiomatic Rust**: Follow `clippy` recommendations. Do not arbitrarily disable clippy checks without confirming with the user.
- **Documentation**: All public modules and non-trivial functions should have doc comments (`///`).
- **Tests**: Every new feature must include unit tests. Use `tempfile` for any file-system-related tests to ensure isolation.
- **Long Methods**: Don't create over long methods when they are not necessary, split them down into more focussed smaller methods.
- **Long conditional branches**: In if, loop, switch, and other control structures factor out long conditional branches into method calls
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
  - **WARNING - SQL Offline**: Never inject `return;` into tests when `SQL_OFFLINE=true` is set. `SQL_OFFLINE` is an `sqlx` compiler flag, not a runtime bypass. Bypassing tests causes spurious passes.
- **Strong Typing**: Never assemble data using string concat or low level (e.g. json types) objects, always define a struct and serialize it instead

### 5. UI/UX Guidelines

- **Beginner Friendly**: Labels and descriptions should be clear.
- **Keyboard Centric**: Design for speed. Use intuitive keybindings (e.g., `/` for search, `Tab` for focus switching, `Enter` to select).
- **Visual Feedback**: Provide clear status indicators for background tasks (e.g., "Scanning PATH...", "Parsing Man page...").

### 6. Quality

- **Tests**: Run all tests before checkin.
- **Coverage**:
  - Before checkin, run tests in coverage mode and verify there is no significant reduction.
  - **Efficient Reporting**: Use summary-only tools (e.g., `cargo llvm-cov --summary-only`) to avoid generating or processing thousands of lines of raw coverage data unless specifically requested.
- **Format**: Run Cargo format *before* attempting to check in or run tests.
- **Improvement**: When assessing code quality for improvements do not attempt to implment new features.
- **Honoring tests**: Never shortcut tests to just return when fixing test failures
- **Work Ethic**: Don't be lazy and use shortcut solutions when there is a comprehensive fix possible

## Agent Performance & Context Efficiency

### 1. Context Optimization

- **Minimal Output**: Prefer tools that provide concise summaries (e.g., `cargo llvm-cov --summary-only`, `grep -c`). Do not ingest thousands of lines of tool output unless deep analysis of a specific failure is required.
- **Strategic Searching**: Use `grep_search` with conservative limits (`total_max_matches`) and specific file patterns (`include`) to find code locations quickly.
- **Parallelism**: Execute multiple independent tool calls (e.g., several `read_file` or `grep_search` calls) in a single turn whenever possible.

### 2. Testing & Snapshots

- **Snapshot Updates**: When intentional UI changes occur, use `INSTA_UPDATE=always cargo test` to update snapshots.
- **Test Isolation**: Ensure tests do not write to real user configuration or cache directories. Always use `tempfile` or override path fields in the `App` instance during testing.
- **UI Regression Suspicion**: Be highly suspicious if a code change that should be independent of the UI (e.g., changes in `model`, `parsing`, or `discovery`) causes a UI test or snapshot failure. Investigate whether the change inadvertently altered data structures or logic that the UI relies on before blindly updating snapshots.
- **Test stability**: Do NOT fix test errors by removing parts of the tests as that removes critical validation of the project

### 3. Technical Integrity

- **Empirical Verification**: Before applying a fix, always reproduce the bug with a new test case.
- **Idiomatic Updates**: Ensure all changes (including tests, documentation, and types) are complete and follow local conventions. Do not take shortcuts to minimize tool calls.

## 4. Tech Stack Decisons

- **TUI Framework**: `ratatui` (latest stable).
- **Backend**: `crossterm`.
- **Serialization**: `serde` + `serde_json` for caching.
- **Graph Logic**: `petgraph`.
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
Never commit to `trunk` branch without explict autorization

```bash
git commit -F commit_msg.txt
```

## 7. Privilage escalation

When you need to use sudo for local privilage escalation, e.g. when cleaning up environments, use pkexec instead to get the desktop integration
