use super::*;
use anyhow::Result;
use stormchaser_model::storage::BackendType;

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

    /// Opens the storage backend create/edit dialog.
    pub fn open_storage_backend_dialog(&mut self, edit: bool) {
        self.storage_backend_dialog_active = true;
        self.storage_backend_focus = 0;

        if edit {
            if let Some(backend) = &self.selected_storage_backend {
                self.storage_backend_edit_id = Some(backend.id);
                self.storage_backend_inputs = vec![
                    ratatui_textarea::TextArea::from(vec![backend.name.clone()]),
                    ratatui_textarea::TextArea::from(
                        backend
                            .description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    ratatui_textarea::TextArea::from(
                        serde_json::to_string_pretty(&backend.config)
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    ratatui_textarea::TextArea::from(vec![backend
                        .aws_assume_role_arn
                        .clone()
                        .unwrap_or_default()]),
                ];
                let type_str = match backend.backend_type {
                    BackendType::S3 => "S3",
                    BackendType::Oci => "Oci",
                    BackendType::Jfrog => "Jfrog",
                    BackendType::Gcs => "Gcs",
                    BackendType::Azure => "Azure",
                };
                self.storage_backend_type_index = crate::app::BACKEND_TYPE_OPTIONS
                    .iter()
                    .position(|&s| s == type_str)
                    .unwrap_or(0);
                self.storage_backend_is_default = backend.is_default_sfs;
                return;
            }
        }

        self.storage_backend_edit_id = None;
        self.storage_backend_inputs = vec![
            ratatui_textarea::TextArea::default(), // name
            ratatui_textarea::TextArea::default(), // description
            ratatui_textarea::TextArea::from(vec!["{}".to_string()]), // config
            ratatui_textarea::TextArea::default(), // assume role arn
        ];
        self.storage_backend_type_index = 0;
        self.storage_backend_is_default = false;
    }

    /// Opens the webhook create/edit dialog.
    pub fn open_webhook_dialog(&mut self, edit: bool) {
        self.webhook_dialog_active = true;
        self.webhook_focus = 0;

        if edit {
            if let Some(webhook) = &self.selected_webhook {
                self.webhook_edit_id = Some(webhook.id);
                self.webhook_inputs = vec![
                    ratatui_textarea::TextArea::from(vec![webhook.name.clone()]),
                    ratatui_textarea::TextArea::from(
                        webhook
                            .description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    ratatui_textarea::TextArea::from(
                        webhook
                            .secret_token
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                ];
                let type_str = webhook.source_type.as_str();
                self.webhook_source_type_index = crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS
                    .iter()
                    .position(|&s| s == type_str)
                    .unwrap_or(0);
                self.webhook_is_active = webhook.is_active;
                return;
            }
        }

        self.webhook_edit_id = None;
        self.webhook_inputs = vec![
            ratatui_textarea::TextArea::default(), // name
            ratatui_textarea::TextArea::default(), // description
            ratatui_textarea::TextArea::default(), // secret token
        ];
        self.webhook_source_type_index = 0;
        self.webhook_is_active = true;
    }

    /// Opens the step approval dialog.
    pub fn open_approval_dialog(&mut self) {
        self.approval_dialog_active = true;
        self.approval_inputs = ratatui_textarea::TextArea::from(vec!["{}".to_string()]);
    }

    /// Opens the delete run dialog.
    pub fn open_delete_run_dialog(&mut self) {
        if self.runs_state.selected().is_some() {
            self.delete_run_dialog_active = true;
        }
    }

    /// Opens the event rule create/edit dialog.
    pub fn open_event_rule_dialog(&mut self, edit: bool) {
        self.event_rule_dialog_active = true;
        self.event_rule_focus = 0;

        if edit {
            if let Some(rule) = &self.selected_event_rule {
                self.event_rule_edit_id = Some(rule.id);
                self.event_rule_inputs = vec![
                    ratatui_textarea::TextArea::from(vec![rule.name.clone()]),
                    ratatui_textarea::TextArea::from(
                        rule.description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    ratatui_textarea::TextArea::from(
                        rule.webhook_id
                            .map(|id| id.to_string())
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    ratatui_textarea::TextArea::from(vec![rule.event_type_pattern.clone()]),
                    ratatui_textarea::TextArea::from(
                        rule.condition_expr
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    ratatui_textarea::TextArea::from(vec![rule.workflow_name.clone()]),
                    ratatui_textarea::TextArea::from(vec![rule.repo_url.clone()]),
                    ratatui_textarea::TextArea::from(vec![rule.workflow_path.clone()]),
                    ratatui_textarea::TextArea::from(vec![rule.git_ref.clone()]),
                    ratatui_textarea::TextArea::from(
                        serde_json::to_string_pretty(&rule.input_mappings)
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                ];
                self.event_rule_is_active = rule.is_active;
                return;
            }
        }

        self.event_rule_edit_id = None;
        self.event_rule_inputs = vec![
            ratatui_textarea::TextArea::default(), // Name
            ratatui_textarea::TextArea::default(), // Description
            ratatui_textarea::TextArea::default(), // Webhook ID
            ratatui_textarea::TextArea::default(), // Event Type Pattern
            ratatui_textarea::TextArea::default(), // Condition Expr
            ratatui_textarea::TextArea::default(), // Workflow Name
            ratatui_textarea::TextArea::default(), // Repo URL
            ratatui_textarea::TextArea::default(), // Workflow Path
            ratatui_textarea::TextArea::default(), // Git Ref
            ratatui_textarea::TextArea::from(vec!["{}".to_string()]), // Input Mappings
        ];
        self.event_rule_is_active = true;
    }

    /// Opens the cron workflow create/edit dialog.
    pub fn open_cron_dialog(&mut self, edit: bool) {
        self.cron_dialog_active = true;
        self.cron_focus = 0;

        if edit {
            if let Some(cron) = &self.selected_cron_workflow {
                self.cron_edit_id = Some(cron.id);
                self.cron_inputs = vec![
                    ratatui_textarea::TextArea::from(vec![cron.name.clone()]),
                    ratatui_textarea::TextArea::from(
                        cron.description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    ratatui_textarea::TextArea::from(vec![cron.cronspec.clone()]),
                    ratatui_textarea::TextArea::from(vec![cron.workflow_name.clone()]),
                    ratatui_textarea::TextArea::from(vec![cron.repo_url.clone()]),
                    ratatui_textarea::TextArea::from(vec![cron.workflow_path.clone()]),
                    ratatui_textarea::TextArea::from(vec![cron.git_ref.clone()]),
                    ratatui_textarea::TextArea::from(
                        serde_json::to_string_pretty(&cron.inputs)
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                ];
                self.cron_is_active = cron.is_active;
                return;
            }
        }

        self.cron_edit_id = None;
        self.cron_inputs = vec![
            ratatui_textarea::TextArea::default(), // Name
            ratatui_textarea::TextArea::default(), // Description
            ratatui_textarea::TextArea::default(), // Cron Spec
            ratatui_textarea::TextArea::default(), // Workflow Name
            ratatui_textarea::TextArea::default(), // Repo URL
            ratatui_textarea::TextArea::default(), // Workflow Path
            ratatui_textarea::TextArea::default(), // Git Ref
            ratatui_textarea::TextArea::from(vec!["{}".to_string()]), // Inputs
        ];
        self.cron_is_active = true;
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
                    let mut properties = serde_json::Map::new();
                    for input in workflow.inputs {
                        let mut prop = serde_json::Map::new();
                        prop.insert("type".to_string(), serde_json::json!("string"));
                        prop.insert(
                            "description".to_string(),
                            serde_json::json!(input.description.as_deref().unwrap_or(&input.name)),
                        );
                        if let Some(default) = &input.default {
                            if let Some(s) = default.as_str() {
                                prop.insert("default".to_string(), serde_json::json!(s));
                            } else {
                                prop.insert(
                                    "default".to_string(),
                                    serde_json::json!(default.to_string()),
                                );
                            }
                        }
                        properties.insert(input.name, serde_json::Value::Object(prop));
                    }
                    let schema = serde_json::json!({
                        "type": "object",
                        "title": "Workflow Inputs",
                        "properties": properties
                    });
                    self.pending_schema_ui = Some((schema, dsl));
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
    pub async fn submit_direct_form(&mut self, inputs_json: serde_json::Value) -> Result<()> {
        if let Some(dsl) = self.direct_submit_dsl.take() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::cron::CronWorkflow;
    use stormchaser_model::event_rules::{EventRule, WebhookConfig};
    use stormchaser_model::storage::StorageBackend;

    fn setup_app() -> App<'static> {
        let (tx, _) = tokio::sync::mpsc::channel(1);
        App::new("http://test".to_string(), Some("token".to_string()), tx)
    }

    #[test]
    fn test_open_filter_dialog() {
        let mut app = setup_app();
        app.filter_owner = Some("owner".to_string());
        app.filter_status = Some("failed".to_string());

        app.open_filter_dialog();

        assert!(app.filter_dialog_active);
        assert_eq!(app.filter_focus, 0);
        assert_eq!(app.filter_inputs.len(), 6);
        assert_eq!(app.filter_inputs[0].lines()[0], "owner");
        assert_eq!(app.filter_status_index, 5); // Failed is index 5 in FILTER_STATUS_OPTIONS
    }

    #[test]
    fn test_open_file_browser() {
        let mut app = setup_app();
        app.open_file_browser();
        assert!(app.file_browser_active);
    }

    #[test]
    fn test_open_schedule_git_dialog() {
        let mut app = setup_app();
        app.open_schedule_git_dialog();
        assert!(app.schedule_git_dialog_active);
        assert_eq!(app.schedule_git_focus, 0);
        assert_eq!(app.schedule_git_inputs.len(), 3);
    }

    #[test]
    fn test_open_storage_backend_dialog_new() {
        let mut app = setup_app();
        app.open_storage_backend_dialog(false);
        assert!(app.storage_backend_dialog_active);
        assert_eq!(app.storage_backend_edit_id, None);
        assert_eq!(app.storage_backend_inputs.len(), 4);
    }

    #[test]
    fn test_open_storage_backend_dialog_edit() {
        let mut app = setup_app();
        let backend = StorageBackend {
            id: BackendId::new_v4(),
            name: "test_backend".to_string(),
            description: Some("desc".to_string()),
            backend_type: BackendType::S3,
            is_default_sfs: true,
            config: serde_json::json!({"region": "us-east-1"}),
            aws_assume_role_arn: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        app.selected_storage_backend = Some(backend.clone());

        app.open_storage_backend_dialog(true);
        assert!(app.storage_backend_dialog_active);
        assert_eq!(app.storage_backend_edit_id, Some(backend.id));
        assert_eq!(app.storage_backend_inputs[0].lines()[0], "test_backend");
        assert_eq!(app.storage_backend_type_index, 0);
        assert!(app.storage_backend_is_default);
    }

    #[test]
    fn test_open_webhook_dialog_new() {
        let mut app = setup_app();
        app.open_webhook_dialog(false);
        assert!(app.webhook_dialog_active);
        assert_eq!(app.webhook_edit_id, None);
        assert_eq!(app.webhook_inputs.len(), 3);
    }

    #[test]
    fn test_open_webhook_dialog_edit() {
        let mut app = setup_app();
        let webhook = WebhookConfig {
            id: WebhookId::new_v4(),
            name: "hook".to_string(),
            description: Some("desc".to_string()),
            source_type: "github".to_string(),
            is_active: false,
            secret_token: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        app.selected_webhook = Some(webhook.clone());

        app.open_webhook_dialog(true);
        assert!(app.webhook_dialog_active);
        assert_eq!(app.webhook_edit_id, Some(webhook.id));
        assert_eq!(app.webhook_inputs[0].lines()[0], "hook");
        assert_eq!(app.webhook_source_type_index, 0); // github
        assert!(!app.webhook_is_active);
    }

    #[test]
    fn test_open_event_rule_dialog_new() {
        let mut app = setup_app();
        app.open_event_rule_dialog(false);
        assert!(app.event_rule_dialog_active);
        assert_eq!(app.event_rule_edit_id, None);
        assert_eq!(app.event_rule_inputs.len(), 10);
    }

    #[test]
    fn test_open_event_rule_dialog_edit() {
        let mut app = setup_app();
        let rule = EventRule {
            id: RuleId::new_v4(),
            name: "rule1".to_string(),
            description: None,
            webhook_id: None,
            event_type_pattern: "push".to_string(),
            condition_expr: None,
            workflow_name: "wf1".to_string(),
            repo_url: "http".to_string(),
            workflow_path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            input_mappings: serde_json::json!({}),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        app.selected_event_rule = Some(rule.clone());

        app.open_event_rule_dialog(true);
        assert!(app.event_rule_dialog_active);
        assert_eq!(app.event_rule_edit_id, Some(rule.id));
        assert_eq!(app.event_rule_inputs[0].lines()[0], "rule1");
        assert!(app.event_rule_is_active);
    }

    #[test]
    fn test_open_cron_dialog_new() {
        let mut app = setup_app();
        app.open_cron_dialog(false);
        assert!(app.cron_dialog_active);
        assert_eq!(app.cron_edit_id, None);
        assert_eq!(app.cron_inputs.len(), 8);
    }

    #[test]
    fn test_open_cron_dialog_edit() {
        let mut app = setup_app();
        let cron = CronWorkflow {
            id: CronWorkflowId::new_v4(),
            name: "cron1".to_string(),
            description: None,
            cronspec: "* * * * *".to_string(),
            workflow_name: "wf1".to_string(),
            repo_url: "http".to_string(),
            workflow_path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            inputs: serde_json::json!({}),
            secret_token: "".to_string(),
            is_active: false,
            external_job_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        app.selected_cron_workflow = Some(cron.clone());

        app.open_cron_dialog(true);
        assert!(app.cron_dialog_active);
        assert_eq!(app.cron_edit_id, Some(cron.id));
        assert_eq!(app.cron_inputs[0].lines()[0], "cron1");
        assert!(!app.cron_is_active);
    }
}
