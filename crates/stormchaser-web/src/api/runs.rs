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

/// A filter value from the UI: `None` when it is empty.
#[cfg(feature = "ssr")]
fn filter(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

#[cfg(feature = "ssr")]
fn filter_time(
    value: Option<String>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ServerFnError> {
    filter(value)
        .map(|v| {
            chrono::DateTime::parse_from_rfc3339(&v)
                .map(|t| t.with_timezone(&chrono::Utc))
                .map_err(|_| ServerFnError::new(format!("Not a valid time: {v:?}")))
        })
        .transpose()
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
    let client = authed_client().await?;
    let status = filter(status)
        .map(|s| {
            serde_json::from_value::<RunStatus>(serde_json::Value::String(s.clone()))
                .map_err(|_| ServerFnError::new(format!("Not a run status: {s:?}")))
        })
        .transpose()?;
    let query = stormchaser_model::api::ListRunsQuery {
        status,
        initiating_user: filter(initiating_user),
        workflow_name: filter(workflow_name),
        repo_url: filter(repo_url),
        workflow_path: filter(workflow_path),
        created_after: filter_time(created_after)?,
        created_before: filter_time(created_before)?,
        ..Default::default()
    };
    let runs = client
        .list_runs(&query)
        .await
        .map_err(api_error("Failed to fetch workflow runs"))?;
    Ok(runs.into_iter().map(Into::into).collect())
}

#[server(input = Json, output = Json)]
pub async fn fetch_workflow_run_detail(id: String) -> Result<WorkflowRunFullDetail, ServerFnError> {
    let id = RunId::new(super::parse_id(&id)?);
    let run = authed_client()
        .await?
        .get_run(id)
        .await
        .map_err(api_error("Failed to fetch workflow run detail"))?;
    Ok(run.into())
}

#[server(input = Json, output = Json)]
pub async fn fetch_step_logs(
    run_id: String,
    step_id: String,
    limit: Option<u32>,
) -> Result<Vec<String>, ServerFnError> {
    let run_id = RunId::new(super::parse_id(&run_id)?);
    let step_id = StepInstanceId::new(super::parse_id(&step_id)?);
    authed_client()
        .await?
        .step_logs(run_id, step_id, limit)
        .await
        .map_err(api_error("Failed to fetch step logs"))
}

/// Approves an approval step. `step_id` is the step instance's id, not its name.
#[server(input = Json, output = Json)]
pub async fn approve_step(
    run_id: String,
    step_id: String,
    inputs: serde_json::Value,
) -> Result<(), ServerFnError> {
    let run_id = RunId::new(super::parse_id(&run_id)?);
    let step_id = StepInstanceId::new(super::parse_id(&step_id)?);
    authed_client()
        .await?
        .approve(run_id, step_id, &inputs)
        .await
        .map_err(api_error("Failed to approve step"))
}

/// Rejects an approval step. `step_id` is the step instance's id, not its name.
#[server(input = Json, output = Json)]
pub async fn reject_step(run_id: String, step_id: String) -> Result<(), ServerFnError> {
    let run_id = RunId::new(super::parse_id(&run_id)?);
    let step_id = StepInstanceId::new(super::parse_id(&step_id)?);
    authed_client()
        .await?
        .reject(run_id, step_id)
        .await
        .map_err(api_error("Failed to reject step"))
}

#[server(input = Json, output = Json)]
pub async fn delete_workflow_run(run_id: String) -> Result<(), ServerFnError> {
    let run_id = RunId::new(super::parse_id(&run_id)?);
    authed_client()
        .await?
        .delete_run(run_id)
        .await
        .map_err(api_error("Failed to delete run"))
}
