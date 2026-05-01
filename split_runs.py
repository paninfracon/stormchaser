import os
import re

runs_rs_path = "crates/stormchaser-cli/src/commands/runs.rs"
out_dir = "crates/stormchaser-cli/src/commands/runs"
os.makedirs(out_dir, exist_ok=True)

with open(runs_rs_path, "r") as f:
    content = f.read()

# I will write the files directly to avoid parsing complex rust AST in python.
# Actually, I can just use python strings and write the whole files directly.

mod_rs = """use crate::utils::{handle_response, handle_run_response, parse_key_val_list, require_token};
use anyhow::Result;
use clap::Subcommand;
use uuid::Uuid;

pub mod approve;
pub mod artifacts;
pub mod enqueue;
pub mod get;
pub mod list;
pub mod logs;
pub mod reports;
pub mod watch;

#[derive(Subcommand)]
pub enum RunCommands {
    /// List workflow runs
    List {
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        repo_url: Option<String>,
        #[arg(long)]
        workflow_path: Option<String>,
        #[arg(long)]
        created_after: Option<String>,
        #[arg(long)]
        created_before: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// Get run details
    Get { id: Uuid },
    /// List artifacts for a run
    Artifacts { id: Uuid },
    /// List test reports for a run
    Reports { id: Uuid },
    /// Get a specific test report content
    Report {
        id: Uuid,
        #[arg(long)]
        report_id: Uuid,
    },
    /// Stream logs for a specific step in a run
    Logs {
        id: Uuid,
        #[arg(long)]
        step_name: String,
    },
    /// Stream real-time state transition events for a run
    Watch { id: Uuid },
    /// Enqueue a workflow from a git repository
    Enqueue {
        workflow_name: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        git_ref: String,
        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,
        /// Stream all logs from the workflow run until it completes
        #[arg(long, default_value_t = false)]
        tail: bool,
        /// Stream real-time state transition events for a run until it completes
        #[arg(long, default_value_t = false)]
        watch: bool,
    },
    /// List pending approvals/events
    Pending,
    /// Approve a waiting step
    Approve {
        run_id: Uuid,
        step_id: Uuid,
        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,
    },
    /// Reject a waiting step
    Reject { run_id: Uuid, step_id: Uuid },
    /// Approve or reject a step using an encrypted token link
    ApproveLink { token: String },
}

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: RunCommands,
) -> Result<()> {
    match command {
        RunCommands::List {
            owner,
            name,
            repo_url,
            workflow_path,
            created_after,
            created_before,
            status,
        } => {
            list::list_runs(
                url,
                token,
                http_client,
                list::ListRunsFilters {
                    owner,
                    name,
                    repo_url,
                    workflow_path,
                    created_after,
                    created_before,
                    status,
                },
            )
            .await
        }
        RunCommands::Get { id } => get::get_run(url, token, http_client, id).await,
        RunCommands::Artifacts { id } => artifacts::list_artifacts(url, token, http_client, id).await,
        RunCommands::Reports { id } => reports::list_reports(url, token, http_client, id).await,
        RunCommands::Report { id, report_id } => {
            reports::get_report(url, token, http_client, id, report_id).await
        }
        RunCommands::Logs { id, step_name } => {
            logs::stream_logs(url, token, http_client, id, step_name).await
        }
        RunCommands::Watch { id } => watch::watch_run(url, token, http_client, id).await,
        RunCommands::Enqueue {
            workflow_name,
            repo,
            path,
            git_ref,
            input,
            tail,
            watch,
        } => {
            enqueue::enqueue_run(
                url,
                token,
                http_client,
                enqueue::EnqueueRunParams {
                    workflow_name,
                    repo,
                    path,
                    git_ref,
                    input,
                    tail,
                    watch,
                },
            )
            .await
        }
        RunCommands::Approve {
            run_id,
            step_id,
            input,
        } => approve::approve_step(url, token, http_client, run_id, step_id, input).await,
        RunCommands::Reject { run_id, step_id } => {
            approve::reject_step(url, token, http_client, run_id, step_id).await
        }
        RunCommands::ApproveLink { token: link_token } => {
            approve::approve_link(url, http_client, link_token).await
        }
        RunCommands::Pending => approve::list_pending(url, token, http_client).await,
    }
}
"""

list_rs = """use crate::utils::{handle_response, require_token};
use anyhow::Result;

pub struct ListRunsFilters {
    pub owner: Option<String>,
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub workflow_path: Option<String>,
    pub created_after: Option<String>,
    pub created_before: Option<String>,
    pub status: Option<String>,
}

pub async fn list_runs(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    filters: ListRunsFilters,
) -> Result<()> {
    let token = require_token(token)?;
    let url = build_list_runs_url(url, filters)?;

    let res = http_client
        .get(url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

fn build_list_runs_url(base_url: &str, filters: ListRunsFilters) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&format!("{}/api/v1/runs", base_url))?;
    if let Some(o) = filters.owner {
        url.query_pairs_mut().append_pair("initiating_user", &o);
    }
    if let Some(n) = filters.name {
        url.query_pairs_mut().append_pair("workflow_name", &n);
    }
    if let Some(r) = filters.repo_url {
        url.query_pairs_mut().append_pair("repo_url", &r);
    }
    if let Some(w) = filters.workflow_path {
        url.query_pairs_mut().append_pair("workflow_path", &w);
    }
    if let Some(ca) = filters.created_after {
        url.query_pairs_mut().append_pair("created_after", &ca);
    }
    if let Some(cb) = filters.created_before {
        url.query_pairs_mut().append_pair("created_before", &cb);
    }
    if let Some(s) = filters.status {
        url.query_pairs_mut().append_pair("status", &s);
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use serde_json::json;

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

    #[tokio::test]
    async fn test_runs_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/runs"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let filters = ListRunsFilters {
            owner: None,
            name: None,
            repo_url: None,
            workflow_path: None,
            created_after: None,
            created_before: None,
            status: None,
        };

        let result = list_runs(&server.uri(), Some("test-token"), &client, filters).await;
        assert!(result.is_ok());
    }
}
"""

get_rs = """use crate::utils::{handle_response, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn get_run(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}", url, id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use serde_json::json;

    #[tokio::test]
    async fn test_runs_get() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}", id)))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": id})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = get_run(&server.uri(), Some("test-token"), &client, id).await;
        assert!(result.is_ok());
    }
}
"""

artifacts_rs = """use crate::utils::{handle_response, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn list_artifacts(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/artifacts", url, id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use serde_json::json;

    #[tokio::test]
    async fn test_runs_artifacts() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/runs/{}/artifacts", id)))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = list_artifacts(&server.uri(), Some("test-token"), &client, id).await;
        assert!(result.is_ok());
    }
}
"""

reports_rs = """use crate::utils::{handle_response, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn list_reports(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/reports", url, id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

pub async fn get_report(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
    report_id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/reports/{}", url, id, report_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}
"""

logs_rs = """use crate::utils::{handle_response, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn stream_logs(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
    step_name: String,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!(
            "{}/api/v1/runs/{}/steps/{}/logs/stream",
            url,
            id,
            urlencoding::encode(&step_name)
        ))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !res.status().is_success() {
        return handle_response(res).await;
    }

    use eventsource_stream::Eventsource;
    use futures::stream::StreamExt;
    let mut stream = res.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", event.data);
                    break;
                }
                println!("{}", event.data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}
"""

watch_rs = """use crate::utils::{handle_response, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn watch_run(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/status/stream", url, id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !res.status().is_success() {
        return handle_response(res).await;
    }

    use eventsource_stream::Eventsource;
    use futures::stream::StreamExt;
    let mut stream = res.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", event.data);
                    break;
                }
                println!("{}: {}", event.event, event.data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}
"""

enqueue_rs = """use crate::utils::{handle_run_response, parse_key_val_list, require_token};
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
        .header("Authorization", format!("Bearer {}", token_str))
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
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_runs_enqueue() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "12345678-1234-1234-1234-123456789012",
                "status": "queued"
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
"""

approve_rs = """use crate::utils::{handle_response, parse_key_val_list, require_token};
use anyhow::Result;
use uuid::Uuid;

pub async fn approve_step(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    run_id: Uuid,
    step_id: Uuid,
    input: Vec<String>,
) -> Result<()> {
    let token = require_token(token)?;
    let inputs = parse_key_val_list(input);
    let res = http_client
        .post(format!(
            "{}/api/v1/runs/{}/steps/{}/approve",
            url, run_id, step_id
        ))
        .header("Authorization", format!("Bearer {}", token))
        .json(&inputs)
        .send()
        .await?;
    handle_response(res).await
}

pub async fn reject_step(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    run_id: Uuid,
    step_id: Uuid,
) -> Result<()> {
    let token = require_token(token)?;
    let res = http_client
        .post(format!(
            "{}/api/v1/runs/{}/steps/{}/reject",
            url, run_id, step_id
        ))
        .header("Authorization", format!("Bearer {}", token))
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
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;
    handle_response(res).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use serde_json::json;

    #[tokio::test]
    async fn test_runs_approve() {
        let server = MockServer::start().await;
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/runs/{}/steps/{}/approve",
                run_id, step_id
            )))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "approved"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let result = approve_step(&server.uri(), Some("test-token"), &client, run_id, step_id, vec![]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_runs_reject() {
        let server = MockServer::start().await;
        let run_id = Uuid::new_v4();
        let step_id = Uuid::new_v4();
        Mock::given(method("POST"))
            .and(path(format!(
                "/api/v1/runs/{}/steps/{}/reject",
                run_id, step_id
            )))
            .and(header("Authorization", "Bearer test-token"))
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
            .and(header("Authorization", "Bearer test-token"))
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
"""

with open(f"{out_dir}/mod.rs", "w") as f: f.write(mod_rs)
with open(f"{out_dir}/list.rs", "w") as f: f.write(list_rs)
with open(f"{out_dir}/get.rs", "w") as f: f.write(get_rs)
with open(f"{out_dir}/artifacts.rs", "w") as f: f.write(artifacts_rs)
with open(f"{out_dir}/reports.rs", "w") as f: f.write(reports_rs)
with open(f"{out_dir}/logs.rs", "w") as f: f.write(logs_rs)
with open(f"{out_dir}/watch.rs", "w") as f: f.write(watch_rs)
with open(f"{out_dir}/enqueue.rs", "w") as f: f.write(enqueue_rs)
with open(f"{out_dir}/approve.rs", "w") as f: f.write(approve_rs)

import shutil
os.remove(runs_rs_path)
print("done")
