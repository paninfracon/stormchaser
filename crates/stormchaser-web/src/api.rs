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
pub async fn check_auth() -> Result<bool, ServerFnError> {
    let _ = require_auth().await?;
    Ok(true)
}

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

#[server(input = Json, output = Json)]
pub async fn fetch_connections() -> Result<Vec<crate::models::Connection>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/connections", api_url);

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let connections = res
            .json::<Vec<crate::models::Connection>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(connections)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch connections: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn fetch_webhooks() -> Result<Vec<crate::models::WebhookConfig>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/webhooks", api_url);

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let webhooks = res
            .json::<Vec<crate::models::WebhookConfig>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(webhooks)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch webhooks: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn fetch_event_rules() -> Result<Vec<crate::models::EventRule>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
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

#[server(input = Json, output = Json)]
pub async fn fetch_cron_workflows() -> Result<Vec<crate::models::CronWorkflow>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
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

#[server(input = Json, output = Json)]
pub async fn submit_run_git(
    repo_url: String,
    workflow_path: String,
    git_ref: String,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/runs/git", api_url);

    let res = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "repo_url": repo_url,
            "workflow_path": workflow_path,
            "git_ref": git_ref,
            "inputs": {}
        }))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(ServerFnError::new(format!(
            "Failed to submit run: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn submit_run_direct(
    dsl: String,
    inputs: serde_json::Value,
) -> Result<String, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/runs/direct", api_url);

    let res = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "dsl": dsl,
            "inputs": inputs
        }))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let body = res
            .json::<serde_json::Value>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        if let Some(id) = body.get("run_id").and_then(|v| v.as_str()) {
            Ok(id.to_string())
        } else {
            Ok("".to_string())
        }
    } else {
        Err(ServerFnError::new(format!(
            "Failed to submit run: {}",
            res.status()
        )))
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ParseDslResult {
    pub inputs_schema: Option<serde_json::Value>,
    pub inputs: serde_json::Value,
    pub queries: Option<serde_json::Value>,
    pub inputs_view: Option<stormchaser_model::dsl::InputView>,
}

#[server(input = Json, output = Json)]
pub async fn parse_dsl(dsl: String) -> Result<ParseDslResult, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use stormchaser_dsl::StormchaserParser;
        match StormchaserParser.parse(&dsl) {
            Ok(workflow) => {
                let initial_inputs = serde_json::json!({});
                let queries_val = serde_json::to_value(&workflow.queries).ok();

                let inputs_schema = if let Some(schema) = workflow.inputs_schema {
                    Some(schema)
                } else if !workflow.inputs.is_empty() {
                    let mut properties = serde_json::Map::new();
                    for input in workflow.inputs {
                        let mut prop = serde_json::Map::new();
                        prop.insert("type".to_string(), serde_json::json!("string"));
                        prop.insert(
                            "description".to_string(),
                            serde_json::json!(input.description.as_deref().unwrap_or(&input.name)),
                        );
                        if let Some(default) = &input.default {
                            if let Some(s) = default.as_str() {
                                prop.insert("default".to_string(), serde_json::json!(s));
                            } else {
                                prop.insert(
                                    "default".to_string(),
                                    serde_json::json!(default.to_string()),
                                );
                            }
                        }
                        properties.insert(input.name, serde_json::Value::Object(prop));
                    }
                    Some(serde_json::json!({
                        "type": "object",
                        "title": "Workflow Inputs",
                        "properties": properties
                    }))
                } else {
                    None
                };

                Ok(ParseDslResult {
                    inputs_schema,
                    inputs: initial_inputs,
                    queries: queries_val,
                    inputs_view: workflow.inputs_view,
                })
            }
            Err(e) => Err(ServerFnError::new(format!("Parse error: {:?}", e))),
        }
    }
    #[cfg(not(feature = "ssr"))]
    {
        unreachable!("This should only run on the server")
    }
}

#[server(input = Json, output = Json)]
pub async fn hydrate_schema_complete(
    schema: serde_json::Value,
    inputs: serde_json::Value,
    queries: Option<serde_json::Value>,
) -> Result<(serde_json::Value, String, Vec<String>), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let payload = if let Some(q) = queries {
        serde_json::json!({ "schema": schema, "inputs": inputs, "queries": q })
    } else {
        serde_json::json!({ "schema": schema, "inputs": inputs })
    };

    let res = client
        .post(format!("{}/api/v1/schema/hydrate", api_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&payload)
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        use futures::stream::StreamExt;
        let mut byte_stream = res.bytes_stream();
        let mut buffer = String::new();

        let mut final_schema = schema.clone();
        let mut final_status = "Update pending".to_string();
        let mut final_errors = vec![];

        while let Some(item) = byte_stream.next().await {
            if let Ok(chunk) = item {
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(idx) = buffer.find("\n\n") {
                    let event_str = buffer[..idx].to_string();
                    buffer = buffer[idx + 2..].to_string();

                    if let Some(data_idx) = event_str.find("data: ") {
                        let json_str = &event_str[data_idx + 6..];
                        if let Ok(event) = serde_json::from_str::<serde_json::Value>(json_str) {
                            if let Some(status) = event.get("status").and_then(|s| s.as_str()) {
                                final_status = status.to_string();
                                if let Some(hydrated) = event.get("hydrated_schema") {
                                    final_schema = hydrated.clone();
                                }
                                if let Some(errors) =
                                    event.get("validation_errors").and_then(|e| e.as_array())
                                {
                                    final_errors = errors
                                        .iter()
                                        .filter_map(|e| e.as_str().map(|s| s.to_string()))
                                        .collect();
                                }

                                if status == "Completed"
                                    || status == "Incomplete Input"
                                    || status == "Schema validation failed"
                                {
                                    return Ok((final_schema, final_status, final_errors));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok((final_schema, final_status, final_errors))
    } else {
        Err(ServerFnError::new(format!(
            "Hydration failed: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn create_storage_backend(
    name: String,
    description: Option<String>,
    connection_type: stormchaser_model::connections::ConnectionType,
    config: serde_json::Value,
    aws_assume_role_arn: Option<String>,
    is_default_sfs: bool,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/api/v1/connections", api_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "name": name,
            "description": description,
            "connection_type": connection_type,
            "config": config,
            "aws_assume_role_arn": aws_assume_role_arn,
            "is_default_sfs": is_default_sfs
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
pub async fn create_cron_workflow(
    name: String,
    description: Option<String>,
    cronspec: String,
    workflow_name: String,
    repo_url: String,
    workflow_path: String,
    git_ref: String,
    inputs: serde_json::Value,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/api/v1/cron", api_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "name": name,
            "description": description,
            "cronspec": cronspec,
            "workflow_name": workflow_name,
            "repo_url": repo_url,
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
pub async fn create_event_rule(
    name: String,
    description: Option<String>,
    webhook_id: String,
    event_type_pattern: String,
    condition_expr: Option<String>,
    workflow_name: String,
    repo_url: String,
    workflow_path: String,
    git_ref: String,
    input_mappings: std::collections::HashMap<String, String>,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = reqwest::Client::new();
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
            "repo_url": repo_url,
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
pub async fn create_webhook(
    name: String,
    description: Option<String>,
    source_type: String,
    secret_token: Option<String>,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/api/v1/webhooks", api_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "name": name,
            "description": description,
            "source_type": source_type,
            "secret_token": secret_token
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
