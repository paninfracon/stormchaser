# Testing Improvement Plan (Completed)

Following the structural quality pass on `stormchaser-tui` and `stormchaser-cli`, several key areas were identified with 0% code coverage. This plan has now been fully executed.

## 1. TUI Application Logic & Event Handling (Completed)

The core event loop and data handling logic for the TUI now have automated tests.

**Target Modules:**

- `crates/stormchaser-tui/src/app/handlers.rs`
- `crates/stormchaser-tui/src/app/api/*.rs` (e.g., `auth.rs`, `runs.rs`, `storage_backends.rs`, `webhooks.rs`)

**Completed Items:**

- [x] Created isolated unit tests for `App::handle_status_update` and `handle_full_run_update` in `handlers/runs.rs` to ensure run state and logs are preserved properly.
- [x] Implemented mock backend API endpoints using `wiremock` to test the API interaction methods in `crates/stormchaser-tui/src/app/api/`.
- [x] Added tests verifying state transitions when processing key events (e.g., `handle_filter_dialog_key`, `handle_storage_backend_dialog_key`) in `handlers.rs`.

## 2. TUI Rendering (Main Runs Tab & Dialogs) (Completed)

The legacy rendering components now have `insta` snapshot coverage matching the newer modules.

**Target Modules:**

- `crates/stormchaser-tui/src/ui/runs.rs`
- `crates/stormchaser-tui/src/ui/dialogs.rs`

**Completed Items:**

- [x] Implemented `insta` snapshot tests for `render_runs_tab` and `render_run_detail` with various mocked application states (empty list, fully populated list, active step, error states).
- [x] Added snapshot tests for the filter dialog, file browser, and schedule git dialog functions located in `ui/dialogs.rs`.

## 3. CLI Interactive Commands (Completed)

The CLI commands that rely on streaming or real-time interaction are now fully tested.

**Target Modules:**

- `crates/stormchaser-cli/src/commands/runs/logs.rs`
- `crates/stormchaser-cli/src/commands/runs/watch.rs`

**Completed Items:**

- [x] Wrote integration tests for the `logs` and `watch` commands using `wiremock` to simulate an active Server-Sent Events (SSE) stream.

## 4. Architectural Enhancements (Pending)

- [ ] **Dynamic Schema Resolution via OCI Event Streams**: Consume event streams from the OCI schema registry to trigger dynamic schema cache updates, allowing instant validation rule changes without waiting for the background sync interval.
