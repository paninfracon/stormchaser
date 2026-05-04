# Testing Improvement Plan

Following the structural quality pass on `stormchaser-tui` and `stormchaser-cli`, several key areas were identified with 0% code coverage. This document outlines the prioritized plan to improve test coverage in those specific modules.

## 1. TUI Application Logic & Event Handling

The core event loop and data handling logic for the TUI currently lack automated tests.

**Target Modules:**

- `crates/stormchaser-tui/src/app/handlers.rs`
- `crates/stormchaser-tui/src/app/api/*.rs` (e.g., `auth.rs`, `runs.rs`, `storage_backends.rs`, `webhooks.rs`)

**Action Items:**

- [ ] Create isolated unit tests for `App::handle_status_update`, `handle_full_run_update`, and other data ingestion methods in `handlers.rs`.
- [ ] Implement mock backend API endpoints using a tool like `wiremock` to test the API interaction methods in `crates/stormchaser-tui/src/app/api/`.
- [ ] Add tests verifying state transitions when processing key events (e.g., `handle_filter_dialog_key`, `handle_storage_backend_dialog_key`) in `handlers.rs`.

## 2. TUI Rendering (Main Runs Tab & Dialogs)

While the recently added `storage.rs` and `webhooks.rs` rendering modules have excellent coverage via `insta` snapshots, the legacy components do not.

**Target Modules:**

- `crates/stormchaser-tui/src/ui/runs.rs`
- `crates/stormchaser-tui/src/ui/dialogs.rs`

**Action Items:**

- [ ] Implement `insta` snapshot tests for `render_runs_tab` and `render_run_detail` with various mocked application states (empty list, fully populated list, active step, error states).
- [ ] Add snapshot tests for the filter dialog, file browser, and schedule git dialog functions located in `ui/dialogs.rs`.

## 3. CLI Interactive Commands

The CLI commands that rely on streaming or real-time interaction are currently untested.

**Target Modules:**

- `crates/stormchaser-cli/src/commands/runs/logs.rs`
- `crates/stormchaser-cli/src/commands/runs/watch.rs`

**Action Items:**

- [ ] Write integration tests for the `logs` command. This will likely require setting up a mock `SSE` (Server-Sent Events) endpoint or mocking the internal HTTP client to simulate an active log stream.

## 4. Architectural Enhancements

- [ ] **Dynamic Schema Resolution via OCI Event Streams**: Consume event streams from the OCI schema registry to trigger dynamic schema cache updates, allowing instant validation rule changes without waiting for the background sync interval.
