use ratatui_textarea::TextArea;
impl<'a> crate::app::App<'a> {
    /// Opens the webhook create/edit dialog.
    pub fn open_webhook_dialog(&mut self, edit: bool) {
        self.webhook_dialog_active = true;
        self.webhook_focus = 0;

        if edit {
            if let Some(webhook) = &self.selected_webhook {
                self.webhook_edit_id = Some(webhook.id);
                self.webhook_inputs = vec![
                    TextArea::from(vec![webhook.name.clone()]),
                    TextArea::from(
                        webhook
                            .description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    TextArea::from(
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
            TextArea::default(), // name
            TextArea::default(), // description
            TextArea::default(), // secret token
        ];
        self.webhook_source_type_index = 0;
        self.webhook_is_active = true;
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::app::App;
    use crate::app::WebhookId;
    use stormchaser_model::event_rules::WebhookConfig;

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
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        app.selected_webhook = Some(webhook.clone());

        app.open_webhook_dialog(true);
        assert!(app.webhook_dialog_active);
        assert_eq!(app.webhook_edit_id, Some(webhook.id));
        assert_eq!(app.webhook_inputs[0].lines()[0], "hook");
        assert_eq!(app.webhook_source_type_index, 0); // github
        assert!(!app.webhook_is_active);
    }
}
