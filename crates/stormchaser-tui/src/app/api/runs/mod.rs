use super::*;
use crate::app::WorkflowRunDetail;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::AUTHORIZATION;
use std::time::Duration;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use tokio::time::sleep;

impl<'a> App<'a> {
    /// Fetches the latest list of workflow runs from the API based on active filters.
    pub async fn refresh_runs(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        // Build query using Url to ensure filter values are properly percent-encoded.
        let base = format!("{}/api/v1/runs", self.url);
        let mut url = url::Url::parse(&base)?;
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(owner) = &self.filter_owner {
                pairs.append_pair("initiating_user", owner);
            }
            if let Some(name) = &self.filter_name {
                pairs.append_pair("workflow_name", name);
            }
            if let Some(repo) = &self.filter_repo_url {
                pairs.append_pair("repo_url", repo);
            }
            if let Some(path) = &self.filter_workflow_path {
                pairs.append_pair("workflow_path", path);
            }
            if let Some(ca) = &self.filter_created_after {
                pairs.append_pair("created_after", ca);
            }
            if let Some(cb) = &self.filter_created_before {
                pairs.append_pair("created_before", cb);
            }
            if let Some(status) = &self.filter_status {
                pairs.append_pair("status", status);
            }
        }

        let client = reqwest::Client::new();
        let mut req = client.request(reqwest::Method::GET, url);
        if let Some(token) = &self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {}", token));
        }
        let res = req.send().await?;

        if res.status().is_success() {
            self.runs = res.json::<Vec<WorkflowRunDetail>>().await?;
            if self.runs.is_empty() {
                self.runs_state.select(None);
                self.selected_run = None;
            } else {
                let mut selected_index = self
                    .runs_state
                    .selected()
                    .map_or(0, |selected| selected.min(self.runs.len() - 1));

                if let Some(forced_id) = self.force_select_run_id {
                    if let Some(idx) = self.runs.iter().position(|r| r.id == forced_id) {
                        selected_index = idx;
                        self.force_select_run_id = None; // Successfully found and selected
                    }
                }

                self.runs_state.select(Some(selected_index));

                let needs_fetch = match &self.selected_run {
                    Some(run) => {
                        if let Some(current_run) = self.runs.get(selected_index) {
                            run.detail.id != current_run.id
                        } else {
                            false
                        }
                    }
                    None => true,
                };

                if needs_fetch {
                    if let Some(run) = self.runs.get(selected_index) {
                        let id = run.id;
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

    /// Fetches the full details for a specific workflow run.
    pub async fn fetch_run_detail(&mut self, run_id: RunId) -> Result<()> {
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
            let full_detail = res.json::<crate::app::WorkflowRunFullDetail>().await?;
            self.handle_full_run_update(full_detail);
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch run detail: {}", res.status()));
        }
        Ok(())
    }

    /// Deletes the currently selected workflow run.
    pub async fn delete_selected_run(&mut self) -> Result<()> {
        if let Some(i) = self.runs_state.selected() {
            if let Some(run) = self.runs.get(i) {
                let id = run.id;
                let res = self
                    .api_request(
                        reqwest::Method::DELETE,
                        &format!("/api/v1/runs/{}", id),
                        None,
                    )
                    .await?;

                if res.status().is_success() {
                    self.error = None;
                    if !self.runs.is_empty() {
                        self.runs_state.select(Some(i.saturating_sub(1)));
                    }
                    self.selected_run = None;
                    self.selected_step_index = 0;
                    self.run_logs.clear();
                    self.refresh_runs().await?;
                } else {
                    self.error = Some(format!("Failed to delete run: {}", res.status()));
                }
            }
        }
        Ok(())
    }

    /// Approves the currently selected step in the active run.
    pub async fn approve_selected_step(&mut self, inputs: serde_json::Value) -> Result<()> {
        if let Some(run) = &self.selected_run {
            if let Some(step) = run.steps.get(self.selected_step_index) {
                let run_id = run.detail.id;
                let step_id_str = step
                    .instance
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if let Ok(step_id) = uuid::Uuid::parse_str(step_id_str).map(StepInstanceId::new) {
                    let res = self
                        .api_request(
                            reqwest::Method::POST,
                            &format!("/api/v1/runs/{}/steps/{}/approve", run_id, step_id),
                            Some(inputs.clone()),
                        )
                        .await?;

                    if res.status().is_success() {
                        self.error = None;
                        self.approval_dialog_active = false;
                        self.refresh_runs().await?;
                    } else {
                        self.error = Some(format!("Failed to approve step: {}", res.status()));
                    }
                }
            }
        }
        Ok(())
    }

    /// Rejects the currently selected step in the active run.
    pub async fn reject_selected_step(&mut self) -> Result<()> {
        if let Some(run) = &self.selected_run {
            if let Some(step) = run.steps.get(self.selected_step_index) {
                let run_id = run.detail.id;
                let step_id_str = step
                    .instance
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                if let Ok(step_id) = uuid::Uuid::parse_str(step_id_str).map(StepInstanceId::new) {
                    let res = self
                        .api_request(
                            reqwest::Method::POST,
                            &format!("/api/v1/runs/{}/steps/{}/reject", run_id, step_id),
                            None,
                        )
                        .await?;

                    if res.status().is_success() {
                        self.error = None;
                        self.approval_dialog_active = false;
                        self.refresh_runs().await?;
                    } else {
                        self.error = Some(format!("Failed to reject step: {}", res.status()));
                    }
                }
            }
        }
        Ok(())
    }

    /// Starts a background task to listen for global workflow run updates via SSE.
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
                        .header(AUTHORIZATION, format!("Bearer {}", token))
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
                                    match serde_json::from_str::<WorkflowRunDetail>(&event.data) {
                                        Ok(run) => {
                                            let _ = tx.send(AppEvent::WorkflowUpdate(run)).await;
                                        }
                                        Err(_e) => {
                                            // Handle error or ignore
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                sleep(Duration::from_secs(2)).await;
            }
        });
        self.workflow_handle = Some(workflow_handle);
    }
}

#[cfg(test)]
mod tests;
