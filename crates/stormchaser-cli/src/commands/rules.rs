use crate::utils::print_json;
use anyhow::Result;
use clap::Subcommand;
use std::collections::HashMap;
use stormchaser_client::Client;
use stormchaser_model::api::CreateEventRuleRequest;

#[derive(Subcommand)]
pub enum RuleCommands {
    /// List event rules
    List,
    /// Create a new event rule
    Create {
        name: String,
        #[arg(long)]
        webhook_id: stormchaser_model::WebhookId,
        #[arg(long)]
        event_pattern: String,
        #[arg(long)]
        workflow: String,
        /// The Git connection (ID or name) holding the workflow
        #[arg(long, visible_alias = "connection")]
        repo: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "main")]
        git_ref: String,
        #[arg(long)]
        description: Option<String>,
        /// Input mappings in name=HCL_EXPR format
        #[arg(short, long)]
        mapping: Vec<String>,
    },
    /// Delete an event rule
    Delete { id: stormchaser_model::RuleId },
}

pub async fn handle(client: &Client, command: RuleCommands) -> Result<()> {
    match command {
        RuleCommands::List => {
            print_json(&client.list_event_rules().await?)?;
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
            let created = client
                .create_event_rule(&CreateEventRuleRequest {
                    name,
                    description,
                    webhook_id,
                    event_type_pattern: event_pattern,
                    condition_expr: None,
                    workflow_name: workflow,
                    connection: repo,
                    workflow_path: path,
                    git_ref,
                    input_mappings: parse_mappings(mapping),
                })
                .await?;
            print_json(&created)?;
        }
        RuleCommands::Delete { id } => {
            client.delete_event_rule(id).await?;
            println!("Success");
        }
    }
    Ok(())
}

/// `name=HCL_EXPR` pairs. The expression is kept as written, not read as JSON.
fn parse_mappings(list: Vec<String>) -> HashMap<String, String> {
    list.iter()
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, expr)| (name.to_string(), expr.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_rules_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/rules"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = RuleCommands::List;

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_rules_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/rules"))
            .and(header("authorization", "Bearer test-token"))
            .and(body_partial_json(json!({
                "connection": "my-repo",
                "event_type_pattern": "push",
                "input_mappings": { "key": "event.ref", "quoted": "\"main\"" }
            })))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"id": uuid::Uuid::new_v4()})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = RuleCommands::Create {
            name: "test-rule".to_string(),
            webhook_id: stormchaser_model::WebhookId::new_v4(),
            event_pattern: "push".to_string(),
            workflow: "my-wf".to_string(),
            repo: "my-repo".to_string(),
            path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            description: Some("desc".to_string()),
            mapping: vec!["key=event.ref".to_string(), "quoted=\"main\"".to_string()],
        };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_rules_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::RuleId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/rules/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = RuleCommands::Delete { id };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }
}
