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
