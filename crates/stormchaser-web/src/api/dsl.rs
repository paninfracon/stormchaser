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
pub async fn submit_run_git(
    connection: String,
    workflow_path: String,
    git_ref: String,
    inputs: serde_json::Value,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = super::http_client();
    let url = format!("{}/api/v1/runs", api_url);

    let workflow_name = workflow_path
        .split('/')
        .next_back()
        .unwrap_or("manual_run")
        .to_string();

    let res = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
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

    let client = super::http_client();
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

#[server(input = Json, output = Json)]
pub async fn fetch_dsl_from_git(
    connection: String,
    workflow_path: String,
    git_ref: String,
) -> Result<String, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = super::http_client();
    let url = format!("{}/api/v1/schema/parse-git", api_url);

    let res = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "connection": connection,
            "workflow_path": workflow_path,
            "git_ref": git_ref
        }))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let text = res
            .text()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        tracing::info!("fetch_dsl_from_git returned DSL of length {}", text.len());
        Ok(text)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch DSL from git: {}",
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
        super::require_session().await?;
        use stormchaser_dsl::StormchaserParser;
        // Parsing is CPU work; keep it off the threads that serve requests
        let parsed = tokio::task::spawn_blocking(move || StormchaserParser.parse(&dsl))
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        match parsed {
            Ok(workflow) => {
                let initial_inputs = serde_json::json!({});
                let queries_val = serde_json::to_value(&workflow.queries).ok();

                // The same derivation the query service uses when it hydrates this form
                let inputs_schema = stormchaser_dsl::form::inputs_form_schema(&workflow);

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
