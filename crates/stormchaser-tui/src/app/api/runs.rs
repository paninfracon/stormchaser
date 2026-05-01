use super::*;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

impl<'a> App<'a> {
    /// Fetches the latest list of workflow runs from the API based on active filters.
    pub async fn refresh_runs(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let mut query_params = Vec::new();
        if let Some(owner) = &self.filter_owner {
            query_params.push(format!("initiating_user={}", owner));
        }
        if let Some(name) = &self.filter_name {
            query_params.push(format!("workflow_name={}", name));
        }
        if let Some(repo) = &self.filter_repo_url {
            query_params.push(format!("repo_url={}", repo));
        }
        if let Some(path) = &self.filter_workflow_path {
            query_params.push(format!("workflow_path={}", path));
        }
        if let Some(ca) = &self.filter_created_after {
            query_params.push(format!("created_after={}", ca));
        }
        if let Some(cb) = &self.filter_created_before {
            query_params.push(format!("created_before={}", cb));
        }
        if let Some(status) = &self.filter_status {
            query_params.push(format!("status={}", status));
        }

        let query = if query_params.is_empty() {
            String::new()
        } else {
            format!("?{}", query_params.join("&"))
        };

        let res = self
            .api_request(
                reqwest::Method::GET,
                &format!("/api/v1/runs{}", query),
                None,
            )
            .await?;

        if res.status().is_success() {
            self.runs = res.json::<Vec<crate::app::WorkflowRunDetail>>().await?;
            if !self.runs.is_empty() {
                if self.runs_state.selected().is_none() {
                    self.runs_state.select(Some(0));
                }
                if self.selected_run.is_none() {
                    if let Some(i) = self.runs_state.selected() {
                        let id = self.runs[i].id;
                        let _ = self.fetch_run_detail(id).await;
                        self.start_watching(id).await;
                    }
                }
            }
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch runs: {}", res.status()));
        }
        Ok(())
    }

    /// Fetches the full details for a specific workflow run.
    pub async fn fetch_run_detail(&mut self, run_id: Uuid) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }
        if let Some(cached) = self.cached_runs.get(&run_id).cloned() {
            self.handle_full_run_update(cached);
            self.error = None;
            return Ok(());
        }
        let res = self
            .api_request(
                reqwest::Method::GET,
                &format!("/api/v1/runs/{}", run_id),
                None,
            )
            .await?;

        if res.status().is_success() {
            let full_detail = res.json::<crate::app::WorkflowRunFullDetail>().await?;
            self.handle_full_run_update(full_detail);
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch run detail: {}", res.status()));
        }
        Ok(())
    }

    /// Starts a background task to listen for global workflow run updates via SSE.
    pub async fn start_listening_for_workflows(&mut self) {
        if let Some(handle) = self.workflow_handle.take() {
            handle.abort();
        }
        let url = self.url.clone();
        let tx = self.status_tx.clone();
        let token = self.token.clone();

        let workflow_handle = tokio::spawn(async move {
            loop {
                if let Some(token) = &token {
                    let client = reqwest::Client::new();
                    if let Ok(res) = client
                        .get(format!("{}/api/v1/runs/stream", url))
                        .header("Authorization", format!("Bearer {}", token))
                        .send()
                        .await
                    {
                        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                            let _ = tx.send(AppEvent::TokenExpired).await;
                            return;
                        }

                        let mut stream = res.bytes_stream().eventsource();
                        while let Some(event) = stream.next().await {
                            if let Ok(event) = event {
                                if event.event == "workflow_run" {
                                    if let Ok(run) =
                                        serde_json::from_str::<crate::app::WorkflowRunDetail>(
                                            &event.data,
                                        )
                                    {
                                        let _ = tx.send(AppEvent::WorkflowUpdate(run)).await;
                                    }
                                }
                            }
                        }
                    }
                }
                sleep(Duration::from_secs(2)).await;
            }
        });
        self.workflow_handle = Some(workflow_handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use stormchaser_model::workflow::RunStatus;
    use tokio::sync::mpsc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_refresh_runs_success() {
        let server = MockServer::start().await;

        let run_detail = crate::app::WorkflowRunDetail {
            id: Uuid::new_v4(),
            workflow_name: "test".to_string(),
            initiating_user: "user".to_string(),
            status: RunStatus::Succeeded,
            created_at: Utc::now(),
            finished_at: None,
        };

        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vec![run_detail.clone()]))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);

        let result = app.refresh_runs().await;
        assert!(result.is_ok());
        assert_eq!(app.runs.len(), 1);
        assert_eq!(app.runs[0].id, run_detail.id);
        assert!(app.error.is_none());
    }

    #[tokio::test]
    async fn test_refresh_runs_filters() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(Vec::<crate::app::WorkflowRunDetail>::new()),
            )
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);
        app.filter_owner = Some("test_user".to_string());
        app.filter_status = Some("failed".to_string());

        let result = app.refresh_runs().await;
        assert!(result.is_ok());

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .url
            .query()
            .unwrap()
            .contains("initiating_user=test_user"));
        assert!(requests[0].url.query().unwrap().contains("status=failed"));
    }

    #[tokio::test]
    async fn test_fetch_run_detail_success() {
        let server = MockServer::start().await;
        let run_id = Uuid::new_v4();

        let full_detail = crate::app::WorkflowRunFullDetail {
            detail: crate::app::WorkflowRunDetail {
                id: run_id,
                workflow_name: "test".to_string(),
                initiating_user: "user".to_string(),
                status: RunStatus::Succeeded,
                created_at: Utc::now(),
                finished_at: None,
            },
            steps: vec![],
            artifacts: vec![],
            test_summaries: vec![],
            test_cases: vec![],
        };

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}", run_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(&full_detail))
            .mount(&server)
            .await;

        let (tx, _rx) = mpsc::channel(1);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);
        app.runs.push(full_detail.detail.clone());
        app.runs_state.select(Some(0));

        let result = app.fetch_run_detail(run_id).await;
        assert!(result.is_ok());
        assert!(app.selected_run.is_some());
        assert_eq!(app.selected_run.unwrap().detail.id, run_id);
    }

    #[tokio::test]
    async fn test_start_listening_for_workflows_sse_success() {
        let server = MockServer::start().await;

        let run_detail = crate::app::WorkflowRunDetail {
            id: Uuid::new_v4(),
            workflow_name: "test-workflow".to_string(),
            initiating_user: "user".to_string(),
            status: RunStatus::Running,
            created_at: Utc::now(),
            finished_at: None,
        };

        let sse_body = format!(
            "event: workflow_run\ndata: {}\n\n",
            serde_json::to_string(&run_detail).unwrap()
        );

        Mock::given(method("GET"))
            .and(path("/api/v1/runs/stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_body),
            )
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(100);
        let mut app = App::new(server.uri(), Some("token".to_string()), tx);

        app.start_listening_for_workflows().await;

        assert!(app.workflow_handle.is_some());

        // We expect to receive a WorkflowUpdate event
        if let Ok(Some(event)) =
            tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
        {
            match event {
                AppEvent::WorkflowUpdate(received_run) => {
                    assert_eq!(received_run.id, run_detail.id);
                    assert_eq!(received_run.status, RunStatus::Running);
                }
                _ => panic!("Expected WorkflowUpdate event"),
            }
        } else {
            panic!("Did not receive WorkflowUpdate event from SSE stream in time");
        }
    }
}
