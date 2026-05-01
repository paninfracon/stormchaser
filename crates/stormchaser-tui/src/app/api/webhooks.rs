use super::*;

impl<'a> App<'a> {
    /// Fetches the latest list of webhooks from the API.
    pub async fn refresh_webhooks(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let res = self
            .api_request(reqwest::Method::GET, "/api/v1/webhooks", None)
            .await?;

        if res.status().is_success() {
            self.webhooks = res
                .json::<Vec<stormchaser_model::event_rules::WebhookConfig>>()
                .await?;
            if !self.webhooks.is_empty() {
                if self.webhooks_state.selected().is_none() {
                    self.webhooks_state.select(Some(0));
                }
                if self.selected_webhook.is_none() {
                    if let Some(i) = self.webhooks_state.selected() {
                        self.selected_webhook = Some(self.webhooks[i].clone());
                    }
                }
            } else {
                self.webhooks_state.select(None);
                self.selected_webhook = None;
            }
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch webhooks: {}", res.status()));
        }
        Ok(())
    }

    /// Submits the webhook form for creation or update.
    pub async fn submit_webhook_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let name = self.webhook_inputs[0].lines().join("\n");
        let description = self.webhook_inputs[1].lines().join("\n");
        let secret_token = self.webhook_inputs[2].lines().join("\n");

        if name.trim().is_empty() {
            self.error = Some("Name is required.".to_string());
            return Ok(());
        }

        let source_type =
            crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS[self.webhook_source_type_index].to_string();

        let (method, path) = if let Some(id) = self.webhook_edit_id {
            (reqwest::Method::PATCH, format!("/api/v1/webhooks/{}", id))
        } else {
            (reqwest::Method::POST, "/api/v1/webhooks".to_string())
        };

        let payload = serde_json::json!({
            "name": name,
            "description": if description.trim().is_empty() { None::<String> } else { Some(description) },
            "source_type": source_type,
            "secret_token": if secret_token.trim().is_empty() { None::<String> } else { Some(secret_token) },
            "is_active": self.webhook_is_active,
        });

        let res = self.api_request(method, &path, Some(payload)).await?;

        if res.status().is_success() {
            self.webhook_dialog_active = false;
            self.refresh_webhooks().await?;
        } else {
            self.error = Some(format!("Failed to save webhook: {}", res.status()));
        }

        Ok(())
    }

    /// Deletes the currently selected webhook.
    pub async fn delete_selected_webhook(&mut self) -> Result<()> {
        if let Some(webhook) = &self.selected_webhook {
            if self.token.is_some() {
                let res = self
                    .api_request(
                        reqwest::Method::DELETE,
                        &format!("/api/v1/webhooks/{}", webhook.id),
                        None,
                    )
                    .await?;

                if res.status().is_success() {
                    self.refresh_webhooks().await?;
                } else {
                    self.error = Some(format!("Failed to delete webhook: {}", res.status()));
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
    use stormchaser_model::event_rules::WebhookConfig;
    use tokio::sync::mpsc;
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_refresh_webhooks_success() {
        let server = MockServer::start().await;

        let webhook = WebhookConfig {
            id: Uuid::new_v4(),
            name: "test-webhook".to_string(),
            description: None,
            source_type: "github".to_string(),
            secret_token: None,
            is_active: true,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/webhooks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![webhook.clone()]))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);

        let result = app.refresh_webhooks().await;
        assert!(result.is_ok());
        assert_eq!(app.webhooks.len(), 1);
        assert_eq!(app.webhooks[0].id, webhook.id);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_submit_webhook_form_create() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/webhooks"))
            .respond_with(ResponseTemplate::new(201))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/webhooks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<WebhookConfig>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);
        app.webhook_dialog_active = true;
        app.webhook_inputs = vec![
            TextArea::from(vec!["test-webhook".to_string()]),
            TextArea::from(vec!["".to_string()]),
            TextArea::from(vec!["secret".to_string()]),
        ];

        let result = app.submit_webhook_form().await;
        assert!(result.is_ok());
        assert!(!app.webhook_dialog_active);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_delete_selected_webhook() {
        let server = MockServer::start().await;
        let webhook_id = Uuid::new_v4();

        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/webhooks/{}", webhook_id)))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/webhooks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<WebhookConfig>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);

        let webhook = WebhookConfig {
            id: webhook_id,
            name: "test-webhook".to_string(),
            description: None,
            source_type: "github".to_string(),
            secret_token: None,
            is_active: true,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        app.selected_webhook = Some(webhook);

        let result = app.delete_selected_webhook().await;
        assert!(result.is_ok());
        assert!(app.error.is_none());
    }
}
