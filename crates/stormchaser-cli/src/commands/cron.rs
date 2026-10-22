use crate::utils::{parse_key_val_list, print_json};
use anyhow::Result;
use clap::Subcommand;
use stormchaser_client::Client;
use stormchaser_model::api::CreateCronWorkflowRequest;

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
        /// The Git connection (ID or name) holding the workflow
        #[arg(long, visible_alias = "connection")]
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

pub async fn handle(client: &Client, command: CronCommands) -> Result<()> {
    match command {
        CronCommands::List => {
            print_json(&client.list_cron_workflows().await?)?;
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
            let created = client
                .create_cron_workflow(&CreateCronWorkflowRequest {
                    name,
                    description,
                    cronspec: cron,
                    workflow_name: workflow,
                    connection: repo,
                    workflow_path: path,
                    git_ref,
                    inputs: inputs.into(),
                })
                .await?;
            print_json(&created)?;
        }
        CronCommands::Delete { id } => {
            client.delete_cron_workflow(id).await?;
            println!("Success");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_cron_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/cron-workflows"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = CronCommands::List;

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_cron_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/cron-workflows"))
            .and(header("authorization", "Bearer test-token"))
            .and(body_partial_json(json!({
                "connection": "my-repo",
                "cronspec": "* * * * *",
                "inputs": { "key": "value" }
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "id": uuid::Uuid::new_v4(),
                "secret_token": "token",
                "external_job_id": null
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = CronCommands::Create {
            name: "test-cron".to_string(),
            cron: "* * * * *".to_string(),
            workflow: "my-wf".to_string(),
            repo: "my-repo".to_string(),
            path: "wf.storm".to_string(),
            git_ref: "main".to_string(),
            description: Some("desc".to_string()),
            input: vec!["key=value".to_string()],
        };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_cron_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::CronWorkflowId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/cron-workflows/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = CronCommands::Delete { id };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }
}
