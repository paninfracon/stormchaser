use crate::AppEvent;
use chrono::{DateTime, Utc};
use ratatui::widgets::ListState;
use serde::{Deserialize, Serialize};
use stormchaser_model::workflow::RunStatus;
use tokio::sync::mpsc;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowRunDetail {
    pub id: Uuid,
    pub workflow_name: String,
    pub initiating_user: String,
    pub status: RunStatus,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepDetail {
    pub instance: serde_json::Value,
    pub outputs: Vec<serde_json::Value>,
    pub history: Vec<serde_json::Value>,
    pub logs: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowRunFullDetail {
    pub detail: WorkflowRunDetail,
    pub steps: Vec<StepDetail>,
    pub artifacts: Vec<stormchaser_model::storage::ArtifactRegistry>,
    pub test_summaries: Vec<stormchaser_model::test_report::TestSummary>,
    pub test_cases: Vec<stormchaser_model::test_report::TestCase>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthExchangeRequest {
    pub sso_token: String,
    pub callback_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthExchangeResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AppState {
    LoggedOut,
    LoggingIn,
    LoggedIn,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Pane {
    RunsList,
    RunDetail,
    TestResults,
}

pub struct App<'a> {
    pub url: String,
    pub state: AppState,
    pub token: Option<String>,
    pub refresh_token: Option<String>,
    pub filter_owner: Option<String>,
    pub filter_name: Option<String>,
    pub filter_repo_url: Option<String>,
    pub filter_workflow_path: Option<String>,
    pub filter_created_after: Option<String>,
    pub filter_created_before: Option<String>,
    pub filter_status: Option<String>,
    pub runs: Vec<WorkflowRunDetail>,
    pub runs_state: ListState,
    pub selected_run: Option<WorkflowRunFullDetail>,
    pub selected_step_index: usize,
    pub run_logs: Vec<String>,
    pub log_scroll: usize,
    pub log_auto_scroll: bool,
    pub overview_scroll: usize,
    pub active_pane: Pane,
    pub error: Option<String>,
    pub render_time: Option<DateTime<Utc>>,
    pub last_refresh_time: Option<DateTime<Utc>>,
    pub status_tx: mpsc::Sender<AppEvent>,
    pub watcher_handle: Option<tokio::task::JoinHandle<()>>,
    pub log_handle: Option<tokio::task::JoinHandle<()>>,
    pub workflow_handle: Option<tokio::task::JoinHandle<()>>,
    pub filter_dialog_active: bool,
    pub filter_focus: usize,
    pub filter_inputs: Vec<ratatui_textarea::TextArea<'a>>,
    pub filter_status_index: usize,
    pub schedule_git_dialog_active: bool,
    pub schedule_git_focus: usize,
    pub schedule_git_inputs: Vec<ratatui_textarea::TextArea<'a>>,
    pub file_browser_active: bool,
    pub file_explorer: tui_file_explorer::FileExplorer,
    pub direct_submit_form: Option<ratatui_form::Form>,
    pub direct_submit_dsl: Option<String>,
    pub cached_runs: std::collections::HashMap<uuid::Uuid, WorkflowRunFullDetail>,
    // Use the lifetime param to satisfy rust compiler. This avoids removing the lifetime everywhere.
    pub _marker: std::marker::PhantomData<&'a ()>,
}

pub const FILTER_STATUS_OPTIONS: &[&str] = &[
    "Any",
    "Queued",
    "Resolving",
    "Running",
    "Succeeded",
    "Failed",
    "Aborted",
];

pub mod api;
pub mod dialogs;
pub mod handlers;
pub mod navigation;
pub mod watch;

impl<'a> App<'a> {
    pub fn new(url: String, token: Option<String>, status_tx: mpsc::Sender<AppEvent>) -> Self {
        Self {
            url,
            state: AppState::LoggedOut,
            token,
            refresh_token: None,
            filter_owner: None,
            filter_name: None,
            filter_repo_url: None,
            filter_workflow_path: None,
            filter_created_after: None,
            filter_created_before: None,
            filter_status: None,
            runs: Vec::new(),
            runs_state: ListState::default(),
            selected_run: None,
            selected_step_index: 0,
            run_logs: Vec::new(),
            log_scroll: 0,
            log_auto_scroll: true,
            overview_scroll: 0,
            active_pane: Pane::RunsList,
            error: None,
            render_time: None,
            last_refresh_time: None,
            status_tx,
            watcher_handle: None,
            log_handle: None,
            workflow_handle: None,
            filter_dialog_active: false,
            filter_focus: 0,
            filter_inputs: Vec::new(),
            filter_status_index: 0,
            schedule_git_dialog_active: false,
            schedule_git_focus: 0,
            schedule_git_inputs: Vec::new(),
            file_browser_active: false,
            direct_submit_form: None,
            direct_submit_dsl: None,
            cached_runs: std::collections::HashMap::new(),
            file_explorer: tui_file_explorer::FileExplorer::new(
                std::env::current_dir().unwrap_or_default(),
                vec!["storm".to_string()],
            ),
            _marker: std::marker::PhantomData,
        }
    }
}
