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
pub async fn fetch_event_rules() -> Result<Vec<crate::models::EventRule>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = super::http_client();
    let url = format!("{}/api/v1/rules", api_url);

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let rules = res
            .json::<Vec<crate::models::EventRule>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(rules)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch event rules: {}",
            res.status()
        )))
    }
}

#[allow(clippy::too_many_arguments)]
#[server(input = Json, output = Json)]
pub async fn create_event_rule(
    name: String,
    description: Option<String>,
    webhook_id: String,
    event_type_pattern: String,
    condition_expr: Option<String>,
    workflow_name: String,
    connection: String,
    workflow_path: String,
    git_ref: String,
    input_mappings: std::collections::HashMap<String, String>,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = super::http_client();
    let res = client
        .post(format!("{}/api/v1/rules", api_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "name": name,
            "description": description,
            "webhook_id": webhook_id,
            "event_type_pattern": event_type_pattern,
            "condition_expr": condition_expr,
            "workflow_name": workflow_name,
            "connection": connection,
            "workflow_path": workflow_path,
            "git_ref": git_ref,
            "input_mappings": input_mappings
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

#[server(input = Json, output = Json)]
pub async fn delete_event_rule(id: String) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = super::http_client();
    let res = client
        .delete(format!("{}/api/v1/rules/{}", api_url, id))
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
