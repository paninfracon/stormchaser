use ratatui_textarea::TextArea;

/// Shown when the user asks to edit an event rule: the API has no route to change one.
pub const EVENT_RULE_EDIT_UNSUPPORTED: &str =
    "The API cannot change an event rule yet: delete it and create a new one.";

impl<'a> crate::app::App<'a> {
    /// Opens the dialog to create an event rule.
    pub fn open_event_rule_dialog(&mut self) {
        self.event_rule_dialog_active = true;
        self.event_rule_focus = 0;
        self.event_rule_inputs = vec![
            TextArea::default(),                    // Name
            TextArea::default(),                    // Description
            TextArea::default(),                    // Webhook ID
            TextArea::default(),                    // Event Type Pattern
            TextArea::default(),                    // Condition Expr
            TextArea::default(),                    // Workflow Name
            TextArea::default(),                    // Git Connection
            TextArea::default(),                    // Workflow Path
            TextArea::default(),                    // Git Ref
            TextArea::from(vec!["{}".to_string()]), // Input Mappings
        ];
        self.event_rule_is_active = true;
    }

    /// Tells the user an existing event rule cannot be edited.
    pub fn refuse_event_rule_edit(&mut self) {
        if self.selected_event_rule.is_some() {
            self.error = Some(EVENT_RULE_EDIT_UNSUPPORTED.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EVENT_RULE_EDIT_UNSUPPORTED;
    use chrono::Utc;

    use crate::app::App;
    use stormchaser_model::event_rules::EventRule;
    use stormchaser_model::RuleId;

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
    fn test_open_event_rule_dialog_new() {
        let mut app = setup_app();
        app.open_event_rule_dialog();
        assert!(app.event_rule_dialog_active);
        assert_eq!(app.event_rule_inputs.len(), 10);
    }

    #[test]
    fn test_editing_an_event_rule_says_the_api_cannot() {
        let mut app = setup_app();
        app.selected_event_rule = Some(EventRule {
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });

        app.refuse_event_rule_edit();
        assert!(!app.event_rule_dialog_active);
        assert_eq!(app.error.as_deref(), Some(EVENT_RULE_EDIT_UNSUPPORTED));
    }
}
