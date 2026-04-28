use super::*;
use crate::AppEvent;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde_json::Value;
use uuid::Uuid;

impl<'a> App<'a> {
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
            } else {
                self.run_logs.clear();
                if reset_scroll {
                    self.log_scroll = 0;
                    self.log_auto_scroll = true;
                }
            }
        }
    }

    pub async fn start_watching(&mut self, id: Uuid) {
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
                    .header("Authorization", format!("Bearer {}", token))
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
                    .header("Authorization", format!("Bearer {}", token))
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
