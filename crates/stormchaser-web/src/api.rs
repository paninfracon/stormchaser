use crate::models::WorkflowRunDetail;
use leptos::prelude::*;
use leptos::server_fn::codec::Json;

#[cfg(feature = "ssr")]
async fn get_cookie_header() -> Result<Option<String>, ServerFnError> {
    use axum::http::{header::COOKIE, HeaderMap};
    use leptos_axum::extract;

    let headers = extract::<HeaderMap>().await?;
    if let Some(cookie) = headers.get(COOKIE) {
        let s = cookie
            .to_str()
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(Some(s.to_string()))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "ssr")]
async fn require_auth() -> Result<String, ServerFnError> {
    match get_cookie_header().await? {
        Some(c) => {
            for part in c.split(';') {
                let part = part.trim();
                if let Some(token) = part.strip_prefix("auth_token=") {
                    return Ok(token.to_string());
                }
            }
            Err(ServerFnError::new("Unauthorized: No active session"))
        }
        None => Err(ServerFnError::new("Unauthorized: No active session")),
    }
}

#[server(input = Json, output = Json)]
pub async fn fetch_workflow_runs(
    status: Option<String>,
) -> Result<Vec<WorkflowRunDetail>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let mut url = format!("{}/api/v1/runs", api_url);
    if let Some(s) = status {
        if !s.is_empty() {
            url.push_str(&format!("?status={}", s));
        }
    }

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let runs = res
            .json::<Vec<WorkflowRunDetail>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
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
