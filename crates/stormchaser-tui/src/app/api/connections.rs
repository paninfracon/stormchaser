use super::*;
use stormchaser_model::api::{
    CreateStorageBackendRequest, TestConnectionRequest, UpdateStorageBackendRequest,
};
use stormchaser_model::connections::ConnectionType;

/// The connection type the dialog's type list offers at `index`.
fn connection_type(index: usize) -> ConnectionType {
    match crate::app::BACKEND_TYPE_OPTIONS[index] {
        "S3" => ConnectionType::S3,
        "Oci" => ConnectionType::Oci,
        "Jfrog" => ConnectionType::Jfrog,
        "Gcs" => ConnectionType::Gcs,
        "Azure" => ConnectionType::Azure,
        "Postgres" => ConnectionType::Postgres,
        "Mysql" => ConnectionType::Mysql,
        "HttpApi" => ConnectionType::HttpApi,
        "Git" => ConnectionType::Git,
        _ => ConnectionType::S3, // Fallback
    }
}

impl<'a> App<'a> {
    /// Fetches the latest list of storage backends from the API.
    pub async fn refresh_connections(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        match self.api().list_connections().await {
            Ok(connections) => {
                self.connections = connections;
                if !self.connections.is_empty() {
                    if self.connections_state.selected().is_none() {
                        self.connections_state.select(Some(0));
                    }
                    if self.selected_connection.is_none() {
                        if let Some(i) = self.connections_state.selected() {
                            self.selected_connection = Some(self.connections[i].clone());
                        }
                    }
                } else {
                    self.connections_state.select(None);
                    self.selected_connection = None;
                }
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to fetch storage backends: {}", refused(e)?));
            }
        }
        Ok(())
    }

    /// Tests the connection configuration without saving it.
    pub async fn test_connection_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let config_str = self.connection_inputs[2].lines().join("\n");
        let role_arn = self.connection_inputs[3].lines().join("\n");

        let config: Value = match serde_json::from_str(&config_str) {
            Ok(c) => c,
            Err(_) => {
                self.error = Some("Invalid JSON configuration.".to_string());
                return Ok(());
            }
        };

        let request = TestConnectionRequest {
            connection_type: connection_type(self.connection_type_index),
            config,
            aws_assume_role_arn: if role_arn.trim().is_empty() {
                None
            } else {
                Some(role_arn.trim().to_string())
            },
        };

        match self.api().test_connection(&request).await {
            Ok(result) if result.success => {
                self.error = Some(format!("Test Connection Success: {}", result.message));
            }
            Ok(result) => {
                self.error = Some(format!("Test Connection Failed: {}", result.message));
            }
            Err(e) => {
                self.error = Some(format!("Failed to test connection: {}", refused(e)?));
            }
        }

        Ok(())
    }

    /// Submits the storage backend form for creation or update.
    pub async fn submit_connection_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let name = self.connection_inputs[0].lines().join("\n");
        let description = self.connection_inputs[1].lines().join("\n");
        let config_str = self.connection_inputs[2].lines().join("\n");
        let role_arn = self.connection_inputs[3].lines().join("\n");

        if name.trim().is_empty() {
            self.error = Some("Name is required.".to_string());
            return Ok(());
        }

        let config: Value = match serde_json::from_str(&config_str) {
            Ok(c) => c,
            Err(_) => {
                self.error = Some("Invalid JSON configuration.".to_string());
                return Ok(());
            }
        };

        let connection_type = connection_type(self.connection_type_index);
        let description = if description.trim().is_empty() {
            None
        } else {
            Some(description)
        };
        let aws_assume_role_arn = if role_arn.trim().is_empty() {
            None
        } else {
            Some(role_arn.trim().to_string())
        };

        let result = if let Some(id) = self.connection_edit_id {
            let request = UpdateStorageBackendRequest {
                name: Some(name),
                description,
                connection_type: Some(connection_type),
                config: Some(config),
                aws_assume_role_arn,
                encrypted_credentials: None,
                is_default_sfs: Some(self.connection_is_default),
            };
            self.api().update_connection(id, &request).await
        } else {
            let request = CreateStorageBackendRequest {
                name,
                description,
                connection_type,
                config,
                aws_assume_role_arn,
                encrypted_credentials: None,
                is_default_sfs: self.connection_is_default,
            };
            self.api().create_connection(&request).await.map(|_| ())
        };

        match result {
            Ok(()) => {
                self.connection_dialog_active = false;
                self.refresh_connections().await?;
            }
            Err(e) => {
                self.error = Some(format!("Failed to save storage backend: {}", refused(e)?));
            }
        }

        Ok(())
    }

    /// Deletes the currently selected storage backend.
    pub async fn delete_selected_connection(&mut self) -> Result<()> {
        if let Some(backend) = &self.selected_connection {
            if self.token.is_some() {
                match self.api().delete_connection(backend.id).await {
                    Ok(()) => self.refresh_connections().await?,
                    Err(e) => {
                        self.error =
                            Some(format!("Failed to delete storage backend: {}", refused(e)?));
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
    use serde_json::json;
    use stormchaser_model::connections::{Connection, ConnectionType};
    use tokio::sync::mpsc;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_refresh_connections_success() {
        let server = MockServer::start().await;

        let backend = Connection {
            id: ConnectionId::new_v4(),
            name: "test-backend".to_string(),
            description: None,
            connection_type: ConnectionType::S3,
            config: json!({}),
            aws_assume_role_arn: None,
            is_default_sfs: true,
            encrypted_credentials: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![backend.clone()]))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );

        let result = app.refresh_connections().await;
        result.unwrap();
        assert_eq!(app.connections.len(), 1);
        assert_eq!(app.connections[0].id, backend.id);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_submit_connection_form_create() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/connections"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({ "id": ConnectionId::new_v4() })),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<Connection>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );
        app.connection_dialog_active = true;

        app.connection_inputs = vec![
            TextArea::from(vec!["test-name".to_string()]),
            TextArea::from(vec!["".to_string()]),
            TextArea::from(vec!["{}".to_string()]),
            TextArea::from(vec!["".to_string()]),
        ];

        let result = app.submit_connection_form().await;
        result.unwrap();
        assert!(!app.connection_dialog_active);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_submit_connection_form_invalid_json() {
        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );
        app.connection_inputs = vec![
            TextArea::from(vec!["test-name".to_string()]),
            TextArea::from(vec!["".to_string()]),
            TextArea::from(vec!["{invalid}".to_string()]), // Invalid JSON
            TextArea::from(vec!["".to_string()]),
        ];

        let result = app.submit_connection_form().await;
        result.unwrap();
        assert_eq!(app.error, Some("Invalid JSON configuration.".to_string()));
    }

    #[tokio::test]
    async fn test_delete_selected_connection() {
        let server = MockServer::start().await;
        let connection_id = ConnectionId::new_v4();

        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/connections/{}", connection_id)))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<Connection>::new()))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );

        let backend = Connection {
            id: connection_id,
            name: "test-backend".to_string(),
            description: None,
            connection_type: ConnectionType::S3,
            config: json!({}),
            aws_assume_role_arn: None,
            is_default_sfs: true,
            encrypted_credentials: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        app.selected_connection = Some(backend);

        let result = app.delete_selected_connection().await;
        result.unwrap();
        assert!(app.error.is_none());
    }
}
