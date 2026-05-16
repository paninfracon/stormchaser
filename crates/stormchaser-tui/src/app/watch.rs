use super::*;
use crate::AppEvent;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::header::AUTHORIZATION;
use serde_json::Value;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

impl<'a> App<'a> {
    /// Re-populates the log view buffer from the currently selected step's logs and fetches full logs.
    pub fn refresh_step_logs(&mut self, reset_scroll: bool) {
        if let Some(run) = &self.selected_run {
            if let Some(step) = run.steps.get(self.selected_step_index) {
                // Efficiently update log vector without reallocating if possible
                self.run_logs.clear();
                self.run_logs.extend_from_slice(&step.logs);

                if reset_scroll {
                    self.log_scroll = 0;
                    self.log_auto_scroll = true;
                }

                let step_id = step
                    .instance
                    .get("id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok().map(StepInstanceId::new));

                let Some(id) = step_id else {
                    // Without a stable UUID we cannot dedupe or fetch logs reliably.
                    return;
                };

                if self.fetched_steps.contains(&id) {
                    return;
                }
                self.fetched_steps.insert(id);

                // Spawn background task to fetch full historical logs for this step
                let run_id = run.detail.id;
                let url = self.url.clone();
                let token = self.token.clone();
                let tx = self.status_tx.clone();
                let step_index = self.selected_step_index;

                tokio::spawn(async move {
                    if let Some(token) = token {
                        let client = reqwest::Client::new();
                        if let Ok(res) = client
                            .get(format!(
                                "{}/api/v1/runs/{}/steps/{}/logs?limit=5000",
                                url, run_id, id
                            ))
                            .header(AUTHORIZATION, format!("Bearer {}", token))
                            .send()
                            .await
                        {
                            if res.status().is_success() {
                                if let Ok(logs) = res.json::<Vec<String>>().await {
                                    let _ = tx
                                        .send(AppEvent::StepLogsFetched(run_id, step_index, logs))
                                        .await;
                                }
                            }
                        }
                    }
                });
            } else {
                self.run_logs.clear();
                if reset_scroll {
                    self.log_scroll = 0;
                    self.log_auto_scroll = true;
                }
            }
        }
    }

    /// Spawns background tasks to listen for real-time SSE updates for a specific workflow run.
    pub async fn start_watching(&mut self, id: RunId) {
        if let Some(handle) = self.watcher_handle.take() {
            handle.abort();
        }
        if let Some(handle) = self.log_handle.take() {
            handle.abort();
        }

        if self.cached_runs.contains_key(&id) {
            // If the run is completed and cached, no need to watch for live updates
            return;
        }

        // We don't clear logs here because we want to keep the historical logs
        // from the full fetch until SSE lines arrive.

        let url = self.url.clone();
        let token = self.token.clone();
        let tx = self.status_tx.clone();

        let status_url = format!("{}/api/v1/runs/{}/status/stream", url, id);
        let log_url = format!("{}/api/v1/runs/{}/logs/stream", url, id);

        let tx_status = tx.clone();
        let token_status = token.clone();
        let status_handle = tokio::spawn(async move {
            if let Some(token) = token_status {
                let client = reqwest::Client::new();
                if let Ok(res) = client
                    .get(&status_url)
                    .header(AUTHORIZATION, format!("Bearer {}", token))
                    .send()
                    .await
                {
                    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                        let _ = tx_status.send(AppEvent::TokenExpired).await;
                        return;
                    }

                    let mut stream = res.bytes_stream().eventsource();
                    while let Some(event) = stream.next().await {
                        if let Ok(event) = event {
                            match event.event.as_str() {
                                "workflow_run" => {
                                    if let Ok(run) =
                                        serde_json::from_str::<WorkflowRunDetail>(&event.data)
                                    {
                                        let _ = tx_status.send(AppEvent::WorkflowUpdate(run)).await;
                                    }
                                }
                                "run_status" => {
                                    let _ = tx_status
                                        .send(AppEvent::StatusUpdate(id, event.data))
                                        .await;
                                }
                                "step_status" => {
                                    if let Ok(payload) = serde_json::from_str::<Value>(&event.data)
                                    {
                                        let step_name = payload["step_name"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string();
                                        let status = payload["status"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string();
                                        let _ = tx_status
                                            .send(AppEvent::StepUpdate(id, step_name, status))
                                            .await;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        });

        let tx_log = tx.clone();
        let token_log = token.clone();
        let log_handle = tokio::spawn(async move {
            if let Some(token) = token_log {
                let client = reqwest::Client::new();
                if let Ok(res) = client
                    .get(&log_url)
                    .header(AUTHORIZATION, format!("Bearer {}", token))
                    .send()
                    .await
                {
                    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
                        let _ = tx_log.send(AppEvent::TokenExpired).await;
                        return;
                    }

                    let mut stream = res.bytes_stream().eventsource();
                    while let Some(event) = stream.next().await {
                        if let Ok(event) = event {
                            if event.event == "log" {
                                let _ = tx_log.send(AppEvent::LogLine(id, event.data)).await;
                            }
                        }
                    }
                }
            }
        });

        self.watcher_handle = Some(status_handle);
        self.log_handle = Some(log_handle);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_start_watching_handles() {
        let (tx, _rx) = mpsc::channel(100);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost:3001".to_string(),
            None,
            tx,
        );

        let id = RunId::new_v4();

        // This will spawn two tokio tasks and set the handles.
        // It won't actually do anything without a server since it's just a handle to a sleeping future waiting for a response or failing fast.
        app.start_watching(id).await;

        assert!(app.watcher_handle.is_some());
        assert!(app.log_handle.is_some());

        // Calling again should abort previous and make new ones
        app.start_watching(id).await;

        assert!(app.watcher_handle.is_some());
        assert!(app.log_handle.is_some());
    }

    #[tokio::test]
    async fn test_start_watching_cached() {
        let (tx, _rx) = mpsc::channel(100);
        let mut app = App::new(
            "http://localhost".to_string(),
            "http://localhost:3001".to_string(),
            None,
            tx,
        );

        let id = RunId::new_v4();
        // Insert a dummy into cached runs
        let dummy_run = WorkflowRunFullDetail {
            detail: crate::app::WorkflowRunDetail {
                id,
                workflow_name: "test".to_string(),
                initiating_user: "u".to_string(),
                status: RunStatus::Succeeded,
                created_at: chrono::Utc::now(),
                finished_at: None,
            },
            steps: vec![],
            artifacts: vec![],
            test_summaries: vec![],
            test_cases: vec![],
        };
        app.cached_runs.insert(id, dummy_run);

        app.start_watching(id).await;

        // Because it is cached, start_watching should return early and not set handles
        assert!(app.watcher_handle.is_none());
        assert!(app.log_handle.is_none());
    }

    #[tokio::test]
    async fn test_start_watching_sse_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let id = RunId::new_v4();

        let sse_status_body = "event: run_status\ndata: {\"status\": \"running\"}\n\n";
        let sse_log_body = "event: log\ndata: [test] Log message\n\n";

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/status/stream", id)))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_status_body),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/logs/stream", id)))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse_log_body),
            )
            .mount(&server)
            .await;

        let (tx, mut rx) = mpsc::channel(100);
        let mut app = App::new(
            server.uri(),
            "http://localhost:3001".to_string(),
            Some("token".to_string()),
            tx,
        );

        app.start_watching(id).await;

        assert!(app.watcher_handle.is_some());
        assert!(app.log_handle.is_some());

        // We expect to receive StatusUpdate and LogLine events
        let mut received_status = false;
        let mut received_log = false;

        // The tasks are running in the background. We need to yield to let them make progress.
        for _ in 0..5 {
            // Allow a few iterations
            if let Ok(event) =
                tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await
            {
                match event {
                    Some(AppEvent::StatusUpdate(run_id, status)) => {
                        assert_eq!(run_id, id);
                        assert_eq!(status, "{\"status\": \"running\"}");
                        received_status = true;
                    }
                    Some(AppEvent::LogLine(run_id, log)) => {
                        assert_eq!(run_id, id);
                        assert_eq!(log, "[test] Log message");
                        received_log = true;
                    }
                    _ => {}
                }
            }
            if received_status && received_log {
                break;
            }
        }

        assert!(received_status, "Did not receive StatusUpdate event");
        assert!(received_log, "Did not receive LogLine event");
    }
}
