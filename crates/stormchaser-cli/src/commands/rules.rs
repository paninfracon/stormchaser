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

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_rules_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/rules"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RuleCommands::List;

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_rules_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/rules"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "created"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RuleCommands::Create {
            name: "test-rule".to_string(),
            webhook_id: Uuid::new_v4(),
            event_pattern: "push".to_string(),
            workflow: "my-wf".to_string(),
            repo: "https://github.com/a/b".to_string(),
            path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            description: Some("desc".to_string()),
            mapping: vec!["key=value".to_string()],
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_rules_delete() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/rules/{}", id)))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RuleCommands::Delete { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }
}
