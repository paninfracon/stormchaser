use crate::utils::{handle_run_response, parse_key_val_list, require_token};
use anyhow::Result;
use serde_json::json;

pub struct EnqueueRunParams {
    pub workflow_name: String,
    pub repo: String,
    pub path: String,
    pub git_ref: String,
    pub input: Vec<String>,
    pub tail: bool,
    pub watch: bool,
}

pub async fn enqueue_run(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    params: EnqueueRunParams,
) -> Result<()> {
    let inputs = parse_key_val_list(params.input);
    let token_str = require_token(token)?;
    let res = http_client
        .post(format!("{}/api/v1/runs", url))
        .header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token_str),
        )
        .json(&json!({
            "workflow_name": params.workflow_name,
            "repo_url": params.repo,
            "workflow_path": params.path,
            "git_ref": params.git_ref,
            "inputs": inputs,
        }))
        .send()
        .await?;

    handle_run_response(http_client, url, token_str, res, params.tail, params.watch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use stormchaser_model::RunStatus;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_enqueue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs"))
            .and(header(reqwest::header::AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "12345678-1234-1234-1234-123456789012",
                "status": RunStatus::Queued
            })))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let params = EnqueueRunParams {
            workflow_name: "test".to_string(),
            repo: "http://git".to_string(),
            path: "workflow.yaml".to_string(),
            git_ref: "main".to_string(),
            input: vec![],
            tail: false,
            watch: false,
        };

        let result = enqueue_run(&server.uri(), Some("test-token"), &client, params).await;
        assert!(result.is_ok());
    }
}
