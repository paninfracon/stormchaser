use ratatui_textarea::TextArea;
impl<'a> crate::app::App<'a> {
    /// Opens the cron workflow create/edit dialog.
    pub fn open_cron_dialog(&mut self, edit: bool) {
        self.cron_dialog_active = true;
        self.cron_focus = 0;

        if edit {
            if let Some(cron) = &self.selected_cron_workflow {
                self.cron_edit_id = Some(cron.id);
                self.cron_inputs = vec![
                    TextArea::from(vec![cron.name.clone()]),
                    TextArea::from(
                        cron.description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    TextArea::from(vec![cron.cronspec.clone()]),
                    TextArea::from(vec![cron.workflow_name.clone()]),
                    TextArea::from(vec![cron.repo_url.clone()]),
                    TextArea::from(vec![cron.workflow_path.clone()]),
                    TextArea::from(vec![cron.git_ref.clone()]),
                    TextArea::from(
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
            TextArea::default(),                    // Name
            TextArea::default(),                    // Description
            TextArea::default(),                    // Cron Spec
            TextArea::default(),                    // Workflow Name
            TextArea::default(),                    // Repo URL
            TextArea::default(),                    // Workflow Path
            TextArea::default(),                    // Git Ref
            TextArea::from(vec!["{}".to_string()]), // Inputs
        ];
        self.cron_is_active = true;
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::app::App;
    use crate::app::CronWorkflowId;
    use stormchaser_model::cron::CronWorkflow;

    fn setup_app() -> App<'static> {
        let (tx, _) = tokio::sync::mpsc::channel(1);
        App::new(
            "http://test".to_string(),
            "http://test".to_string(),
            Some("token".to_string()),
            tx,
        )
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        app.selected_cron_workflow = Some(cron.clone());

        app.open_cron_dialog(true);
        assert!(app.cron_dialog_active);
        assert_eq!(app.cron_edit_id, Some(cron.id));
        assert_eq!(app.cron_inputs[0].lines()[0], "cron1");
        assert!(!app.cron_is_active);
    }
}
