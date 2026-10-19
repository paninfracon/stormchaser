use crate::utils::print_json;
use anyhow::Result;
use clap::Subcommand;
use stormchaser_client::Client;
use stormchaser_model::api::{CreateWebhookRequest, UpdateWebhookRequest};

#[derive(Subcommand)]
pub enum WebhookCommands {
    /// List webhooks
    List,
    /// Create a new webhook
    Create {
        name: String,
        /// `github`, `signed` (Standard Webhooks HMAC signature) or `generic` (bearer token)
        #[arg(long)]
        source_type: String,
        /// The secret deliveries are checked against. Required.
        #[arg(long)]
        secret: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Get webhook details
    Get { id: stormchaser_model::WebhookId },
    /// Update a webhook
    Update {
        id: stormchaser_model::WebhookId,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        source_type: Option<String>,
        #[arg(long)]
        secret: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        is_active: Option<bool>,
    },
    /// Delete a webhook
    Delete { id: stormchaser_model::WebhookId },
}

pub async fn handle(client: &Client, command: WebhookCommands) -> Result<()> {
    match command {
        WebhookCommands::List => {
            print_json(&client.list_webhooks().await?)?;
        }
        WebhookCommands::Create {
            name,
            source_type,
            secret,
            description,
        } => {
            let created = client
                .create_webhook(&CreateWebhookRequest {
                    name,
                    description,
                    source_type,
                    secret_token: secret,
                })
                .await?;
            print_json(&created)?;
        }
        WebhookCommands::Get { id } => {
            print_json(&client.get_webhook(id).await?)?;
        }
        WebhookCommands::Delete { id } => {
            client.delete_webhook(id).await?;
            println!("Success");
        }
        WebhookCommands::Update {
            id,
            name,
            source_type,
            secret,
            description,
            is_active,
        } => {
            client
                .update_webhook(
                    id,
                    &UpdateWebhookRequest {
                        name,
                        description,
                        source_type,
                        secret_token: secret,
                        is_active,
                    },
                )
                .await?;
            println!("Success");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_webhook_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/webhooks"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = WebhookCommands::List;

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/webhooks"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"id": uuid::Uuid::new_v4()})),
            )
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = WebhookCommands::Create {
            name: "test-webhook".to_string(),
            source_type: "github".to_string(),
            secret: Some("secret".to_string()),
            description: None,
        };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_get() {
        let server = MockServer::start().await;
        let id = stormchaser_model::WebhookId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": id,
                "name": "hook",
                "description": null,
                "source_type": "github",
                "secret_token": null,
                "is_active": true,
                "ca_cert": null,
                "client_cert": null,
                "client_key": null,
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            })))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = WebhookCommands::Get { id };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_update() {
        let server = MockServer::start().await;
        let id = stormchaser_model::WebhookId::new_v4();
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .and(body_json(json!({
                "name": "new-name",
                "description": null,
                "source_type": null,
                "secret_token": null,
                "is_active": false
            })))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = WebhookCommands::Update {
            id,
            name: Some("new-name".to_string()),
            source_type: None,
            secret: None,
            description: None,
            is_active: Some(false),
        };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::WebhookId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = WebhookCommands::Delete { id };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }
}
