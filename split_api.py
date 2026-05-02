import os

out_dir = "crates/stormchaser-tui/src/app/api"
os.makedirs(out_dir, exist_ok=True)

mod_rs = """pub mod auth;
pub mod runs;
pub mod storage_backends;
pub mod webhooks;

use super::*;
use crate::AppEvent;
use anyhow::Result;
use serde_json::Value;

impl<'a> App<'a> {
    /// Helper method to make an API request to the backend.
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
}
"""

auth_rs = """use super::*;
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn handle_oauth_callback(
    listener: tokio::net::TcpListener,
    tx: tokio::sync::mpsc::Sender<AppEvent>,
    url: String,
    callback_url: String,
) {
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
            if let Ok(params) = serde_urlencoded::from_str::<HashMap<String, String>>(query) {
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
                "callback_url": callback_url
            }))
            .send()
            .await
        {
            Ok(res) => {
                if res.status().is_success() {
                    if let Ok(auth_res) = res.json::<crate::app::AuthExchangeResponse>().await {
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
}

impl<'a> App<'a> {
    /// Initiates the OAuth login flow, opens a browser, and waits for the callback.
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
            handle_oauth_callback(listener, tx, url, callback_url).await;
        });

        Ok(())
    }

    /// Refreshes the access token using the stored refresh token.
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
                let auth_res: crate::app::AuthExchangeResponse = res.json().await?;
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
}
"""

runs_rs = """use super::*;
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
                                        serde_json::from_str::<crate::app::WorkflowRunDetail>(&event.data)
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
"""

storage_backends_rs = """use super::*;

impl<'a> App<'a> {
    /// Fetches the latest list of storage backends from the API.
    pub async fn refresh_storage_backends(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let res = self
            .api_request(reqwest::Method::GET, "/api/v1/storage-backends", None)
            .await?;

        if res.status().is_success() {
            self.storage_backends = res
                .json::<Vec<stormchaser_model::storage::StorageBackend>>()
                .await?;
            if !self.storage_backends.is_empty() {
                if self.storage_backends_state.selected().is_none() {
                    self.storage_backends_state.select(Some(0));
                }
                if self.selected_storage_backend.is_none() {
                    if let Some(i) = self.storage_backends_state.selected() {
                        self.selected_storage_backend = Some(self.storage_backends[i].clone());
                    }
                }
            } else {
                self.storage_backends_state.select(None);
                self.selected_storage_backend = None;
            }
            self.error = None;
        } else {
            self.error = Some(format!(
                "Failed to fetch storage backends: {}",
                res.status()
            ));
        }
        Ok(())
    }

    /// Submits the storage backend form for creation or update.
    pub async fn submit_storage_backend_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let name = self.storage_backend_inputs[0].lines().join("\\n");
        let description = self.storage_backend_inputs[1].lines().join("\\n");
        let config_str = self.storage_backend_inputs[2].lines().join("\\n");
        let role_arn = self.storage_backend_inputs[3].lines().join("\\n");

        if name.trim().is_empty() {
            self.error = Some("Name is required.".to_string());
            return Ok(());
        }

        let config: Value = match serde_json::from_str(&config_str) {
            Ok(c) => c,
            Err(_) => {
                self.error = Some("Invalid JSON configuration.".to_string());
                return Ok(());
            }
        };

        let backend_type_str = crate::app::BACKEND_TYPE_OPTIONS[self.storage_backend_type_index];
        let backend_type = match backend_type_str {
            "S3" => stormchaser_model::storage::BackendType::S3,
            "Oci" => stormchaser_model::storage::BackendType::Oci,
            "Jfrog" => stormchaser_model::storage::BackendType::Jfrog,
            "Gcs" => stormchaser_model::storage::BackendType::Gcs,
            "Azure" => stormchaser_model::storage::BackendType::Azure,
            _ => stormchaser_model::storage::BackendType::S3, // Fallback
        };

        let (method, path) = if let Some(id) = self.storage_backend_edit_id {
            (
                reqwest::Method::PATCH,
                format!("/api/v1/storage-backends/{}", id),
            )
        } else {
            (
                reqwest::Method::POST,
                "/api/v1/storage-backends".to_string(),
            )
        };

        let aws_assume_role_arn = if role_arn.trim().is_empty() {
            None::<String>
        } else {
            Some(role_arn.trim().to_string())
        };

        let payload = serde_json::json!({
            "name": name,
            "description": if description.trim().is_empty() { None::<String> } else { Some(description) },
            "backend_type": backend_type,
            "config": config,
            "aws_assume_role_arn": aws_assume_role_arn,
            "is_default_sfs": self.storage_backend_is_default
        });

        let res = self.api_request(method, &path, Some(payload)).await?;

        if res.status().is_success() {
            self.storage_backend_dialog_active = false;
            self.refresh_storage_backends().await?;
        } else {
            self.error = Some(format!("Failed to save storage backend: {}", res.status()));
        }

        Ok(())
    }

    /// Deletes the currently selected storage backend.
    pub async fn delete_selected_storage_backend(&mut self) -> Result<()> {
        if let Some(backend) = &self.selected_storage_backend {
            if self.token.is_some() {
                let res = self
                    .api_request(
                        reqwest::Method::DELETE,
                        &format!("/api/v1/storage-backends/{}", backend.id),
                        None,
                    )
                    .await?;

                if res.status().is_success() {
                    self.refresh_storage_backends().await?;
                } else {
                    self.error = Some(format!(
                        "Failed to delete storage backend: {}",
                        res.status()
                    ));
                }
            }
        }
        Ok(())
    }
}
"""

webhooks_rs = """use super::*;

impl<'a> App<'a> {
    /// Fetches the latest list of webhooks from the API.
    pub async fn refresh_webhooks(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let res = self
            .api_request(reqwest::Method::GET, "/api/v1/webhooks", None)
            .await?;

        if res.status().is_success() {
            self.webhooks = res
                .json::<Vec<stormchaser_model::event_rules::WebhookConfig>>()
                .await?;
            if !self.webhooks.is_empty() {
                if self.webhooks_state.selected().is_none() {
                    self.webhooks_state.select(Some(0));
                }
                if self.selected_webhook.is_none() {
                    if let Some(i) = self.webhooks_state.selected() {
                        self.selected_webhook = Some(self.webhooks[i].clone());
                    }
                }
            } else {
                self.webhooks_state.select(None);
                self.selected_webhook = None;
            }
            self.error = None;
        } else {
            self.error = Some(format!("Failed to fetch webhooks: {}", res.status()));
        }
        Ok(())
    }

    /// Submits the webhook form for creation or update.
    pub async fn submit_webhook_form(&mut self) -> Result<()> {
        if self.token.is_none() {
            return Ok(());
        }

        let name = self.webhook_inputs[0].lines().join("\\n");
        let description = self.webhook_inputs[1].lines().join("\\n");
        let secret_token = self.webhook_inputs[2].lines().join("\\n");

        if name.trim().is_empty() {
            self.error = Some("Name is required.".to_string());
            return Ok(());
        }

        let source_type =
            crate::app::WEBHOOK_SOURCE_TYPE_OPTIONS[self.webhook_source_type_index].to_string();

        let (method, path) = if let Some(id) = self.webhook_edit_id {
            (reqwest::Method::PATCH, format!("/api/v1/webhooks/{}", id))
        } else {
            (reqwest::Method::POST, "/api/v1/webhooks".to_string())
        };

        let payload = serde_json::json!({
            "name": name,
            "description": if description.trim().is_empty() { None::<String> } else { Some(description) },
            "source_type": source_type,
            "secret_token": if secret_token.trim().is_empty() { None::<String> } else { Some(secret_token) },
            "is_active": self.webhook_is_active,
        });

        let res = self.api_request(method, &path, Some(payload)).await?;

        if res.status().is_success() {
            self.webhook_dialog_active = false;
            self.refresh_webhooks().await?;
        } else {
            self.error = Some(format!("Failed to save webhook: {}", res.status()));
        }

        Ok(())
    }

    /// Deletes the currently selected webhook.
    pub async fn delete_selected_webhook(&mut self) -> Result<()> {
        if let Some(webhook) = &self.selected_webhook {
            if self.token.is_some() {
                let res = self
                    .api_request(
                        reqwest::Method::DELETE,
                        &format!("/api/v1/webhooks/{}", webhook.id),
                        None,
                    )
                    .await?;

                if res.status().is_success() {
                    self.refresh_webhooks().await?;
                } else {
                    self.error = Some(format!("Failed to delete webhook: {}", res.status()));
                }
            }
        }
        Ok(())
    }
}
"""

with open(f"{out_dir}/mod.rs", "w") as f: f.write(mod_rs)
with open(f"{out_dir}/auth.rs", "w") as f: f.write(auth_rs)
with open(f"{out_dir}/runs.rs", "w") as f: f.write(runs_rs)
with open(f"{out_dir}/storage_backends.rs", "w") as f: f.write(storage_backends_rs)
with open(f"{out_dir}/webhooks.rs", "w") as f: f.write(webhooks_rs)

os.remove("crates/stormchaser-tui/src/app/api.rs")
print("done")
