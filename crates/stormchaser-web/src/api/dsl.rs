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
    let workflow_name = workflow_path
        .split('/')
        .next_back()
        .unwrap_or("manual_run")
        .to_string();
    let request = stormchaser_model::api::EnqueueRequest {
        workflow_name,
        connection,
        workflow_path,
        git_ref,
        inputs,
        overrides: None,
        sops_file: None,
        sops_role_arn: None,
    };
    authed_client()
        .await?
        .enqueue(&request)
        .await
        .map_err(api_error("Failed to submit run"))?;
    Ok(())
}

#[server(input = Json, output = Json)]
pub async fn submit_run_direct(
    dsl: String,
    inputs: serde_json::Value,
) -> Result<String, ServerFnError> {
    let request = stormchaser_model::api::DirectRunRequest { dsl, inputs };
    let response = authed_client()
        .await?
        .run_direct(&request)
        .await
        .map_err(api_error("Failed to submit run"))?;
    Ok(response.run_id.to_string())
}

#[server(input = Json, output = Json)]
pub async fn fetch_dsl_from_git(
    connection: String,
    workflow_path: String,
    git_ref: String,
) -> Result<String, ServerFnError> {
    let request = stormchaser_model::api::ParseGitRequest {
        connection,
        workflow_path,
        git_ref,
    };
    let text = authed_client()
        .await?
        .parse_git(&request)
        .await
        .map_err(api_error("Failed to fetch DSL from git"))?;
    tracing::info!("fetch_dsl_from_git returned DSL of length {}", text.len());
    Ok(text)
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
