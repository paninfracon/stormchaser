use crate::utils::{handle_response, require_token};
use anyhow::Result;
use clap::Subcommand;
use reqwest::header::AUTHORIZATION;
use serde_json::json;

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
                .header(AUTHORIZATION, format!("Bearer {}", token))
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
                .header(AUTHORIZATION, format!("Bearer {}", token))
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
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        WebhookCommands::Delete { id } => {
            let token = require_token(token)?;
            let res = http_client
                .delete(format!("{}/api/v1/webhooks/{}", url, id))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        WebhookCommands::Update {
            id,
            name,
            source_type,
            secret,
            description,
            is_active,
        } => {
            let token = require_token(token)?;
            let mut body = serde_json::Map::new();
            if let Some(n) = name {
                body.insert("name".to_string(), json!(n));
            }
            if let Some(st) = source_type {
                body.insert("source_type".to_string(), json!(st));
            }
            if let Some(sec) = secret {
                body.insert("secret_token".to_string(), json!(sec));
            }
            if let Some(desc) = description {
                body.insert("description".to_string(), json!(desc));
            }
            if let Some(ia) = is_active {
                body.insert("is_active".to_string(), json!(ia));
            }

            let res = http_client
                .patch(format!("{}/api/v1/webhooks/{}", url, id))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .json(&body)
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
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::List;

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/webhooks"))
            .and(header(AUTHORIZATION, "Bearer test-token"))
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
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_get() {
        let server = MockServer::start().await;
        let id = stormchaser_model::WebhookId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": id})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::Get { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_update() {
        let server = MockServer::start().await;
        let id = stormchaser_model::WebhookId::new_v4();
        Mock::given(method("PATCH"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "updated"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::Update {
            id,
            name: Some("new-name".to_string()),
            source_type: None,
            secret: None,
            description: None,
            is_active: Some(false),
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_webhook_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::WebhookId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/webhooks/{}", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = WebhookCommands::Delete { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }
}
