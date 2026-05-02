use super::*;
use crate::AppEvent;
use chrono::Utc;
use serde_json::Value;
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

impl<'a> App<'a> {
    /// Handles an incoming status update for a specific workflow run.
    /// Supports special "refresh" strings to trigger a full refetch.
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

        let parsed_status = if let Ok(payload) = serde_json::from_str::<Value>(&status) {
            payload
                .get("status")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
                .unwrap_or(status.clone())
        } else {
            status.clone()
        };

        if let Some(run) = self.runs.iter_mut().find(|r| r.id == run_id) {
            if let Ok(s) = serde_json::from_value(Value::String(parsed_status.clone())) {
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
                if let Ok(s) = serde_json::from_value(Value::String(parsed_status)) {
                    match run.detail.status {
                        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {}
                        _ => run.detail.status = s,
                    }
                }
            }
        }
    }

    /// Handles a complete update of a workflow run, including its steps and details.
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
                                                *new_status_val =
                                                    Value::String(old_status.to_string());
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

    /// Handles a status update for a specific step within a workflow run.
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
                            obj.insert("status".to_string(), Value::String(status.clone()));
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

    /// Appends a new log line to the appropriate step within the selected run.
    pub fn handle_step_logs_fetched(&mut self, run_id: Uuid, step_index: usize, logs: Vec<String>) {
        if let Some(run) = &mut self.selected_run {
            if run.detail.id == run_id {
                if let Some(step) = run.steps.get_mut(step_index) {
                    let mut final_logs = logs;
                    // Append any logs from the current step.logs that are not in the newly fetched logs.
                    // This preserves any live SSE lines that arrived during the fetch request.
                    for line in &step.logs {
                        if !final_logs.contains(line) {
                            final_logs.push(line.clone());
                        }
                    }
                    step.logs = final_logs;

                    if self.selected_step_index == step_index {
                        self.run_logs.clear();
                        self.run_logs.extend_from_slice(&step.logs);

                        if self.log_auto_scroll {
                            self.log_scroll = self.run_logs.len().saturating_sub(1);
                        }
                    }
                }
            }
        }
    }
    /// Appends a new log line to the appropriate step within the selected run.
    pub fn handle_log_line(&mut self, run_id: Uuid, line: String) {
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
                                .strip_prefix(' ')
                                .unwrap_or_else(|| line.strip_prefix(&prefix).unwrap_or(&line))
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

    /// Handles a partial summary update for a workflow run, usually from the global run list stream.
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
}

use ratatui::crossterm::event::{KeyCode, KeyEvent};

impl<'a> App<'a> {
    pub async fn handle_filter_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.filter_inputs.len() + 1;
        match key.code {
            KeyCode::Esc => {
                self.filter_dialog_active = false;
            }
            KeyCode::Enter => {
                let _ = self.apply_filters().await;
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.filter_focus = (self.filter_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Down | KeyCode::Tab => {
                self.filter_focus = (self.filter_focus + 1) % focus_count;
            }
            KeyCode::Left if self.filter_focus == 6 => {
                let opts_len = crate::app::FILTER_STATUS_OPTIONS.len();
                self.filter_status_index = (self.filter_status_index + opts_len - 1) % opts_len;
            }
            KeyCode::Right if self.filter_focus == 6 => {
                let opts_len = crate::app::FILTER_STATUS_OPTIONS.len();
                self.filter_status_index = (self.filter_status_index + 1) % opts_len;
            }
            _ => {
                if self.filter_focus < 6 {
                    self.filter_inputs[self.filter_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_schedule_git_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.schedule_git_inputs.len();
        match key.code {
            KeyCode::Esc => {
                self.schedule_git_dialog_active = false;
            }
            KeyCode::Enter => {
                let _ = self.submit_schedule_git().await;
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.schedule_git_focus = (self.schedule_git_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Down | KeyCode::Tab => {
                self.schedule_git_focus = (self.schedule_git_focus + 1) % focus_count;
            }
            _ => {
                self.schedule_git_inputs[self.schedule_git_focus].input(key);
            }
        }
    }

    pub async fn handle_storage_backend_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.storage_backend_inputs.len() + 2; // name, desc, config, arn, type, is_default
        match key.code {
            KeyCode::Esc => {
                self.storage_backend_dialog_active = false;
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let _ = self.submit_storage_backend_form().await;
            }
            KeyCode::BackTab => {
                self.storage_backend_focus =
                    (self.storage_backend_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Tab => {
                self.storage_backend_focus = (self.storage_backend_focus + 1) % focus_count;
            }
            KeyCode::Left if self.storage_backend_focus == 4 => {
                let opts_len = crate::app::BACKEND_TYPE_OPTIONS.len();
                self.storage_backend_type_index =
                    (self.storage_backend_type_index + opts_len - 1) % opts_len;
            }
            KeyCode::Right if self.storage_backend_focus == 4 => {
                let opts_len = crate::app::BACKEND_TYPE_OPTIONS.len();
                self.storage_backend_type_index = (self.storage_backend_type_index + 1) % opts_len;
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.storage_backend_focus == 5 => {
                self.storage_backend_is_default = !self.storage_backend_is_default;
            }
            _ => {
                if self.storage_backend_focus < 4 {
                    self.storage_backend_inputs[self.storage_backend_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_webhook_dialog_key(&mut self, key: KeyEvent) {
        let focus_count = self.webhook_inputs.len() + 2; // name, desc, token, type, is_active
        match key.code {
            KeyCode::Esc => {
                self.webhook_dialog_active = false;
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let _ = self.submit_webhook_form().await;
            }
            KeyCode::BackTab => {
                self.webhook_focus = (self.webhook_focus + focus_count - 1) % focus_count;
            }
            KeyCode::Tab => {
                self.webhook_focus = (self.webhook_focus + 1) % focus_count;
            }
            KeyCode::Left if self.webhook_focus == 3 => {
                let opts_len = crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS.len();
                self.webhook_source_type_index =
                    (self.webhook_source_type_index + opts_len - 1) % opts_len;
            }
            KeyCode::Right if self.webhook_focus == 3 => {
                let opts_len = crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS.len();
                self.webhook_source_type_index = (self.webhook_source_type_index + 1) % opts_len;
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.webhook_focus == 4 => {
                self.webhook_is_active = !self.webhook_is_active;
            }
            _ => {
                if self.webhook_focus < 3 {
                    self.webhook_inputs[self.webhook_focus].input(key);
                }
            }
        }
    }

    pub async fn handle_approval_dialog_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.approval_dialog_active = false;
            }
            KeyCode::Char('a') | KeyCode::Char('A')
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL) =>
            {
                let inputs_str = self.approval_inputs.lines().join("\n");
                let inputs_json =
                    serde_json::from_str(&inputs_str).unwrap_or(serde_json::json!({}));
                let _ = self.approve_selected_step(inputs_json).await;
            }
            _ => {
                self.approval_inputs.input(key);
            }
        }
    }

    pub async fn handle_direct_submit_form_key(&mut self, key: KeyEvent) {
        if let Some(ref mut form) = self.direct_submit_form {
            form.handle_input(key);
            match form.result() {
                ratatui_form::FormResult::Submitted => {
                    let _ = self.submit_direct_form().await;
                }
                ratatui_form::FormResult::Cancelled => {
                    self.direct_submit_form = None;
                    self.direct_submit_dsl = None;
                }
                ratatui_form::FormResult::Active => {}
            }
        }
    }

    pub async fn handle_file_browser_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.file_browser_active = false;
            }
            KeyCode::Enter => {
                if self.file_explorer.current_entry().is_some_and(|e| e.is_dir) {
                    let _ = self.file_explorer.handle_key(key);
                } else {
                    let _ = self.submit_file().await;
                }
            }
            _ => {
                let _ = self.file_explorer.handle_key(key);
            }
        }
    }

    pub async fn handle_default_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('f') | KeyCode::Char('/') => {
                self.open_filter_dialog();
            }
            KeyCode::Char('q') => return true,
            KeyCode::Char('j') | KeyCode::Down => match self.active_pane {
                Pane::RunsList => self.next_run(),
                Pane::StorageBackendsList => self.next_storage_backend(),
                Pane::WebhooksList => self.next_webhook(),
                _ => self.next_step(),
            },
            KeyCode::Char('k') | KeyCode::Up => match self.active_pane {
                Pane::RunsList => self.previous_run(),
                Pane::StorageBackendsList => self.previous_storage_backend(),
                Pane::WebhooksList => self.previous_webhook(),
                _ => self.previous_step(),
            },
            KeyCode::Char('1') => {
                self.active_pane = Pane::RunsList;
            }
            KeyCode::Char('2') => {
                self.active_pane = Pane::StorageBackendsList;
            }
            KeyCode::Char('3') => {
                self.active_pane = Pane::WebhooksList;
            }
            KeyCode::Tab => {
                self.active_pane = match self.active_pane {
                    Pane::RunsList => Pane::RunDetail,
                    Pane::RunDetail => Pane::TestResults,
                    Pane::TestResults => Pane::StorageBackendsList,
                    Pane::StorageBackendsList => Pane::StorageBackendDetail,
                    Pane::StorageBackendDetail => Pane::WebhooksList,
                    Pane::WebhooksList => Pane::WebhookDetail,
                    Pane::WebhookDetail => Pane::RunsList,
                };
            }
            KeyCode::Char('t') => {
                if self.active_pane == Pane::TestResults {
                    self.active_pane = Pane::RunDetail;
                } else if self.active_pane == Pane::RunDetail || self.active_pane == Pane::RunsList
                {
                    self.active_pane = Pane::TestResults;
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::SHIFT)
                {
                    self.scroll_logs_left();
                } else {
                    self.active_pane = match self.active_pane {
                        Pane::RunDetail | Pane::TestResults => Pane::RunsList,
                        Pane::StorageBackendDetail => Pane::StorageBackendsList,
                        Pane::WebhookDetail => Pane::WebhooksList,
                        other => other,
                    };
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::SHIFT)
                {
                    self.scroll_logs_right();
                } else {
                    self.active_pane = match self.active_pane {
                        Pane::RunsList => Pane::RunDetail,
                        Pane::StorageBackendsList => Pane::StorageBackendDetail,
                        Pane::WebhooksList => Pane::WebhookDetail,
                        other => other,
                    };
                }
            }
            KeyCode::Char('<') => {
                self.scroll_logs_left();
            }
            KeyCode::Char('>') => {
                self.scroll_logs_right();
            }
            KeyCode::Char('[') => {
                self.scroll_logs_up();
            }
            KeyCode::Char(']') => {
                self.scroll_logs_down();
            }
            KeyCode::Char('{') => {
                self.scroll_overview_up();
            }
            KeyCode::Char('}') => {
                self.scroll_overview_down();
            }
            KeyCode::PageUp => {
                for _ in 0..10 {
                    self.scroll_logs_up();
                }
            }
            KeyCode::PageDown => {
                for _ in 0..10 {
                    self.scroll_logs_down();
                }
            }
            KeyCode::Char('a') => {
                self.log_auto_scroll = !self.log_auto_scroll;
            }
            KeyCode::Char('A') if self.active_pane == Pane::RunDetail => {
                self.open_approval_dialog();
            }
            KeyCode::Char('R') if self.active_pane == Pane::RunDetail => {
                let _ = self.reject_selected_step().await;
            }
            KeyCode::Char('c')
                if self.active_pane == Pane::StorageBackendsList
                    || self.active_pane == Pane::StorageBackendDetail =>
            {
                self.open_storage_backend_dialog(false);
            }
            KeyCode::Char('c')
                if self.active_pane == Pane::WebhooksList
                    || self.active_pane == Pane::WebhookDetail =>
            {
                self.open_webhook_dialog(false);
            }
            KeyCode::Char('c') => {} // No-op if not in backends tab or webhooks tab
            KeyCode::Char('e') => {
                if self.active_pane == Pane::StorageBackendsList
                    || self.active_pane == Pane::StorageBackendDetail
                {
                    self.open_storage_backend_dialog(true);
                } else if self.active_pane == Pane::WebhooksList
                    || self.active_pane == Pane::WebhookDetail
                {
                    self.open_webhook_dialog(true);
                } else {
                    self.open_file_browser();
                }
            }
            KeyCode::Char('d')
                if self.active_pane == Pane::StorageBackendsList
                    || self.active_pane == Pane::StorageBackendDetail =>
            {
                let _ = self.delete_selected_storage_backend().await;
            }
            KeyCode::Char('d')
                if self.active_pane == Pane::WebhooksList
                    || self.active_pane == Pane::WebhookDetail =>
            {
                let _ = self.delete_selected_webhook().await;
            }
            KeyCode::Char('d') => {} // No-op if not in backends tab or webhooks tab
            KeyCode::Char('r') => {
                self.open_file_browser();
            }
            _ => {}
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use serde_json::json;
    use stormchaser_model::workflow::RunStatus;
    use tokio::sync::mpsc;

    fn setup_app() -> App<'static> {
        let (tx, _rx) = mpsc::channel(1);
        App::new("http://localhost".to_string(), None, tx)
    }

    fn mock_run_detail(id: Uuid, status: RunStatus) -> crate::app::WorkflowRunDetail {
        crate::app::WorkflowRunDetail {
            id,
            workflow_name: "test_flow".to_string(),
            initiating_user: "test_user".to_string(),
            status,
            created_at: Utc::now(),
            finished_at: None,
        }
    }

    fn mock_step_detail(name: &str, status: &str) -> crate::app::StepDetail {
        crate::app::StepDetail {
            instance: json!({"step_name": name, "status": status}),
            outputs: vec![],
            history: vec![],
            logs: vec![],
        }
    }

    fn mock_full_detail(
        id: Uuid,
        status: RunStatus,
        steps: Vec<crate::app::StepDetail>,
    ) -> crate::app::WorkflowRunFullDetail {
        crate::app::WorkflowRunFullDetail {
            detail: mock_run_detail(id, status),
            steps,
            artifacts: vec![],
            test_summaries: vec![],
            test_cases: vec![],
        }
    }

    #[test]
    fn test_handle_status_update_basic() {
        let mut app = setup_app();
        let run_id = Uuid::new_v4();

        app.runs.push(mock_run_detail(run_id, RunStatus::Queued));

        // Update to Running
        app.handle_status_update(run_id, r#"{"status": "running"}"#.to_string());
        assert_eq!(app.runs[0].status, RunStatus::Running);

        // Update to terminal shouldn't be overwritten
        app.handle_status_update(run_id, r#"{"status": "succeeded"}"#.to_string());
        assert_eq!(app.runs[0].status, RunStatus::Succeeded);

        // Try to overwrite with queued, should remain Succeeded
        app.handle_status_update(run_id, r#"{"status": "queued"}"#.to_string());
        assert_eq!(app.runs[0].status, RunStatus::Succeeded);
    }

    #[test]
    fn test_handle_workflow_update() {
        let mut app = setup_app();
        let run_id = Uuid::new_v4();

        let detail = mock_run_detail(run_id, RunStatus::Queued);
        app.handle_workflow_update(detail.clone());
        assert_eq!(app.runs.len(), 1);
        assert_eq!(app.runs[0].status, RunStatus::Queued);

        let update = mock_run_detail(run_id, RunStatus::Running);
        app.handle_workflow_update(update);
        assert_eq!(app.runs.len(), 1);
        assert_eq!(app.runs[0].status, RunStatus::Running);
    }

    #[tokio::test]
    async fn test_handle_step_update() {
        let mut app = setup_app();
        let run_id = Uuid::new_v4();

        let step = mock_step_detail("test_step", "queued");
        let detail = mock_full_detail(run_id, RunStatus::Running, vec![step]);

        app.selected_run = Some(detail);

        app.handle_step_update(run_id, "test_step".to_string(), "running".to_string());

        let updated_step = &app.selected_run.as_ref().unwrap().steps[0];
        assert_eq!(
            updated_step
                .instance
                .get("status")
                .unwrap()
                .as_str()
                .unwrap(),
            "running"
        );
    }

    #[test]
    fn test_handle_step_logs_fetched_merge_with_live() {
        let mut app = setup_app();
        let run_id = Uuid::new_v4();

        let mut step = mock_step_detail("test_step", "running");
        step.logs = vec!["live log 1".to_string(), "live log 2".to_string()];

        let detail = mock_full_detail(run_id, RunStatus::Running, vec![step]);
        app.selected_run = Some(detail);
        app.selected_step_index = 0;

        let fetched_logs = vec![
            "historic log 1".to_string(),
            "historic log 2".to_string(),
            "live log 1".to_string(), // simulate overlap
        ];

        app.handle_step_logs_fetched(run_id, 0, fetched_logs);

        let updated_step = &app.selected_run.as_ref().unwrap().steps[0];
        assert_eq!(updated_step.logs.len(), 4);
        assert_eq!(updated_step.logs[0], "historic log 1");
        assert_eq!(updated_step.logs[1], "historic log 2");
        assert_eq!(updated_step.logs[2], "live log 1");
        assert_eq!(updated_step.logs[3], "live log 2");

        assert_eq!(app.run_logs.len(), 4);
        assert_eq!(app.run_logs[3], "live log 2");
    }

    #[test]
    fn test_handle_log_line() {
        let mut app = setup_app();
        let run_id = Uuid::new_v4();

        let step = mock_step_detail("test_step", "running");
        let detail = mock_full_detail(run_id, RunStatus::Running, vec![step]);

        app.selected_run = Some(detail);
        app.selected_step_index = 0;

        app.handle_log_line(run_id, "[test_step] Hello World".to_string());

        let updated_step = &app.selected_run.as_ref().unwrap().steps[0];
        assert_eq!(updated_step.logs.len(), 1);
        assert_eq!(updated_step.logs[0], "Hello World");

        // Should also update run_logs because the step is currently selected
        assert_eq!(app.run_logs.len(), 1);
        assert_eq!(app.run_logs[0], "Hello World");
    }

    #[tokio::test]
    async fn test_handle_filter_dialog_key() {
        let mut app = setup_app();
        app.filter_dialog_active = true;

        app.handle_filter_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.filter_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_storage_backend_dialog_key() {
        let mut app = setup_app();
        app.storage_backend_dialog_active = true;

        app.handle_storage_backend_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.storage_backend_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_approval_dialog_key() {
        let mut app = setup_app();
        app.approval_dialog_active = true;

        app.handle_approval_dialog_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .await;
        assert!(!app.approval_dialog_active);
    }

    #[tokio::test]
    async fn test_handle_default_key_scroll_logs() {
        let mut app = setup_app();

        // Right / Shift+Right
        app.handle_default_key(KeyEvent::new(KeyCode::Right, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 1);

        app.handle_default_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 2);

        app.handle_default_key(KeyEvent::new(KeyCode::Char('>'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.log_scroll_x, 3);

        // Left / Shift+Left
        app.handle_default_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 2);

        app.handle_default_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::SHIFT))
            .await;
        assert_eq!(app.log_scroll_x, 1);

        app.handle_default_key(KeyEvent::new(KeyCode::Char('<'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.log_scroll_x, 0);

        // Don't underflow
        app.handle_default_key(KeyEvent::new(KeyCode::Char('<'), KeyModifiers::NONE))
            .await;
        assert_eq!(app.log_scroll_x, 0);
    }
}
