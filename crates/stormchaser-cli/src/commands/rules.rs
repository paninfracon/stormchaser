use crate::utils::{handle_response, parse_key_val_list, require_token};
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;
use uuid::Uuid;

#[derive(Subcommand)]
pub enum RuleCommands {
    /// List event rules
    List,
    /// Create a new event rule
    Create {
        name: String,
        #[arg(long)]
        webhook_id: Uuid,
        #[arg(long)]
        event_pattern: String,
        #[arg(long)]
        workflow: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "main")]
        git_ref: String,
        #[arg(long)]
        description: Option<String>,
        /// Input mappings in name=CEL_EXPR format
        #[arg(short, long)]
        mapping: Vec<String>,
    },
    /// Delete an event rule
    Delete { id: Uuid },
}

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: RuleCommands,
) -> Result<()> {
    match command {
        RuleCommands::List => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/rules", url))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        RuleCommands::Create {
            name,
            webhook_id,
            event_pattern,
            workflow,
            repo,
            path,
            git_ref,
            description,
            mapping,
        } => {
            let mappings = parse_key_val_list(mapping);
            let token = require_token(token)?;
            let res = http_client
                .post(format!("{}/api/v1/rules", url))
                .header("Authorization", format!("Bearer {}", token))
                .json(&json!({
                    "name": name,
                    "webhook_id": webhook_id,
                    "event_type_pattern": event_pattern,
                    "workflow_name": workflow,
                    "repo_url": repo,
                    "workflow_path": path,
                    "git_ref": git_ref,
                    "description": description,
                    "input_mappings": mappings,
                }))
                .send()
                .await?;
            handle_response(res).await?;
        }
        RuleCommands::Delete { id } => {
            let token = require_token(token)?;
            let res = http_client
                .delete(format!("{}/api/v1/rules/{}", url, id))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
    }
    Ok(())
}
