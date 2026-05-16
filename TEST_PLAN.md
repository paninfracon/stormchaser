# Stormchaser Development Plan: Testing & Architecture

This plan outlines the phased approach to implementing the outstanding items identified in the project's `future_todo.md`.

## Phase 1: TUI Application Logic & Event Handling (Unit & Mock Testing)

**Goal:** Establish 100% test coverage for the core TUI event loop and API interaction layers without relying on a live backend.

1. **Refactor for Testability (if necessary):** Ensure the `App` struct in `stormchaser-tui` can be instantiated cleanly in a test context (e.g., injecting mock configuration or skipping immediate network calls on startup).
2. **API Mocking Setup:** Introduce `wiremock` (or a similar lightweight HTTP mocking library) as a `dev-dependency` in `crates/stormchaser-tui/Cargo.toml`.
3. **Test API Modules (`app/api/*.rs`):**
    * Create mock server instances in tests.
    * Test successful responses, error handling, and deserialization for modules like `auth.rs`, `runs.rs`, `storage_backends.rs`, and `webhooks.rs`.
4. **Test State Ingestion (`app/handlers.rs`):**
    * Write isolated unit tests for `App::handle_status_update` and `handle_full_run_update`.
    * Provide mock NATS/SSE payloads and assert that the internal `App` state (e.g., the `RunGraph` or lists of runs) mutates correctly.
5. **Test Key Event Transitions (`app/handlers.rs`):**
    * Simulate keypress events using `crossterm::event::Event`.
    * Verify that pressing specific keys in specific dialog states transitions the UI correctly (e.g., opening a filter dialog, submitting a form).

## Phase 2: TUI Rendering with Insta (Snapshot Testing)

**Goal:** Prevent visual regressions in the legacy UI components by cementing their expected output via `insta` snapshot tests.

1. **Preparation:** Leverage the existing `insta` setup used in the newer `storage.rs` and `webhooks.rs` UI components. Ensure `INSTA_UPDATE=always ./scripts/test-unit.sh` workflow is understood.
2. **Snapshot `ui/runs.rs`:**
    * Create test fixtures for `App` state representing: an empty run list, a populated run list, a run with a failure, and a run in progress.
    * Write tests that call `render_runs_tab` or `render_run_detail` with these states and snapshot the resulting `ratatui::buffer::Buffer`.
3. **Snapshot `ui/dialogs.rs`:**
    * Setup `App` state with various dialogs active (e.g., `App::show_filter_dialog = true`).
    * Snapshot the rendering of the filter dialog, file browser, and schedule git dialog.

## Phase 3: CLI Interactive Command Tests (Integration Testing)

**Goal:** Verify that the CLI correctly handles long-lived, streaming connections for logs and watch commands.

1. **Test Harness Setup:** Create a lightweight integration test harness in `crates/stormchaser-cli/tests/` (if one doesn't exist) or within the `src/commands/runs/` modules using `#[cfg(test)]`.
2. **Mocking the Stream:**
    * Set up a local mock server (via `wiremock` or a minimal `axum` test server) that emits Server-Sent Events (SSE).
    * Configure the CLI in the test to point to this mock server.
3. **Test `runs logs` (`logs.rs`):**
    * Execute the `logs` command programmatically.
    * Emit mock log events from the server.
    * Capture the stdout of the CLI and assert that the log lines are formatted and printed correctly.
    * Test reconnection logic or stream termination.
4. **Test `runs watch` (`watch.rs`):**
    * Similar to logs, but emit run status update events.
    * Verify the CLI output updates accordingly.

## Phase 4: Dynamic Schema Resolution (Architectural Enhancement)

**Goal:** Implement the OCI event stream consumer to dynamically update the JSON schema cache, removing the reliance on a background sync interval.

1. **Design & Discovery:**
    * Define the structure of the OCI event payload we expect to receive (e.g., conforming to CloudEvents).
    * Identify where the current schema cache resides (likely in `stormchaser-api` or `stormchaser-engine`).
2. **Event Ingestion Layer:**
    * Create a new module (e.g., in `stormchaser-engine/src/handler/integrations/oci/` or an API webhook route) to receive the event stream.
    * Implement deserialization and validation of the incoming OCI events.
3. **Cache Invalidation/Update Logic:**
    * When an event indicating a schema change (e.g., a new tag pushed to the registry) is received, trigger a targeted fetch of the new schema.
    * Update the internal shared state/cache safely (using `RwLock` or similar concurrency controls).
4. **Testing:**
    * Write unit tests simulating incoming OCI events and asserting the cache updates.
    * Ensure thread safety in tests.
