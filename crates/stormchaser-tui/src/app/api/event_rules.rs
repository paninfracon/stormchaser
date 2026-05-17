use super::*;
use stormchaser_model::event_rules::EventRule;
use stormchaser_model::WebhookId;

impl<'a> App<'a> {
    /// Fetches the latest list of event rules from the API.
    pub async fn refresh_event_rules(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let res = self
            .api_request(reqwest::Method::GET, "/api/v1/rules", None)
            .await?;

        if res.status().is_success() {
            self.event_rules = res.json::<Vec<EventRule>>().await?;
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
        } else {
            self.error = Some(format!("Failed to fetch event rules: {}", res.status()));
        }
        Ok(())
    }

    /// Submits the event rule form for creation or update.
    pub async fn submit_event_rule_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let name = self.event_rule_inputs[0].lines().join("\n");
        let description = self.event_rule_inputs[1].lines().join("\n");
        let webhook_id = self.event_rule_inputs[2].lines().join("\n");
        let event_type_pattern = self.event_rule_inputs[3].lines().join("\n");
        let condition_expr = self.event_rule_inputs[4].lines().join("\n");
        let workflow_name = self.event_rule_inputs[5].lines().join("\n");
        let repo_url = self.event_rule_inputs[6].lines().join("\n");
        let workflow_path = self.event_rule_inputs[7].lines().join("\n");
        let git_ref = self.event_rule_inputs[8].lines().join("\n");
        let input_mappings_str = self.event_rule_inputs[9].lines().join("\n");

        if name.trim().is_empty()
            || event_type_pattern.trim().is_empty()
            || workflow_name.trim().is_empty()
            || repo_url.trim().is_empty()
            || workflow_path.trim().is_empty()
            || git_ref.trim().is_empty()
        {
            self.error = Some("Required fields are missing.".to_string());
            return Ok(());
        }

        let input_mappings: serde_json::Value = if input_mappings_str.trim().is_empty() {
            serde_json::json!({})
        } else {
            match serde_json::from_str(&input_mappings_str) {
                Ok(v) => v,
                Err(_) => {
                    self.error = Some("Input mappings must be valid JSON.".to_string());
                    return Ok(());
                }
            }
        };

        let webhook_id_typed = if webhook_id.trim().is_empty() {
            None
        } else {
            match uuid::Uuid::parse_str(&webhook_id).map(WebhookId::new) {
                Ok(id) => Some(id),
                Err(_) => {
                    self.error = Some("Invalid Webhook ID format".to_string());
                    return Ok(());
                }
            }
        };

        let (method, path) = if let Some(id) = self.event_rule_edit_id {
            (reqwest::Method::PATCH, format!("/api/v1/rules/{}", id))
        } else {
            (reqwest::Method::POST, "/api/v1/rules".to_string())
        };

        let payload = serde_json::json!({
            "name": name,
            "description": if description.trim().is_empty() { None::<String> } else { Some(description) },
            "webhook_id": webhook_id_typed.map(|i| i.into_inner()),
            "event_type_pattern": event_type_pattern,
            "condition_expr": if condition_expr.trim().is_empty() { None::<String> } else { Some(condition_expr) },
            "workflow_name": workflow_name,
            "repo_url": repo_url,
            "workflow_path": workflow_path,
            "git_ref": git_ref,
            "input_mappings": input_mappings,
            "is_active": self.event_rule_is_active,
        });

        let res = self.api_request(method, &path, Some(payload)).await?;

        if res.status().is_success() {
            self.event_rule_dialog_active = false;
            self.refresh_event_rules().await?;
        } else {
            self.error = Some(format!("Failed to save event rule: {}", res.status()));
        }

        Ok(())
    }

    /// Deletes the currently selected event rule.
    pub async fn delete_selected_event_rule(&mut self) -> Result<()> {
        if let Some(rule) = &self.selected_event_rule {
            if self.token.is_some() {
                let res = self
                    .api_request(
                        reqwest::Method::DELETE,
                        &format!("/api/v1/rules/{}", rule.id),
                        None,
                    )
                    .await?;

                if res.status().is_success() {
                    self.refresh_event_rules().await?;
                } else {
                    self.error = Some(format!("Failed to delete event rule: {}", res.status()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
