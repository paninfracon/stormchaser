use super::*;
use reqwest::header::AUTHORIZATION;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_build_list_runs_url_basic() {
        let url = build_list_runs_url(
            "http://localhost:8080",
            ListRunsFilters {
                owner: None,
                name: None,
                repo_url: None,
                workflow_path: None,
                created_after: None,
                created_before: None,
                status: None,
            },
        )
        .unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/v1/runs");
    }

    #[test]
    fn test_build_list_runs_url_with_params() {
        let url = build_list_runs_url(
            "http://localhost:8080",
            ListRunsFilters {
                owner: Some("alice".to_string()),
                name: Some("my-workflow".to_string()),
                repo_url: None,
                workflow_path: None,
                created_after: None,
                created_before: None,
                status: Some("Succeeded".to_string()),
            },
        )
        .unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("initiating_user").unwrap(), "alice");
        assert_eq!(query.get("workflow_name").unwrap(), "my-workflow");
        assert_eq!(query.get("status").unwrap(), "Succeeded");
        assert!(!query.contains_key("repo_url"));
    }

    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RunCommands::List {
            owner: None,
            name: None,
            repo_url: None,
            workflow_path: None,
            created_after: None,
            created_before: None,
            status: None,
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_runs_get() {
        let server = MockServer::start().await;
        let id = RunId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": id})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RunCommands::Get { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_runs_artifacts() {
        let server = MockServer::start().await;
        let id = RunId::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/artifacts", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RunCommands::Artifacts { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
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
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "approved"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RunCommands::Approve {
            run_id,
            step_id,
            input: vec![],
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
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
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "rejected"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RunCommands::Reject { run_id, step_id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_runs_pending() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            // It expects a query string `?status=Running` but wiremock path matcher ignores query
            // so we can just match path or use path_and_query
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RunCommands::Pending;

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_runs_enqueue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs/enqueue"))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "12345678-1234-1234-1234-123456789012",
                "status": RunStatus::Queued
            })))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = RunCommands::Enqueue {
            workflow_name: "test".to_string(),
            repo: "http://git".to_string(),
            path: "workflow.yaml".to_string(),
            git_ref: "main".to_string(),
            input: vec![],
            tail: false,
            watch: false,
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        result.unwrap();
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
        let cmd = RunCommands::ApproveLink {
            token: "my-secret-token".to_string(),
        };

        let result = handle(&server.uri(), None, &client, cmd).await;
        result.unwrap();
    }
}
