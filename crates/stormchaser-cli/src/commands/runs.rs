use crate::utils::{handle_response, handle_run_response, parse_key_val_list, require_token};
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;
use uuid::Uuid;

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

pub struct ListRunsFilters {
    pub owner: Option<String>,
    pub name: Option<String>,
    pub repo_url: Option<String>,
    pub workflow_path: Option<String>,
    pub created_after: Option<String>,
    pub created_before: Option<String>,
    pub status: Option<String>,
}

pub struct EnqueueRunParams {
    pub workflow_name: String,
    pub repo: String,
    pub path: String,
    pub git_ref: String,
    pub input: Vec<String>,
    pub tail: bool,
    pub watch: bool,
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
            list_runs(
                url,
                token,
                http_client,
                ListRunsFilters {
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
        RunCommands::Get { id } => get_run(url, token, http_client, id).await,
        RunCommands::Artifacts { id } => list_artifacts(url, token, http_client, id).await,
        RunCommands::Reports { id } => list_reports(url, token, http_client, id).await,
        RunCommands::Report { id, report_id } => {
            get_report(url, token, http_client, id, report_id).await
        }
        RunCommands::Logs { id, step_name } => {
            stream_logs(url, token, http_client, id, step_name).await
        }
        RunCommands::Watch { id } => watch_run(url, token, http_client, id).await,
        RunCommands::Enqueue {
            workflow_name,
            repo,
            path,
            git_ref,
            input,
            tail,
            watch,
        } => {
            enqueue_run(
                url,
                token,
                http_client,
                EnqueueRunParams {
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
        } => approve_step(url, token, http_client, run_id, step_id, input).await,
        RunCommands::Reject { run_id, step_id } => {
            reject_step(url, token, http_client, run_id, step_id).await
        }
        RunCommands::ApproveLink { token: link_token } => {
            approve_link(url, http_client, link_token).await
        }
        RunCommands::Pending => list_pending(url, token, http_client).await,
    }
}

async fn list_runs(
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

async fn get_run(
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

async fn list_artifacts(
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

async fn list_reports(
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

async fn get_report(
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

async fn stream_logs(
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

async fn watch_run(
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

async fn enqueue_run(
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

async fn approve_step(
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

async fn reject_step(
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

async fn approve_link(
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

async fn list_pending(
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
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("initiating_user").unwrap(), "alice");
        assert_eq!(query.get("workflow_name").unwrap(), "my-workflow");
        assert_eq!(query.get("status").unwrap(), "Succeeded");
        assert!(!query.contains_key("repo_url"));
    }
}
