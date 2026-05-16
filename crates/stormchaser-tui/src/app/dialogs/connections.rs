use ratatui_textarea::TextArea;
use stormchaser_model::connections::ConnectionType;

impl<'a> crate::app::App<'a> {
    /// Opens the storage backend create/edit dialog.
    pub fn open_connection_dialog(&mut self, edit: bool) {
        self.connection_dialog_active = true;
        self.connection_focus = 0;

        if edit {
            if let Some(backend) = &self.selected_connection {
                self.connection_edit_id = Some(backend.id);
                self.connection_inputs = vec![
                    TextArea::from(vec![backend.name.clone()]),
                    TextArea::from(
                        backend
                            .description
                            .clone()
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    TextArea::from(
                        serde_json::to_string_pretty(&backend.config)
                            .unwrap_or_default()
                            .lines()
                            .map(String::from)
                            .collect::<Vec<_>>(),
                    ),
                    TextArea::from(vec![backend
                        .aws_assume_role_arn
                        .clone()
                        .unwrap_or_default()]),
                ];
                let type_str = match backend.connection_type {
                    ConnectionType::S3 => "S3",
                    ConnectionType::Oci => "Oci",
                    ConnectionType::Jfrog => "Jfrog",
                    ConnectionType::Gcs => "Gcs",
                    ConnectionType::Azure => "Azure",
                    ConnectionType::Postgres => "Postgres",
                    ConnectionType::Mysql => "Mysql",
                    ConnectionType::HttpApi => "HttpApi",
                    ConnectionType::Git => "Git",
                };
                self.connection_type_index = crate::app::BACKEND_TYPE_OPTIONS
                    .iter()
                    .position(|&s| s == type_str)
                    .unwrap_or(0);
                self.connection_is_default = backend.is_default_sfs;
                return;
            }
        }

        self.connection_edit_id = None;
        self.connection_inputs = vec![
            TextArea::default(),                    // name
            TextArea::default(),                    // description
            TextArea::from(vec!["{}".to_string()]), // config
            TextArea::default(),                    // assume role arn
        ];
        self.connection_type_index = 0;
        self.connection_is_default = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::app::ConnectionId;
    use chrono::Utc;
    use stormchaser_model::connections::Connection;

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
    fn test_open_connection_dialog_new() {
        let mut app = setup_app();
        app.open_connection_dialog(false);
        assert!(app.connection_dialog_active);
        assert_eq!(app.connection_edit_id, None);
        assert_eq!(app.connection_inputs.len(), 4);
    }

    #[test]
    fn test_open_connection_dialog_edit() {
        let mut app = setup_app();
        let backend = Connection {
            id: ConnectionId::new_v4(),
            name: "test_backend".to_string(),
            description: Some("desc".to_string()),
            connection_type: ConnectionType::S3,
            is_default_sfs: true,
            encrypted_credentials: None,
            config: serde_json::json!({"region": "us-east-1"}),
            aws_assume_role_arn: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        app.selected_connection = Some(backend.clone());

        app.open_connection_dialog(true);
        assert!(app.connection_dialog_active);
        assert_eq!(app.connection_edit_id, Some(backend.id));
        assert_eq!(app.connection_inputs[0].lines()[0], "test_backend");
        assert_eq!(app.connection_type_index, 0);
        assert!(app.connection_is_default);
    }
}
