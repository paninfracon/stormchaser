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
pub async fn fetch_cron_workflows() -> Result<Vec<crate::models::CronWorkflow>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = super::http_client();
    let url = format!("{}/api/v1/cron-workflows", api_url);

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let crons = res
            .json::<Vec<crate::models::CronWorkflow>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(crons)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch cron workflows: {}",
            res.status()
        )))
    }
}

#[allow(clippy::too_many_arguments)]
#[server(input = Json, output = Json)]
pub async fn create_cron_workflow(
    name: String,
    description: Option<String>,
    cronspec: String,
    workflow_name: String,
    connection: String,
    workflow_path: String,
    git_ref: String,
    inputs: serde_json::Value,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = super::http_client();
    let res = client
        .post(format!("{}/api/v1/cron", api_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "name": name,
            "description": description,
            "cronspec": cronspec,
            "workflow_name": workflow_name,
            "connection": connection,
            "workflow_path": workflow_path,
            "git_ref": git_ref,
            "inputs": inputs
        }))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(ServerFnError::new(format!("Failed: {}", res.status())))
    }
}

#[allow(clippy::too_many_arguments)]
#[server(input = Json, output = Json)]
pub async fn delete_cron_workflow(id: String) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = super::http_client();
    let res = client
        .delete(format!("{}/api/v1/cron-workflows/{}", api_url, id))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(ServerFnError::new(format!("Failed: {}", res.status())))
    }
}
