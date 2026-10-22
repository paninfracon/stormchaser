use crate::utils::{parse_key_val_list, print_json};
use anyhow::Result;
use stormchaser_client::Client;
use stormchaser_model::api::ListRunsQuery;
use stormchaser_model::RunStatus;

pub async fn approve_step(
    client: &Client,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
    input: Vec<String>,
) -> Result<()> {
    let inputs = parse_key_val_list(input);
    client.approve(run_id, step_id, &inputs.into()).await?;
    println!("Approved");
    Ok(())
}

pub async fn reject_step(
    client: &Client,
    run_id: stormchaser_model::RunId,
    step_id: stormchaser_model::StepInstanceId,
) -> Result<()> {
    client.reject(run_id, step_id).await?;
    println!("Rejected");
    Ok(())
}

pub async fn approve_link(client: &Client, link_token: String) -> Result<()> {
    // POST acts on the link; GET only returns its confirmation page
    client.approve_link(&link_token).await?;
    println!("Success");
    Ok(())
}

pub async fn list_pending(client: &Client) -> Result<()> {
    let runs = client
        .list_runs(&ListRunsQuery {
            status: Some(RunStatus::Running),
            ..Default::default()
        })
        .await?;
    print_json(&runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stormchaser_model::{RunId, StepInstanceId};
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> Client {
        Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap()
    }

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
            .and(header("authorization", "Bearer test-token"))
            .and(body_json(json!({ "reason": "ok" })))
            .respond_with(ResponseTemplate::new(200).set_body_string("Approved"))
            .expect(1)
            .mount(&server)
            .await;

        let result = approve_step(
            &client(&server),
            run_id,
            step_id,
            vec!["reason=ok".to_string()],
        )
        .await;
        result.unwrap();
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
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Rejected"))
            .expect(1)
            .mount(&server)
            .await;

        let result = reject_step(&client(&server), run_id, step_id).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_runs_pending() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .and(query_param("status", "running"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(&server)
            .await;

        let result = list_pending(&client(&server)).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_runs_approve_link() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/approve-link/my-secret-token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Successfully approved"))
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri()).build().unwrap();
        let result = approve_link(&client, "my-secret-token".to_string()).await;
        result.unwrap();
    }
}
