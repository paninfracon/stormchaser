use crate::utils::{handle_response, parse_key_val_list, require_token};
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;
use uuid::Uuid;

#[derive(Subcommand)]
pub enum CronCommands {
    /// List scheduled cron workflows
    List,
    /// Schedule a new cron workflow
    Create {
        name: String,
        #[arg(long)]
        cron: String,
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
        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,
    },
    /// Delete a scheduled cron workflow
    Delete { id: Uuid },
}

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: CronCommands,
) -> Result<()> {
    match command {
        CronCommands::List => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/cron-workflows", url))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        CronCommands::Create {
            name,
            cron,
            workflow,
            repo,
            path,
            git_ref,
            description,
            input,
        } => {
            let inputs = parse_key_val_list(input);
            let token = require_token(token)?;
            let res = http_client
                .post(format!("{}/api/v1/cron-workflows", url))
                .header("Authorization", format!("Bearer {}", token))
                .json(&json!({
                    "name": name,
                    "cronspec": cron,
                    "workflow_name": workflow,
                    "repo_url": repo,
                    "workflow_path": path,
                    "git_ref": git_ref,
                    "inputs": inputs,
                    "description": description,
                }))
                .send()
                .await?;
            handle_response(res).await?;
        }
        CronCommands::Delete { id } => {
            let token = require_token(token)?;
            let res = http_client
                .delete(format!("{}/api/v1/cron-workflows/{}", url, id))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
    }
    Ok(())
}
