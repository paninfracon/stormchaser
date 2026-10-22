use ratatui_textarea::TextArea;

/// Shown when the user asks to edit a cron schedule: the API has no route to change one.
pub const CRON_EDIT_UNSUPPORTED: &str =
    "The API cannot change a cron schedule yet: delete it and create a new one.";

impl<'a> crate::app::App<'a> {
    /// Opens the dialog to create a cron workflow.
    pub fn open_cron_dialog(&mut self) {
        self.cron_dialog_active = true;
        self.cron_focus = 0;
        self.cron_inputs = vec![
            TextArea::default(),                    // Name
            TextArea::default(),                    // Description
            TextArea::default(),                    // Cron Spec
            TextArea::default(),                    // Workflow Name
            TextArea::default(),                    // Git Connection
            TextArea::default(),                    // Workflow Path
            TextArea::default(),                    // Git Ref
            TextArea::from(vec!["{}".to_string()]), // Inputs
        ];
        self.cron_is_active = true;
    }

    /// Tells the user an existing cron schedule cannot be edited.
    pub fn refuse_cron_edit(&mut self) {
        if self.selected_cron_workflow.is_some() {
            self.error = Some(CRON_EDIT_UNSUPPORTED.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CRON_EDIT_UNSUPPORTED;
    use chrono::Utc;

    use crate::app::App;
    use stormchaser_model::cron::CronWorkflow;
    use stormchaser_model::CronWorkflowId;

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
        app.open_cron_dialog();
        assert!(app.cron_dialog_active);
        assert_eq!(app.cron_inputs.len(), 8);
    }

    #[test]
    fn test_editing_a_cron_workflow_says_the_api_cannot() {
        let mut app = setup_app();
        app.selected_cron_workflow = Some(CronWorkflow {
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
        });

        app.refuse_cron_edit();
        assert!(!app.cron_dialog_active);
        assert_eq!(app.error.as_deref(), Some(CRON_EDIT_UNSUPPORTED));
    }
}
