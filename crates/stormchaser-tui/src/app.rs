use crate::AppEvent;
use anyhow::Result;
use chrono::{DateTime, Utc};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use ratatui::widgets::ListState;
use serde::{Deserialize, Serialize};
use stormchaser_model::workflow::RunStatus;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

    pub async fn api_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let client = reqwest::Client::new();
        let mut req = client.request(method, format!("{}{}", self.url, path));
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        Ok(req.send().await?)
    }

    pub async fn login(&mut self) -> Result<()> {
        self.state = AppState::LoggingIn;
        self.error = None;
        let port = 8080;
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
        let callback_url = format!("http://localhost:{}/callback", port);

        let login_url = format!(
            "{}/api/v1/auth/login?callback_url={}",
            self.url,
            urlencoding::encode(&callback_url)
        );

        if let Err(e) = open::that(&login_url) {
            self.error = Some(format!("Failed to open browser: {}", e));
            return Ok(());
        }

        let url = self.url.clone();
        let tx = self.status_tx.clone();

        tokio::spawn(async move {
            let callback_url_val = callback_url.clone();
            let (mut stream, _) = match tokio::time::timeout(
                std::time::Duration::from_secs(300), // 5 minute timeout for user to login
                listener.accept(),
            )
            .await
            {
                Ok(Ok(s)) => s,
                _ => {
                    let _ = tx
                        .send(AppEvent::LoginFailed(
                            "Login timed out or failed to accept connection".to_string(),
                        ))
                        .await;
                    return;
                }
            };

            let mut buf = [0; 4096];
            let mut request_str = String::new();
            if let Ok(n) = stream.read(&mut buf).await {
                request_str = String::from_utf8_lossy(&buf[0..n]).to_string();
            }

            let mut code = None;
            if let Some(path) = request_str.split_whitespace().nth(1) {
                if let Some(query) = path.split('?').nth(1) {
                    if let Ok(params) = serde_urlencoded::from_str::<
                        std::collections::HashMap<String, String>,
                    >(query)
                    {
                        code = params.get("code").cloned();
                    }
                }
            }

            // Respond to browser
            let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Login Successful</h1><p>You can close this window now.</p><script>window.close();</script></body></html>";
            let _ = stream.write_all(response.as_bytes()).await;

            if let Some(code) = code {
                let client = reqwest::Client::new();
                match client
                    .post(format!("{}/api/v1/auth/exchange", url))
                    .json(&serde_json::json!({
                        "sso_token": code,
                        "callback_url": callback_url_val
                    }))
                    .send()
                    .await
                {
                    Ok(res) => {
                        if res.status().is_success() {
                            if let Ok(auth_res) = res.json::<AuthExchangeResponse>().await {
                                let _ = tx
                                    .send(AppEvent::LoginSuccessful(
                                        auth_res.access_token,
                                        auth_res.refresh_token,
                                    ))
                                    .await;
                            } else {
                                let _ = tx
                                    .send(AppEvent::LoginFailed(
                                        "Failed to parse token response".to_string(),
                                    ))
                                    .await;
                            }
                        } else {
                            let _ = tx
                                .send(AppEvent::LoginFailed(format!(
                                    "Token exchange failed: {}",
                                    res.status()
                                )))
                                .await;
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(AppEvent::LoginFailed(format!("Request failed: {}", e)))
                            .await;
                    }
                }
            } else {
                let _ = tx
                    .send(AppEvent::LoginFailed(
                        "No authorization code received from provider".to_string(),
                    ))
                    .await;
            }
        });

        Ok(())
    }

    pub async fn refresh_session(&mut self) -> Result<bool> {
        if let Some(refresh_token) = self.refresh_token.clone() {
            let res = self
                .api_request(
                    reqwest::Method::POST,
                    "/api/v1/auth/refresh",
                    Some(serde_json::json!({ "refresh_token": refresh_token })),
                )
                .await?;

            if res.status().is_success() {
                let auth_res: AuthExchangeResponse = res.json().await?;
                self.token = Some(auth_res.access_token);
                if auth_res.refresh_token.is_some() {
                    self.refresh_token = auth_res.refresh_token;
                }
                self.state = AppState::LoggedIn;
                return Ok(true);
            }
        }

        self.token = None;
        self.refresh_token = None;
        self.state = AppState::LoggedOut;
        Ok(false)
    }

    pub fn open_filter_dialog(&mut self) {
        self.filter_dialog_active = true;
        self.filter_focus = 0;
        self.filter_inputs = vec![
            ratatui_textarea::TextArea::from(vec![self.filter_owner.clone().unwrap_or_default()]),
            ratatui_textarea::TextArea::from(vec![self.filter_name.clone().unwrap_or_default()]),
            ratatui_textarea::TextArea::from(vec![self
                .filter_repo_url
                .clone()
                .unwrap_or_default()]),
            ratatui_textarea::TextArea::from(vec![self
                .filter_workflow_path
                .clone()
                .unwrap_or_default()]),
            ratatui_textarea::TextArea::from(vec![self
                .filter_created_after
                .clone()
                .unwrap_or_default()]),
            ratatui_textarea::TextArea::from(vec![self
                .filter_created_before
                .clone()
                .unwrap_or_default()]),
        ];

        let current_status = self.filter_status.as_deref().unwrap_or("Any");
        self.filter_status_index = FILTER_STATUS_OPTIONS
            .iter()
            .position(|&s| s.eq_ignore_ascii_case(current_status))
            .unwrap_or(0);
    }

    pub fn open_file_browser(&mut self) {
        self.file_browser_active = true;
    }

    pub fn open_schedule_git_dialog(&mut self) {
        self.schedule_git_dialog_active = true;
        self.schedule_git_focus = 0;
        self.schedule_git_inputs = vec![
            ratatui_textarea::TextArea::default(), // repo_url
            ratatui_textarea::TextArea::default(), // workflow_path
            ratatui_textarea::TextArea::default(), // git_ref
        ];
    }

    pub async fn submit_schedule_git(&mut self) -> Result<()> {
        if self.schedule_git_inputs.len() == 3 {
            let repo_url = self.schedule_git_inputs[0].lines()[0].trim().to_string();
            let workflow_path = self.schedule_git_inputs[1].lines()[0].trim().to_string();
            let git_ref = self.schedule_git_inputs[2].lines()[0].trim().to_string();

            if repo_url.is_empty() || workflow_path.is_empty() || git_ref.is_empty() {
                self.error = Some("All fields must be provided".to_string());
                return Ok(());
            }

            let res = self
                .api_request(
                    reqwest::Method::POST,
                    "/api/v1/runs/git",
                    Some(serde_json::json!({
                        "repo_url": repo_url,
                        "workflow_path": workflow_path,
                        "git_ref": git_ref,
                        "inputs": {}
                    })),
                )
                .await?;

            if res.status().is_success() {
                self.schedule_git_dialog_active = false;
                self.refresh_runs().await?;
            } else {
                self.error = Some(format!("Failed to schedule workflow: {}", res.status()));
            }
        }
        Ok(())
    }

    pub async fn submit_file(&mut self) -> Result<()> {
        let path = self
            .file_explorer
            .current_entry()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("storm") {
            let dsl = std::fs::read_to_string(path)?;

            if let Ok(workflow) = stormchaser_dsl::StormchaserParser.parse(&dsl) {
                if !workflow.inputs.is_empty() {
                    let mut builder = ratatui_form::Form::builder().title("Workflow Inputs");
                    for input in workflow.inputs {
                        let mut field = builder.text(
                            &input.name,
                            input.description.as_deref().unwrap_or(&input.name),
                        );
                        if let Some(default) = &input.default {
                            if let Some(s) = default.as_str() {
                                field = field.placeholder(s);
                            } else {
                                field = field.placeholder(default.to_string());
                            }
                        }
                        builder = field.done();
                    }
                    self.direct_submit_form = Some(builder.build());
                    self.direct_submit_dsl = Some(dsl);
                    self.file_browser_active = false;
                    return Ok(());
                }
            }

            let res = self
                .api_request(
                    reqwest::Method::POST,
                    "/api/v1/runs/direct",
                    Some(serde_json::json!({ "dsl": dsl, "inputs": {} })),
                )
                .await?;

            if res.status().is_success() {
                self.file_browser_active = false;
                self.refresh_runs().await?;
            } else {
                self.error = Some(format!("Failed to submit workflow: {}", res.status()));
            }
        }
        Ok(())
    }

    pub async fn submit_direct_form(&mut self) -> Result<()> {
        if let (Some(form), Some(dsl)) = (
            self.direct_submit_form.take(),
            self.direct_submit_dsl.take(),
        ) {
            let inputs_json = form.to_json();
            let res = self
                .api_request(
                    reqwest::Method::POST,
                    "/api/v1/runs/direct",
                    Some(serde_json::json!({ "dsl": dsl, "inputs": inputs_json })),
                )
                .await?;

            if res.status().is_success() {
                self.refresh_runs().await?;
            } else {
                self.error = Some(format!("Failed to submit workflow: {}", res.status()));
            }
        }
        Ok(())
    }

    pub async fn apply_filters(&mut self) -> Result<()> {
        if self.filter_inputs.len() == 6 {
            let o = self.filter_inputs[0].lines()[0].trim().to_string();
            self.filter_owner = if o.is_empty() { None } else { Some(o) };

            let n = self.filter_inputs[1].lines()[0].trim().to_string();
            self.filter_name = if n.is_empty() { None } else { Some(n) };

            let r = self.filter_inputs[2].lines()[0].trim().to_string();
            self.filter_repo_url = if r.is_empty() { None } else { Some(r) };

            let w = self.filter_inputs[3].lines()[0].trim().to_string();
            self.filter_workflow_path = if w.is_empty() { None } else { Some(w) };

            let ca = self.filter_inputs[4].lines()[0].trim().to_string();
            self.filter_created_after = if ca.is_empty() { None } else { Some(ca) };

            let cb = self.filter_inputs[5].lines()[0].trim().to_string();
            self.filter_created_before = if cb.is_empty() { None } else { Some(cb) };

            let s = FILTER_STATUS_OPTIONS[self.filter_status_index];
            self.filter_status = if s == "Any" {
                None
            } else {
                Some(s.to_lowercase())
            };
        }
        self.filter_dialog_active = false;
        self.refresh_runs().await
    }

    pub async fn refresh_runs(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let mut query_params = Vec::new();
        if let Some(owner) = &self.filter_owner {
            query_params.push(format!("initiating_user={}", owner));
        }
        if let Some(name) = &self.filter_name {
            query_params.push(format!("workflow_name={}", name));
        }
        if let Some(repo) = &self.filter_repo_url {
            query_params.push(format!("repo_url={}", repo));
        }
        if let Some(path) = &self.filter_workflow_path {
            query_params.push(format!("workflow_path={}", path));
        }
        if let Some(ca) = &self.filter_created_after {
            query_params.push(format!("created_after={}", ca));
        }
        if let Some(cb) = &self.filter_created_before {
            query_params.push(format!("created_before={}", cb));
        }
        if let Some(status) = &self.filter_status {
            query_params.push(format!("status={}", status));
        }

        let query = if query_params.is_empty() {
            String::new()
        } else {
            format!("?{}", query_params.join("&"))
        };

        let res = self
            .api_request(
                reqwest::Method::GET,
                &format!("/api/v1/runs{}", query),
                None,
            )
            .await?;

        if res.status().is_success() {
            self.runs = res.json::<Vec<WorkflowRunDetail>>().await?;
            if !self.runs.is_empty() {
                if self.runs_state.selected().is_none() {
                    self.runs_state.select(Some(0));
                }
                if self.selected_run.is_none() {
                    if let Some(i) = self.runs_state.selected() {
                        let id = self.runs[i].id;
                        let _ = self.fetch_run_detail(id).await;
                        self.start_watching(id).await;
                    }
                }
            }
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch runs: {}", res.status()));
        }
        Ok(())
    }

    pub async fn fetch_run_detail(&mut self, run_id: Uuid) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }
        if let Some(cached) = self.cached_runs.get(&run_id).cloned() {
            self.handle_full_run_update(cached);
            self.error = None;
            return Ok(());
        }
        let res = self
            .api_request(
                reqwest::Method::GET,
                &format!("/api/v1/runs/{}", run_id),
                None,
            )
            .await?;

        if res.status().is_success() {
            let full_detail = res.json::<WorkflowRunFullDetail>().await?;
            self.handle_full_run_update(full_detail);
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch run detail: {}", res.status()));
        }
        Ok(())
    }

    pub fn next_run(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let i = match self.runs_state.selected() {
            Some(i) => {
                if i >= self.runs.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.runs_state.select(Some(i));
        self.selected_step_index = 0;
        self.selected_run = None;

        // Trigger fetch of new details
        let id = self.runs[i].id;
        let tx = self.status_tx.clone();
        tokio::spawn(async move {
            let _ = tx
                .send(AppEvent::StatusUpdate(id, "force_refresh".to_string()))
                .await;
            let _ = tx.send(AppEvent::StartWatching(id)).await;
        });
    }

    pub fn previous_run(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let i = match self.runs_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.runs.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.runs_state.select(Some(i));
        self.selected_step_index = 0;
        self.selected_run = None;

        // Trigger fetch of new details
        let id = self.runs[i].id;
        let tx = self.status_tx.clone();
        tokio::spawn(async move {
            let _ = tx
                .send(AppEvent::StatusUpdate(id, "force_refresh".to_string()))
                .await;
            let _ = tx.send(AppEvent::StartWatching(id)).await;
        });
    }

    pub fn next_step(&mut self) {
        if let Some(run) = &self.selected_run {
            if !run.steps.is_empty() {
                self.selected_step_index = (self.selected_step_index + 1) % run.steps.len();
                self.refresh_step_logs(true);
            }
        }
    }

    pub fn previous_step(&mut self) {
        if let Some(run) = &self.selected_run {
            if !run.steps.is_empty() {
                if self.selected_step_index == 0 {
                    self.selected_step_index = run.steps.len() - 1;
                } else {
                    self.selected_step_index -= 1;
                }
                self.refresh_step_logs(true);
            }
        }
    }

    pub fn refresh_step_logs(&mut self, reset_scroll: bool) {
        if let Some(run) = &self.selected_run {
            if let Some(step) = run.steps.get(self.selected_step_index) {
                // Efficiently update log vector without reallocating if possible
                self.run_logs.clear();
                self.run_logs.extend_from_slice(&step.logs);

                if reset_scroll {
                    self.log_scroll = 0;
                    self.log_auto_scroll = true;
                }
            } else {
                self.run_logs.clear();
                if reset_scroll {
                    self.log_scroll = 0;
                    self.log_auto_scroll = true;
                }
            }
        }
    }

    pub async fn start_watching(&mut self, id: Uuid) {
        if let Some(handle) = self.watcher_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.log_handle.take() {
            handle.abort();
        }

        if self.cached_runs.contains_key(&id) {
            // If the run is completed and cached, no need to watch for live updates
            return;
        }

        // We don't clear logs here because we want to keep the historical logs
        // from the full fetch until SSE lines arrive.

        let url = self.url.clone();
        let token = self.token.clone();
        let tx = self.status_tx.clone();

        let status_url = format!("{}/api/v1/runs/{}/status/stream", url, id);
        let log_url = format!("{}/api/v1/runs/{}/logs/stream", url, id);

        let tx_status = tx.clone();
        let token_status = token.clone();
        let status_handle = tokio::spawn(async move {
            if let Some(token) = token_status {
                let client = reqwest::Client::new();
                if let Ok(res) = client
                    .get(&status_url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                        let _ = tx_status.send(AppEvent::TokenExpired).await;
                        return;
                    }

                    let mut stream = res.bytes_stream().eventsource();
                    while let Some(event) = stream.next().await {
                        if let Ok(event) = event {
                            match event.event.as_str() {
                                "workflow_run" => {
                                    if let Ok(run) =
                                        serde_json::from_str::<WorkflowRunDetail>(&event.data)
                                    {
                                        let _ = tx_status.send(AppEvent::WorkflowUpdate(run)).await;
                                    }
                                }
                                "run_status" => {
                                    let _ = tx_status
                                        .send(AppEvent::StatusUpdate(id, event.data))
                                        .await;
                                }
                                "step_status" => {
                                    if let Ok(payload) =
                                        serde_json::from_str::<serde_json::Value>(&event.data)
                                    {
                                        let step_name = payload["step_name"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string();
                                        let status = payload["status"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string();
                                        let _ = tx_status
                                            .send(AppEvent::StepUpdate(id, step_name, status))
                                            .await;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        });

        let tx_log = tx.clone();
        let token_log = token.clone();
        let log_handle = tokio::spawn(async move {
            if let Some(token) = token_log {
                let client = reqwest::Client::new();
                if let Ok(res) = client
                    .get(&log_url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                        let _ = tx_log.send(AppEvent::TokenExpired).await;
                        return;
                    }

                    let mut stream = res.bytes_stream().eventsource();
                    while let Some(event) = stream.next().await {
                        if let Ok(event) = event {
                            if event.event == "log" {
                                let _ = tx_log.send(AppEvent::LogLine(id, event.data)).await;
                            }
                        }
                    }
                }
            }
        });

        self.watcher_handle = Some(status_handle);
        self.log_handle = Some(log_handle);
    }

    pub fn handle_status_update(&mut self, run_id: Uuid, status: String) {
        if status == "refresh" || status == "force_refresh" {
            if let Some(cached) = self.cached_runs.get(&run_id).cloned() {
                let tx = self.status_tx.clone();
                tokio::spawn(async move {
                    let _ = tx.send(AppEvent::FullRunUpdate(cached)).await;
                });
                return;
            }

            let now = Utc::now();
            if status == "refresh" {
                if let Some(last) = self.last_refresh_time {
                    if now.signed_duration_since(last).num_seconds() < 2 {
                        return; // Throttle periodic refreshes
                    }
                }
            }
            self.last_refresh_time = Some(now);

            let url = self.url.clone();
            let token = self.token.clone();
            let tx = self.status_tx.clone();
            tokio::spawn(async move {
                if let Some(token) = token {
                    let client = reqwest::Client::new();
                    if let Ok(res) = client
                        .get(format!("{}/api/v1/runs/{}", url, run_id))
                        .header("Authorization", format!("Bearer {}", token))
                        .send()
                        .await
                    {
                        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                            let _ = tx.send(AppEvent::TokenExpired).await;
                            return;
                        }
                        if let Ok(full_detail) = res.json::<WorkflowRunFullDetail>().await {
                            let _ = tx.send(AppEvent::FullRunUpdate(full_detail)).await;
                        }
                    }
                }
            });
            return;
        }

        let parsed_status = if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&status)
        {
            payload
                .get("status")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
                .unwrap_or(status.clone())
        } else {
            status.clone()
        };

        if let Some(run) = self.runs.iter_mut().find(|r| r.id == run_id) {
            if let Ok(s) = serde_json::from_value(serde_json::Value::String(parsed_status.clone()))
            {
                // Prevent downgrading terminal statuses
                match run.status {
                    RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {
                        // Do not overwrite a terminal status with a non-terminal one
                    }
                    _ => run.status = s,
                }
            }
        }
        if let Some(run) = &mut self.selected_run {
            if run.detail.id == run_id {
                if let Ok(s) = serde_json::from_value(serde_json::Value::String(parsed_status)) {
                    match run.detail.status {
                        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {}
                        _ => run.detail.status = s,
                    }
                }
            }
        }
    }

    pub fn handle_full_run_update(&mut self, mut full_detail: WorkflowRunFullDetail) {
        let current_selected_id = self.runs_state.selected().map(|i| self.runs[i].id);

        if Some(full_detail.detail.id) == current_selected_id {
            // Preserve existing state if the new detail has "older" status or empty logs
            if let Some(old_run) = &self.selected_run {
                if old_run.detail.id == full_detail.detail.id {
                    // 1. Prevent run status regression
                    match old_run.detail.status {
                        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {
                            full_detail.detail.status = old_run.detail.status.clone();
                        }
                        _ => {}
                    }

                    // 2. Prevent step status regression and preserve logs
                    for step in &mut full_detail.steps {
                        if let Some(step_name) =
                            step.instance.get("step_name").and_then(|v| v.as_str())
                        {
                            if let Some(old_step) = old_run.steps.iter().find(|s| {
                                s.instance.get("step_name").and_then(|v| v.as_str())
                                    == Some(step_name)
                            }) {
                                // Preserve logs if new ones are empty
                                if step.logs.is_empty() && !old_step.logs.is_empty() {
                                    step.logs = old_step.logs.clone();
                                }

                                // Prevent step status regression
                                if let Some(old_status) =
                                    old_step.instance.get("status").and_then(|v| v.as_str())
                                {
                                    if let Some(new_status_val) = step.instance.get_mut("status") {
                                        if let Some(new_status) = new_status_val.as_str() {
                                            // Simple heuristic: don't move back to unpacking/pending if already running
                                            if (old_status == "running"
                                                || old_status == "succeeded"
                                                || old_status == "failed")
                                                && (new_status == "unpacking_sfs"
                                                    || new_status == "pending")
                                            {
                                                *new_status_val = serde_json::Value::String(
                                                    old_status.to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Optimized log cleaning
            for step in &mut full_detail.steps {
                for log in &mut step.logs {
                    if log.contains('\r') {
                        *log = log.replace('\r', "");
                    }
                    let trimmed = log.trim_end();
                    if trimmed.len() != log.len() {
                        *log = trimmed.to_string();
                    }
                }
            }

            self.selected_run = Some(full_detail.clone());

            match full_detail.detail.status {
                RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {
                    self.cached_runs.insert(full_detail.detail.id, full_detail);
                }
                _ => {}
            }

            if let Some(run) = &self.selected_run {
                // Keep selected_step_index in bounds
                if self.selected_step_index >= run.steps.len() {
                    self.selected_step_index = if run.steps.is_empty() {
                        0
                    } else {
                        run.steps.len() - 1
                    };
                }
            }
            self.refresh_step_logs(false);
        }
    }

    pub fn handle_step_update(&mut self, run_id: Uuid, step_name: String, status: String) {
        if let Some(run) = &mut self.selected_run {
            if run.detail.id == run_id {
                let mut updated_step_index = None;

                for (i, step_detail) in run.steps.iter_mut().enumerate() {
                    if step_detail
                        .instance
                        .get("step_name")
                        .and_then(|v| v.as_str())
                        == Some(&step_name)
                    {
                        if let Some(obj) = step_detail.instance.as_object_mut() {
                            obj.insert(
                                "status".to_string(),
                                serde_json::Value::String(status.clone()),
                            );
                            updated_step_index = Some(i);
                        }
                    }
                }

                // If ANY step status updated, refresh logs and potentially trigger fetch
                if let Some(idx) = updated_step_index {
                    if idx == self.selected_step_index {
                        self.refresh_step_logs(false);
                    }

                    // Always trigger a refresh to get the new history entry (throttled)
                    let tx = self.status_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx
                            .send(AppEvent::StatusUpdate(run_id, "refresh".to_string()))
                            .await;
                    });
                }
            }
        }
    }

    pub fn handle_log_line(&mut self, run_id: Uuid, line: String) {
        let line = line.replace('\r', "");
        if let Some(run) = &mut self.selected_run {
            if run.detail.id == run_id {
                let mut step_name_and_clean_line = None;

                // Update the logs for the specific step in our model
                for step in &mut run.steps {
                    if let Some(name) = step.instance.get("step_name").and_then(|v| v.as_str()) {
                        // Check if the log line starts with [STEP_NAME]
                        let prefix = format!("[{}]", name);
                        if line.starts_with(&prefix) {
                            let clean_line = line
                                .strip_prefix(&prefix)
                                .unwrap_or(&line)
                                .trim()
                                .to_string();
                            step.logs.push(clean_line.clone());
                            step_name_and_clean_line = Some((name.to_string(), clean_line));
                            break;
                        }
                    }
                }

                // If the selected step was updated, update current log view
                if let Some((name, clean_line)) = step_name_and_clean_line {
                    if let Some(current_step) = run.steps.get(self.selected_step_index) {
                        if current_step
                            .instance
                            .get("step_name")
                            .and_then(|v| v.as_str())
                            == Some(&name)
                        {
                            self.run_logs.push(clean_line);
                        }
                    }
                }
            }
        }
    }

    pub fn handle_workflow_update(&mut self, run_detail: WorkflowRunDetail) {
        if let Some(run) = self.runs.iter_mut().find(|r| r.id == run_detail.id) {
            *run = run_detail.clone();
        } else {
            self.runs.insert(0, run_detail.clone());
            // Adjust selection if something was selected
            if let Some(selected) = self.runs_state.selected() {
                self.runs_state.select(Some(selected + 1));
            }
        }

        let current_selected_id = self.runs_state.selected().map(|i| self.runs[i].id);
        if Some(run_detail.id) == current_selected_id {
            if let Some(run) = &mut self.selected_run {
                run.detail = run_detail;
            }
        }
    }

    pub fn scroll_logs_up(&mut self) {
        self.log_auto_scroll = false;
        if self.log_scroll > 0 {
            self.log_scroll -= 1;
        }
    }

    pub fn scroll_logs_down(&mut self) {
        self.log_scroll += 1;
    }

    pub fn scroll_overview_up(&mut self) {
        if self.overview_scroll > 0 {
            self.overview_scroll -= 1;
        }
    }

    pub fn scroll_overview_down(&mut self) {
        self.overview_scroll += 1;
    }

    pub fn scroll_logs_to_top(&mut self) {
        self.log_auto_scroll = false;
        self.log_scroll = 0;
    }

    pub async fn start_listening_for_workflows(&mut self) {
        if let Some(handle) = self.workflow_handle.take() {
            handle.abort();
        }
        let url = self.url.clone();
        let tx = self.status_tx.clone();
        let token = self.token.clone();

        let workflow_handle = tokio::spawn(async move {
            loop {
                if let Some(token) = &token {
                    let client = reqwest::Client::new();
                    if let Ok(res) = client
                        .get(format!("{}/api/v1/runs/stream", url))
                        .header("Authorization", format!("Bearer {}", token))
                        .send()
                        .await
                    {
                        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                            let _ = tx.send(AppEvent::TokenExpired).await;
                            return;
                        }

                        let mut stream = res.bytes_stream().eventsource();
                        while let Some(event) = stream.next().await {
                            if let Ok(event) = event {
                                if event.event == "workflow_run" {
                                    if let Ok(run) =
                                        serde_json::from_str::<WorkflowRunDetail>(&event.data)
                                    {
                                        let _ = tx.send(AppEvent::WorkflowUpdate(run)).await;
                                    }
                                }
                            }
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
        self.workflow_handle = Some(workflow_handle);
    }
}
