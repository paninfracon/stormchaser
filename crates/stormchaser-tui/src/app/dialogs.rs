use super::*;
use anyhow::Result;

use stormchaser_dsl::StormchaserParser;

impl<'a> App<'a> {
    /// Opens the filter dialog and initializes input fields with current values.
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

    /// Opens the file browser for selecting a local `.storm` file.
    pub fn open_file_browser(&mut self) {
        self.file_browser_active = true;
    }

    /// Opens the dialog to schedule a workflow from a Git repository.
    pub fn open_schedule_git_dialog(&mut self) {
        self.schedule_git_dialog_active = true;
        self.schedule_git_focus = 0;
        self.schedule_git_inputs = vec![
            ratatui_textarea::TextArea::default(), // repo_url
            ratatui_textarea::TextArea::default(), // workflow_path
            ratatui_textarea::TextArea::default(), // git_ref
        ];
    }

    /// Submits the data from the schedule git dialog to start a workflow run.
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

    /// Handles the submission of a selected local `.storm` file, optionally prompting for inputs.
    pub async fn submit_file(&mut self) -> Result<()> {
        let path = self
            .file_explorer
            .current_entry()
            .map(|e| e.path.clone())
            .unwrap_or_default();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("storm") {
            let dsl = std::fs::read_to_string(path)?;

            if let Ok(workflow) = StormchaserParser.parse(&dsl) {
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

    /// Submits the dynamically generated form with inputs for a local workflow file.
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

    /// Applies the values from the filter dialog inputs to the active filters and refreshes runs.
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
}
