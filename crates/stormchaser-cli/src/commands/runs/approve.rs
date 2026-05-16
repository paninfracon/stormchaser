use crate::utils::{handle_response, parse_key_val_list, require_token};
use anyhow::Result;
use reqwest::header::AUTHORIZATION;

pub async fn approve_step(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    input: Vec<String>,
) -> Result<()> {
    let token = require_token(token)?;
    let inputs = parse_key_val_list(input);
    let res = http_client
        .post(format!(
            "{}/api/v1/runs/{}/steps/{}/approve",
            url, run_id, step_id
        ))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .json(&inputs)
        .send()
        .await?;
    handle_response(res).await
}

pub async fn reject_step(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .post(format!(
            "{}/api/v1/runs/{}/steps/{}/reject",
            url, run_id, step_id
        ))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

pub async fn approve_link(
    url: &str,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    link_token: String,
) -> Result<()> {
    let res = http_client
        .get(format!("{}/api/v1/approve-link/{}", url, link_token))
        .send()
        .await?;
    handle_response(res).await
}

pub async fn list_pending(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs?status=Running", url))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use serde_json::json;
    use stormchaser_model::{RunId, StepInstanceId};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_approve() {
        let server = MockServer::start().await;
        let run_id = RunId::new_v4();
        let step_id = StepInstanceId::new_v4();
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/runs/{}/steps/{}/approve",
                run_id, step_id
            )))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "approved"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = approve_step(
            &server.uri(),
            Some("test-token"),
            &client,
            run_id,
            step_id,
            vec![],
        )
        .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_runs_reject() {
        let server = MockServer::start().await;
        let run_id = RunId::new_v4();
        let step_id = StepInstanceId::new_v4();
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/runs/{}/steps/{}/reject",
                run_id, step_id
            )))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "rejected"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = reject_step(&server.uri(), Some("test-token"), &client, run_id, step_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_runs_pending() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = list_pending(&server.uri(), Some("test-token"), &client).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_runs_approve_link() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/approve-link/my-secret-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "approved"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = approve_link(&server.uri(), &client, "my-secret-token".to_string()).await;
        assert!(result.is_ok());
    }
}
