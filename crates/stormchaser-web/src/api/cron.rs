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
pub async fn fetch_cron_workflows() -> Result<Vec<CronWorkflow>, ServerFnError> {
    authed_client()
        .await?
        .list_cron_workflows()
        .await
        .map_err(api_error("Failed to fetch cron workflows"))
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
    let request = stormchaser_model::api::CreateCronWorkflowRequest {
        name,
        description,
        cronspec,
        workflow_name,
        connection,
        workflow_path,
        git_ref,
        inputs,
    };
    authed_client()
        .await?
        .create_cron_workflow(&request)
        .await
        .map_err(api_error("Failed"))?;
    Ok(())
}

#[server(input = Json, output = Json)]
pub async fn delete_cron_workflow(id: String) -> Result<(), ServerFnError> {
    let id = stormchaser_model::CronWorkflowId::new(super::parse_id(&id)?);
    authed_client()
        .await?
        .delete_cron_workflow(id)
        .await
        .map_err(api_error("Failed"))
}
