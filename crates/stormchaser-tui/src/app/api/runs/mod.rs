use super::*;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use std::time::Duration;
use stormchaser_model::api::ListRunsQuery;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use tokio::time::sleep;

/// The run status a filter names, such as `failed` or `Failed`.
fn parse_run_status(name: &str) -> Option<RunStatus> {
    serde_json::from_value(Value::String(name.to_lowercase())).ok()
}

/// The time a filter gives, as RFC 3339.
fn parse_time(time: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(time)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

impl<'a> App<'a> {
    /// The query the active filters select, or a message saying which filter is not valid.
    fn runs_query(&self) -> Result<ListRunsQuery, String> {
        let status = match &self.filter_status {
            Some(name) => {
                Some(parse_run_status(name).ok_or(format!("Unknown status filter: {name}"))?)
            }
            None => None,
        };
        let time = |filter: &Option<String>| match filter {
            Some(time) => parse_time(time)
                .map(Some)
                .ok_or(format!("Not an RFC 3339 time: {time}")),
            None => Ok(None),
        };
        Ok(ListRunsQuery {
            workflow_name: self.filter_name.clone(),
            status,
            initiating_user: self.filter_owner.clone(),
            repo_url: self.filter_repo_url.clone(),
            workflow_path: self.filter_workflow_path.clone(),
            created_after: time(&self.filter_created_after)?,
            created_before: time(&self.filter_created_before)?,
            limit: None,
            offset: None,
        })
    }

    /// Fetches the latest list of workflow runs from the API based on active filters.
    pub async fn refresh_runs(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let query = match self.runs_query() {
            Ok(query) => query,
            Err(message) => {
                self.error = Some(message);
                return Ok(());
            }
        };

        match self.api().list_runs(&query).await {
            Ok(runs) => {
                self.runs = runs;
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
            }
            Err(e) => {
                self.error = Some(format!("Failed to fetch runs: {}", refused(e)?));
            }
        }
        Ok(())
    }

    /// Fetches the list of pending approvals (runs with status Running).
    pub async fn refresh_pending_approvals(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let query = ListRunsQuery {
            status: Some(RunStatus::Running),
            ..Default::default()
        };
        match self.api().list_runs(&query).await {
            Ok(runs) => {
                self.pending_approvals = runs;
                if self.pending_approvals.is_empty() {
                    self.pending_approvals_state.select(None);
                } else {
                    let selected_index = self
                        .pending_approvals_state
                        .selected()
                        .map_or(0, |s| s.min(self.pending_approvals.len() - 1));
                    self.pending_approvals_state.select(Some(selected_index));
                }
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!(
                    "Failed to fetch pending approvals: {}",
                    refused(e)?
                ));
            }
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

        match self.api().get_run(run_id).await {
            Ok(full_detail) => {
                self.handle_full_run_update(full_detail);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to fetch run detail: {}", refused(e)?));
            }
        }
        Ok(())
    }

    /// Deletes the currently selected workflow run.
    pub async fn delete_selected_run(&mut self) -> Result<()> {
        if let Some(i) = self.runs_state.selected() {
            if let Some(run) = self.runs.get(i) {
                match self.api().delete_run(run.id).await {
                    Ok(()) => {
                        self.error = None;
                        if !self.runs.is_empty() {
                            self.runs_state.select(Some(i.saturating_sub(1)));
                        }
                        self.selected_run = None;
                        self.selected_step_index = 0;
                        self.run_logs.clear();
                        self.refresh_runs().await?;
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to delete run: {}", refused(e)?));
                    }
                }
            }
        }
        Ok(())
    }

    /// Approves the currently selected step in the active run.
    pub async fn approve_selected_step(&mut self, inputs: Value) -> Result<()> {
        if let Some(run) = &self.selected_run {
            if let Some(step) = run.steps.get(self.selected_step_index) {
                let result = self
                    .api()
                    .approve(run.detail.id, step.instance.id, &inputs)
                    .await;
                match result {
                    Ok(()) => {
                        self.error = None;
                        self.approval_dialog_active = false;
                        self.refresh_runs().await?;
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to approve step: {}", refused(e)?));
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
                match self.api().reject(run.detail.id, step.instance.id).await {
                    Ok(()) => {
                        self.error = None;
                        self.approval_dialog_active = false;
                        self.refresh_runs().await?;
                    }
                    Err(e) => {
                        self.error = Some(format!("Failed to reject step: {}", refused(e)?));
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
        let tx = self.status_tx.clone();
        let api = self.token.is_some().then(|| self.api());

        let workflow_handle = tokio::spawn(async move {
            loop {
                if let Some(api) = &api {
                    match api.stream_runs().await {
                        Ok(stream) => {
                            let mut stream = std::pin::pin!(stream);
                            while let Some(event) = stream.next().await {
                                let Ok(event) = event else { continue };
                                if event.event == "workflow_run" {
                                    if let Ok(run) =
                                        serde_json::from_str::<WorkflowRunDetail>(&event.data)
                                    {
                                        let _ = tx.send(AppEvent::WorkflowUpdate(run)).await;
                                    }
                                }
                            }
                        }
                        Err(e) if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) => {
                            let _ = tx.send(AppEvent::TokenExpired).await;
                            return;
                        }
                        Err(_) => {}
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
