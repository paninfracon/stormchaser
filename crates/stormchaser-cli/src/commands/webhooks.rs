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
