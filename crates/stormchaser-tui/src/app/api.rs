use super::*;
use crate::AppEvent;
use anyhow::Result;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::sleep;
use uuid::Uuid;

impl<'a> App<'a> {
    pub async fn api_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let client = reqwest::Client::new();
        let mut req = client.request(method, format!("{}{}", self.url, path));
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        Ok(req.send().await?)
    }

    pub async fn login(&mut self) -> Result<()> {
        self.state = AppState::LoggingIn;
        self.error = None;
        let port = 8080;
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
        let callback_url = format!("http://localhost:{}/callback", port);

        let login_url = format!(
            "{}/api/v1/auth/login?callback_url={}",
            self.url,
            urlencoding::encode(&callback_url)
        );

        if let Err(e) = open::that(&login_url) {
            self.error = Some(format!("Failed to open browser: {}", e));
            return Ok(());
        }

        let url = self.url.clone();
        let tx = self.status_tx.clone();

        tokio::spawn(async move {
            let callback_url_val = callback_url.clone();
            let (mut stream, _) = match tokio::time::timeout(
                Duration::from_secs(300), // 5 minute timeout for user to login
                listener.accept(),
            )
            .await
            {
                Ok(Ok(s)) => s,
                _ => {
                    let _ = tx
                        .send(AppEvent::LoginFailed(
                            "Login timed out or failed to accept connection".to_string(),
                        ))
                        .await;
                    return;
                }
            };

            let mut buf = [0; 4096];
            let mut request_str = String::new();
            if let Ok(n) = stream.read(&mut buf).await {
                request_str = String::from_utf8_lossy(&buf[0..n]).to_string();
            }

            let mut code = None;
            if let Some(path) = request_str.split_whitespace().nth(1) {
                if let Some(query) = path.split('?').nth(1) {
                    if let Ok(params) = serde_urlencoded::from_str::<HashMap<String, String>>(query)
                    {
                        code = params.get("code").cloned();
                    }
                }
            }

            // Respond to browser
            let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Login Successful</h1><p>You can close this window now.</p><script>window.close();</script></body></html>";
            let _ = stream.write_all(response.as_bytes()).await;

            if let Some(code) = code {
                let client = reqwest::Client::new();
                match client
                    .post(format!("{}/api/v1/auth/exchange", url))
                    .json(&serde_json::json!({
                        "sso_token": code,
                        "callback_url": callback_url_val
                    }))
                    .send()
                    .await
                {
                    Ok(res) => {
                        if res.status().is_success() {
                            if let Ok(auth_res) = res.json::<AuthExchangeResponse>().await {
                                let _ = tx
                                    .send(AppEvent::LoginSuccessful(
                                        auth_res.access_token,
                                        auth_res.refresh_token,
                                    ))
                                    .await;
                            } else {
                                let _ = tx
                                    .send(AppEvent::LoginFailed(
                                        "Failed to parse token response".to_string(),
                                    ))
                                    .await;
                            }
                        } else {
                            let _ = tx
                                .send(AppEvent::LoginFailed(format!(
                                    "Token exchange failed: {}",
                                    res.status()
                                )))
                                .await;
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(AppEvent::LoginFailed(format!("Request failed: {}", e)))
                            .await;
                    }
                }
            } else {
                let _ = tx
                    .send(AppEvent::LoginFailed(
                        "No authorization code received from provider".to_string(),
                    ))
                    .await;
            }
        });

        Ok(())
    }

    pub async fn refresh_session(&mut self) -> Result<bool> {
        if let Some(refresh_token) = self.refresh_token.clone() {
            let res = self
                .api_request(
                    reqwest::Method::POST,
                    "/api/v1/auth/refresh",
                    Some(serde_json::json!({ "refresh_token": refresh_token })),
                )
                .await?;

            if res.status().is_success() {
                let auth_res: AuthExchangeResponse = res.json().await?;
                self.token = Some(auth_res.access_token);
                if auth_res.refresh_token.is_some() {
                    self.refresh_token = auth_res.refresh_token;
                }
                self.state = AppState::LoggedIn;
                return Ok(true);
            }
        }

        self.token = None;
        self.refresh_token = None;
        self.state = AppState::LoggedOut;
        Ok(false)
    }

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
            self.runs = res.json::<Vec<WorkflowRunDetail>>().await?;
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
            let full_detail = res.json::<WorkflowRunFullDetail>().await?;
            self.handle_full_run_update(full_detail);
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch run detail: {}", res.status()));
        }
        Ok(())
    }

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
                                        serde_json::from_str::<WorkflowRunDetail>(&event.data)
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
