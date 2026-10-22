use crate::app::{parse_step_status, App, WorkflowRunDetail, WorkflowRunFullDetail};
use crate::AppEvent;
use chrono::Utc;
use serde_json::Value;
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;

impl<'a> App<'a> {
    /// Handles an incoming status update for a specific workflow run.
    /// Supports special "refresh" strings to trigger a full refetch.
    pub fn handle_status_update(&mut self, run_id: RunId, status: String) {
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

            if self.token.is_none() {
                return;
            }
            let api = self.api();
            let tx = self.status_tx.clone();
            tokio::spawn(async move {
                match api.get_run(run_id).await {
                    Ok(full_detail) => {
                        let _ = tx.send(AppEvent::FullRunUpdate(full_detail)).await;
                    }
                    Err(e) if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) => {
                        let _ = tx.send(AppEvent::TokenExpired).await;
                    }
                    Err(_) => {}
                }
            });
            return;
        }

        let parsed_status = if let Ok(payload) = serde_json::from_str::<Value>(&status) {
            payload.as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                payload
                    .get("status")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or(status.clone())
            })
        } else {
            status.clone()
        }
        .trim_matches('"')
        .to_string();

        if let Some(run) = self.runs.iter_mut().find(|r| r.id == run_id) {
            let s = RunStatus::from(parsed_status.clone());
            // Prevent downgrading terminal statuses
            match run.status {
                RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {
                    // Do not overwrite a terminal status with a non-terminal one
                }
                _ => run.status = s,
            }
        }
        if let Some(run) = &mut self.selected_run {
            if run.detail.id == run_id {
                let s = RunStatus::from(parsed_status);
                match run.detail.status {
                    RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {}
                    _ => run.detail.status = s,
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
                        if let Some(old_step) = old_run
                            .steps
                            .iter()
                            .find(|s| s.instance.step_name == step.instance.step_name)
                        {
                            // Preserve logs if new ones are empty
                            if step.logs.is_empty() && !old_step.logs.is_empty() {
                                step.logs = old_step.logs.clone();
                            }

                            // Simple heuristic: don't move back to unpacking/pending if already running
                            if matches!(
                                old_step.instance.status,
                                StepStatus::Running | StepStatus::Succeeded | StepStatus::Failed
                            ) && matches!(
                                step.instance.status,
                                StepStatus::UnpackingSfs | StepStatus::Pending
                            ) {
                                step.instance.status = old_step.instance.status.clone();
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
    pub fn handle_step_update(&mut self, run_id: RunId, step_name: String, status: String) {
        if let Some(run) = &mut self.selected_run {
            if run.detail.id == run_id {
                let mut updated_step_index = None;

                if let Some(new_status) = parse_step_status(&status) {
                    for (i, step_detail) in run.steps.iter_mut().enumerate() {
                        if step_detail.instance.step_name == step_name {
                            step_detail.instance.status = new_status.clone();
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
                    let is_terminal = status == "succeeded"
                        || status == "failed"
                        || status == "aborted"
                        || status == "skipped";
                    let refresh_type = if is_terminal {
                        "force_refresh"
                    } else {
                        "refresh"
                    };
                    tokio::spawn(async move {
                        let _ = tx
                            .send(AppEvent::StatusUpdate(run_id, refresh_type.to_string()))
                            .await;
                    });
                }
            }
        }
    }

    /// Appends a new log line to the appropriate step within the selected run.
    pub fn handle_step_logs_fetched(
        &mut self,
        run_id: RunId,
        step_index: usize,
        logs: Vec<String>,
    ) {
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
    pub fn handle_log_line(&mut self, run_id: RunId, line: String) {
        if let Some(run) = &mut self.selected_run {
            if run.detail.id == run_id {
                let mut step_name_and_clean_line = None;

                // Update the logs for the specific step in our model
                for step in &mut run.steps {
                    let name = &step.instance.step_name;
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
                        step_name_and_clean_line = Some((name.clone(), clean_line));
                        break;
                    }
                }

                // If the selected step was updated, update current log view
                if let Some((name, clean_line)) = step_name_and_clean_line {
                    if let Some(current_step) = run.steps.get(self.selected_step_index) {
                        if current_step.instance.step_name == name {
                            self.run_logs.push(clean_line);
                        }
                    }
                }
            }
        }
    }

    /// Handles a partial summary update for a workflow run, usually from the global run list stream.
    pub fn handle_workflow_update(&mut self, run_detail: WorkflowRunDetail) {
        let is_forced = self.force_select_run_id == Some(run_detail.id);
        let id = run_detail.id;

        if let Some(run) = self.runs.iter_mut().find(|r| r.id == run_detail.id) {
            *run = run_detail.clone();
            if is_forced {
                if let Some(pos) = self.runs.iter().position(|r| r.id == run_detail.id) {
                    self.runs_state.select(Some(pos));
                    self.force_select_run_id = None;

                    let tx = self.status_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx
                            .send(crate::AppEvent::StatusUpdate(
                                id,
                                "force_refresh".to_string(),
                            ))
                            .await;
                        let _ = tx.send(crate::AppEvent::StartWatching(id)).await;
                    });
                }
            }
        } else {
            self.runs.insert(0, run_detail.clone());
            if is_forced {
                self.runs_state.select(Some(0));
                self.force_select_run_id = None;

                let tx = self.status_tx.clone();
                tokio::spawn(async move {
                    let _ = tx
                        .send(crate::AppEvent::StatusUpdate(
                            id,
                            "force_refresh".to_string(),
                        ))
                        .await;
                    let _ = tx.send(crate::AppEvent::StartWatching(id)).await;
                });
            } else if let Some(selected) = self.runs_state.selected() {
                self.runs_state.select(Some(selected + 1));
            }
        }

        let current_selected_id = self.runs_state.selected().map(|i| self.runs[i].id);
        if Some(run_detail.id) == current_selected_id {
            let mut needs_refresh = false;
            let id = run_detail.id;
            if let Some(run) = &mut self.selected_run {
                if run.detail.id == id {
                    if run.detail.status != run_detail.status {
                        needs_refresh = true;
                    }
                    run.detail = run_detail;
                } else {
                    needs_refresh = true;
                }
            } else {
                needs_refresh = true;
            }

            if needs_refresh {
                let tx = self.status_tx.clone();
                tokio::spawn(async move {
                    let _ = tx
                        .send(crate::AppEvent::StatusUpdate(
                            id,
                            "force_refresh".to_string(),
                        ))
                        .await;
                    let _ = tx.send(crate::AppEvent::StartWatching(id)).await;
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::handlers::test_utils::{
        mock_full_detail, mock_run_detail, mock_step_detail, setup_app,
    };
    use stormchaser_model::workflow::RunStatus;
    use stormchaser_model::RunId;
    use uuid::Uuid;

    #[test]
    fn test_handle_status_update_basic() {
        let mut app = setup_app();
        let run_id = RunId::new(Uuid::new_v4());

        let run = mock_run_detail(run_id, RunStatus::Running);
        app.runs.push(run.clone());

        app.handle_status_update(run_id, "succeeded".to_string());
        assert_eq!(app.runs[0].status, RunStatus::Succeeded);
    }

    #[test]
    fn test_handle_status_update_prevent_downgrade() {
        let mut app = setup_app();
        let run_id = RunId::new(Uuid::new_v4());

        let run = mock_run_detail(run_id, RunStatus::Succeeded);
        app.runs.push(run.clone());

        app.handle_status_update(run_id, "running".to_string());
        assert_eq!(app.runs[0].status, RunStatus::Succeeded);
    }

    #[tokio::test]
    async fn test_handle_full_run_update_preserves_logs() {
        let mut app = setup_app();
        let run_id = RunId::new(Uuid::new_v4());

        let mut old_step = mock_step_detail("build", "running");
        old_step.logs = vec!["compiling...".to_string()];

        let old_detail = mock_full_detail(run_id, RunStatus::Running, vec![old_step]);
        app.runs.push(old_detail.detail.clone());
        app.selected_run = Some(old_detail);
        app.runs_state.select(Some(0));

        let new_step = mock_step_detail("build", "running");
        let new_detail = mock_full_detail(run_id, RunStatus::Running, vec![new_step]);

        app.handle_full_run_update(new_detail);

        assert_eq!(
            app.selected_run.as_ref().unwrap().steps[0].logs,
            vec!["compiling..."]
        );
    }
}
