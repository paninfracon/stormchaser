pub mod app;
#[cfg(test)]
mod tests;
pub mod ui;

use ratatui::crossterm::event::Event;
use uuid::Uuid;

pub enum AppEvent {
    Terminal(Event),
    Tick,
    StatusUpdate(Uuid, String),       // run_id, status
    StepUpdate(Uuid, String, String), // run_id, step_name, status
    LogLine(Uuid, String),            // run_id, line
    WorkflowUpdate(app::WorkflowRunDetail),
    FullRunUpdate(app::WorkflowRunFullDetail),
    StartWatching(Uuid),
    LoginSuccessful(String, Option<String>), // token, refresh_token
    LoginFailed(String),
    TokenExpired,
}
