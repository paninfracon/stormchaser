//! Core definitions and event loops for the Stormchaser Terminal User Interface (TUI).

/// Application state and business logic for the TUI.
pub mod app;
#[cfg(test)]
mod tests;
/// User interface rendering logic.
pub mod ui;

use ratatui::crossterm::event::Event;
use stormchaser_model::RunId;

/// Events that can trigger state changes in the TUI application.
pub enum AppEvent {
    /// A terminal event such as a key press or resize.
    Terminal(Event),
    /// A regular tick event for background processing or UI updates.
    Tick,
    /// A status update for a specific workflow run.
    StatusUpdate(RunId, String), // run_id, status
    /// A status update for a specific step within a workflow run.
    StepUpdate(RunId, String, String), // run_id, step_name, status
    /// A new log line received for a specific workflow run.
    LogLine(RunId, String), // run_id, line
    /// Fetched historical logs for a specific step.
    StepLogsFetched(RunId, usize, Vec<String>), // run_id, step_index, logs
    /// An update with partial details for a workflow run, typically from the run list.
    WorkflowUpdate(app::WorkflowRunDetail),
    /// An update with full details for a workflow run, including steps and artifacts.
    FullRunUpdate(app::WorkflowRunFullDetail),
    /// Request to start watching a specific workflow run.
    StartWatching(RunId),
    /// Successful login event with token and optional refresh token.
    LoginSuccessful(String, Option<String>), // token, refresh_token
    /// Login failure event with an error message.
    LoginFailed(String),
    /// Token expired event requiring a new login or refresh.
    TokenExpired,
}
