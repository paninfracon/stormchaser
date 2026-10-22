use super::*;
use std::collections::HashMap;
use stormchaser_model::api::CreateEventRuleRequest;
use stormchaser_model::WebhookId;

impl<'a> App<'a> {
    /// Fetches the latest list of event rules from the API.
    pub async fn refresh_event_rules(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        match self.api().list_event_rules().await {
            Ok(rules) => {
                self.event_rules = rules;
                if !self.event_rules.is_empty() {
                    if self.event_rules_state.selected().is_none() {
                        self.event_rules_state.select(Some(0));
                    }
                    if self.selected_event_rule.is_none() {
                        if let Some(i) = self.event_rules_state.selected() {
                            self.selected_event_rule = Some(self.event_rules[i].clone());
                        }
                    }
                } else {
                    self.event_rules_state.select(None);
                    self.selected_event_rule = None;
                }
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to fetch event rules: {}", refused(e)?));
            }
        }
        Ok(())
    }

    /// Submits the event rule form, creating a rule. The API cannot change a rule.
    pub async fn submit_event_rule_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let field = |i: usize| self.event_rule_inputs[i].lines().join("\n");
        let name = field(0);
        let description = field(1);
        let webhook_id = field(2);
        let event_type_pattern = field(3);
        let condition_expr = field(4);
        let workflow_name = field(5);
        let connection = field(6);
        let workflow_path = field(7);
        let git_ref = field(8);
        let input_mappings_str = field(9);

        if name.trim().is_empty()
            || webhook_id.trim().is_empty()
            || event_type_pattern.trim().is_empty()
            || workflow_name.trim().is_empty()
            || connection.trim().is_empty()
            || workflow_path.trim().is_empty()
            || git_ref.trim().is_empty()
        {
            self.error = Some("Required fields are missing.".to_string());
            return Ok(());
        }

        let input_mappings: HashMap<String, String> = if input_mappings_str.trim().is_empty() {
            HashMap::new()
        } else {
            match serde_json::from_str(&input_mappings_str) {
                Ok(v) => v,
                Err(_) => {
                    self.error =
                        Some("Input mappings must be a JSON object of strings.".to_string());
                    return Ok(());
                }
            }
        };

        let webhook_id = match uuid::Uuid::parse_str(webhook_id.trim()).map(WebhookId::new) {
            Ok(id) => id,
            Err(_) => {
                self.error = Some("Invalid Webhook ID format".to_string());
                return Ok(());
            }
        };

        let request = CreateEventRuleRequest {
            name,
            description: if description.trim().is_empty() {
                None
            } else {
                Some(description)
            },
            webhook_id,
            event_type_pattern,
            condition_expr: if condition_expr.trim().is_empty() {
                None
            } else {
                Some(condition_expr)
            },
            workflow_name,
            connection,
            workflow_path,
            git_ref,
            input_mappings,
        };

        match self.api().create_event_rule(&request).await {
            Ok(_) => {
                self.event_rule_dialog_active = false;
                self.refresh_event_rules().await?;
            }
            Err(e) => {
                self.error = Some(format!("Failed to save event rule: {}", refused(e)?));
            }
        }

        Ok(())
    }

    /// Deletes the currently selected event rule.
    pub async fn delete_selected_event_rule(&mut self) -> Result<()> {
        if let Some(rule) = &self.selected_event_rule {
            if self.token.is_some() {
                match self.api().delete_event_rule(rule.id).await {
                    Ok(()) => self.refresh_event_rules().await?,
                    Err(e) => {
                        self.error = Some(format!("Failed to delete event rule: {}", refused(e)?));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ratatui_textarea::TextArea;
    use stormchaser_model::event_rules::EventRule;
    use stormchaser_model::RuleId;
    use tokio::sync::mpsc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_event_rule(id: RuleId) -> EventRule {
        EventRule {
            id,
            name: "test-rule".to_string(),
            description: None,
            webhook_id: None,
            event_type_pattern: "push".to_string(),
            condition_expr: None,
            workflow_name: "test".to_string(),
            repo_url: "https://github.com/org/repo".to_string(),
            workflow_path: "workflow.storm".to_string(),
            git_ref: "main".to_string(),
            input_mappings: serde_json::json!({}),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_refresh_event_rules_uses_correct_path() {
        let server = MockServer::start().await;
        let id = RuleId::new_v4();

        Mock::given(method("GET"))
            .and(path("/api/v1/rules"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![make_event_rule(id)]))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );

        let result = app.refresh_event_rules().await;
        result.unwrap();
        assert_eq!(app.event_rules.len(), 1);
        assert_eq!(app.event_rules[0].id, id);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_delete_event_rule_uses_correct_path() {
        let server = MockServer::start().await;
        let id = RuleId::new_v4();

        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/rules/{}", id)))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/rules"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<EventRule>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );
        app.selected_event_rule = Some(make_event_rule(id));

        let result = app.delete_selected_event_rule().await;
        result.unwrap();
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_submit_event_rule_form_creates_a_rule() {
        let server = MockServer::start().await;
        let webhook_id = WebhookId::new_v4();

        Mock::given(method("POST"))
            .and(path("/api/v1/rules"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "webhook_id": webhook_id,
                "connection": "repo",
                "input_mappings": { "sha": "event.after" },
            })))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(serde_json::json!({ "id": RuleId::new_v4() })),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/rules"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<EventRule>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );
        app.open_event_rule_dialog();
        let values = [
            "rule",
            "",
            &webhook_id.to_string(),
            "push",
            "",
            "wf",
            "repo",
            "wf.storm",
            "main",
            r#"{"sha": "event.after"}"#,
        ];
        app.event_rule_inputs = values
            .iter()
            .map(|v| TextArea::from(vec![v.to_string()]))
            .collect();

        app.submit_event_rule_form().await.unwrap();
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.event_rule_dialog_active);
    }
}
