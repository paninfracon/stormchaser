#[allow(unused_imports)]
#[cfg(feature = "ssr")]
use super::require_auth;
#[allow(unused_imports)]
use super::*;
#[allow(unused_imports)]
use crate::models::*;
#[allow(unused_imports)]
use leptos::prelude::*;
#[allow(unused_imports)]
use leptos::server_fn::codec::Json;

#[server(input = Json, output = Json)]
pub async fn fetch_workflow_runs(
    status: Option<String>,
    initiating_user: Option<String>,
    workflow_name: Option<String>,
    repo_url: Option<String>,
    workflow_path: Option<String>,
    created_after: Option<String>,
    created_before: Option<String>,
) -> Result<Vec<WorkflowRunDetail>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/runs", api_url);

    let mut query = Vec::new();
    if let Some(s) = status {
        if !s.is_empty() {
            query.push(("status", s));
        }
    }
    if let Some(s) = initiating_user {
        if !s.is_empty() {
            query.push(("initiating_user", s));
        }
    }
    if let Some(s) = workflow_name {
        if !s.is_empty() {
            query.push(("workflow_name", s));
        }
    }
    if let Some(s) = repo_url {
        if !s.is_empty() {
            query.push(("repo_url", s));
        }
    }
    if let Some(s) = workflow_path {
        if !s.is_empty() {
            query.push(("workflow_path", s));
        }
    }
    if let Some(s) = created_after {
        if !s.is_empty() {
            query.push(("created_after", s));
        }
    }
    if let Some(s) = created_before {
        if !s.is_empty() {
            query.push(("created_before", s));
        }
    }

    let res = client
        .get(&url)
        .query(&query)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let text = res
            .text()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        println!("RAW RUNS JSON: {}", text);
        let runs = serde_json::from_str::<Vec<WorkflowRunDetail>>(&text).map_err(|e| {
            println!("DESERIALIZATION ERROR: {}", e);
            ServerFnError::new(e.to_string())
        })?;
        Ok(runs)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch workflow runs: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn fetch_workflow_run_detail(
    id: String,
) -> Result<crate::models::WorkflowRunFullDetail, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/runs/{}", api_url, id);

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let run = res
            .json::<crate::models::WorkflowRunFullDetail>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(run)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch workflow run detail: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn fetch_step_logs(
    run_id: String,
    step_id: String,
    limit: Option<usize>,
) -> Result<Vec<String>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let mut url = format!("{}/api/v1/runs/{}/steps/{}/logs", api_url, run_id, step_id);
    if let Some(l) = limit {
        url.push_str(&format!("?limit={}", l));
    }

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let logs = res
            .json::<Vec<String>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(logs)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch step logs: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn approve_step(
    run_id: String,
    step_name: String,
    inputs: serde_json::Value,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v1/runs/{}/steps/{}/approve",
        api_url, run_id, step_name
    );

    let res = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&inputs)
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(ServerFnError::new(format!(
            "Failed to approve step: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn reject_step(run_id: String, step_name: String) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/v1/runs/{}/steps/{}/reject",
        api_url, run_id, step_name
    );

    let res = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(ServerFnError::new(format!(
            "Failed to reject step: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn delete_workflow_run(run_id: String) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/runs/{}", api_url, run_id);

    let res = client
        .delete(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(ServerFnError::new(format!(
            "Failed to delete run: {}",
            res.status()
        )))
    }
}
