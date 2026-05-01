use crate::utils::{handle_response, require_token};
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;
use uuid::Uuid;

#[derive(Subcommand)]
pub enum WebhookCommands {
    /// List webhooks
    List,
    /// Create a new webhook
    Create {
        name: String,
        #[arg(long)]
        source_type: String,
        #[arg(long)]
        secret: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Get webhook details
    Get { id: Uuid },
    /// Delete a webhook
    Delete { id: Uuid },
}

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: WebhookCommands,
) -> Result<()> {
    match command {
        WebhookCommands::List => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/webhooks", url))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        WebhookCommands::Create {
            name,
            source_type,
            secret,
            description,
        } => {
            let token = require_token(token)?;
            let res = http_client
                .post(format!("{}/api/v1/webhooks", url))
                .header("Authorization", format!("Bearer {}", token))
                .json(&json!({
                    "name": name,
                    "source_type": source_type,
                    "secret_token": secret,
                    "description": description,
                }))
                .send()
                .await?;
            handle_response(res).await?;
        }
        WebhookCommands::Get { id } => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/webhooks/{}", url, id))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        WebhookCommands::Delete { id } => {
            let token = require_token(token)?;
            let res = http_client
                .delete(format!("{}/api/v1/webhooks/{}", url, id))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_webhook_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/webhooks"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::List;

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/webhooks"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "created"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::Create {
            name: "test-webhook".to_string(),
            source_type: "github".to_string(),
            secret: Some("secret".to_string()),
            description: None,
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_get() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": id})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::Get { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_webhook_delete() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::Delete { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }
}
