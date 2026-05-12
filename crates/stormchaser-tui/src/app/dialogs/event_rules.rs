use ratatui_textarea::TextArea;
impl<'a> crate::app::App<'a> {
    /// Opens the event rule create/edit dialog.
    pub fn open_event_rule_dialog(&mut self, edit: bool) {
        self.event_rule_dialog_active = true;
        self.event_rule_focus = 0;

        if edit {
            if let Some(rule) = &self.selected_event_rule {
                self.event_rule_edit_id = Some(rule.id);
                self.event_rule_inputs = vec![
                    TextArea::from(vec![rule.name.clone()]),
                    TextArea::from(
                        rule.description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    TextArea::from(
                        rule.webhook_id
                            .map(|id| id.to_string())
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    TextArea::from(vec![rule.event_type_pattern.clone()]),
                    TextArea::from(
                        rule.condition_expr
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    TextArea::from(vec![rule.workflow_name.clone()]),
                    TextArea::from(vec![rule.repo_url.clone()]),
                    TextArea::from(vec![rule.workflow_path.clone()]),
                    TextArea::from(vec![rule.git_ref.clone()]),
                    TextArea::from(
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
            TextArea::default(),                    // Name
            TextArea::default(),                    // Description
            TextArea::default(),                    // Webhook ID
            TextArea::default(),                    // Event Type Pattern
            TextArea::default(),                    // Condition Expr
            TextArea::default(),                    // Workflow Name
            TextArea::default(),                    // Repo URL
            TextArea::default(),                    // Workflow Path
            TextArea::default(),                    // Git Ref
            TextArea::from(vec!["{}".to_string()]), // Input Mappings
        ];
        self.event_rule_is_active = true;
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::app::App;
    use crate::app::RuleId;
    use stormchaser_model::event_rules::EventRule;

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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        app.selected_event_rule = Some(rule.clone());

        app.open_event_rule_dialog(true);
        assert!(app.event_rule_dialog_active);
        assert_eq!(app.event_rule_edit_id, Some(rule.id));
        assert_eq!(app.event_rule_inputs[0].lines()[0], "rule1");
        assert!(app.event_rule_is_active);
    }
}
