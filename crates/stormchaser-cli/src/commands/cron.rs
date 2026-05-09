use crate::utils::{handle_response, parse_key_val_list, require_token};
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;

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
    Delete {
        id: stormchaser_model::CronWorkflowId,
    },
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
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
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
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
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
                .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
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
    async fn test_cron_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/cron-workflows"))
            .and(header(reqwest::header::AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = CronCommands::List;

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cron_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/cron-workflows"))
            .and(header(reqwest::header::AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "created"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = CronCommands::Create {
            name: "test-cron".to_string(),
            cron: "* * * * *".to_string(),
            workflow: "my-wf".to_string(),
            repo: "https://github.com/a/b".to_string(),
            path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            description: Some("desc".to_string()),
            input: vec!["key=value".to_string()],
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_cron_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::CronWorkflowId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/cron-workflows/{}", id)))
            .and(header(reqwest::header::AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = CronCommands::Delete { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }
}
