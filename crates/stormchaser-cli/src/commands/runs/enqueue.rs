use crate::utils::{follow_run, parse_key_val_list};
use anyhow::Result;
use stormchaser_client::Client;
use stormchaser_model::api::EnqueueRequest;

pub struct EnqueueRunParams {
    pub workflow_name: String,
    pub connection: String,
    pub path: String,
    pub git_ref: String,
    pub input: Vec<String>,
    pub tail: bool,
    pub watch: bool,
}

pub async fn enqueue_run(client: &Client, params: EnqueueRunParams) -> Result<()> {
    let inputs = parse_key_val_list(params.input);
    let run = client
        .enqueue(&EnqueueRequest {
            workflow_name: params.workflow_name,
            connection: params.connection,
            workflow_path: params.path,
            git_ref: params.git_ref,
            inputs: inputs.into(),
            overrides: None,
            sops_file: None,
            sops_role_arn: None,
        })
        .await;

    follow_run(client, run, params.tail, params.watch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_enqueue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs"))
            .and(header("authorization", "Bearer test-token"))
            .and(body_partial_json(json!({
                "workflow_name": "test",
                "connection": "git",
                "workflow_path": "main.storm",
                "git_ref": "main",
                "inputs": {}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "run_id": "12345678-1234-1234-1234-123456789012",
                "status": "queued"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let params = EnqueueRunParams {
            workflow_name: "test".to_string(),
            connection: "git".to_string(),
            path: "main.storm".to_string(),
            git_ref: "main".to_string(),
            input: vec![],
            tail: false,
            watch: false,
        };

        let result = enqueue_run(&client, params).await;
        result.unwrap();
    }
}
