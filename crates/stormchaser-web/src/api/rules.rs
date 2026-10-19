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
pub async fn fetch_event_rules() -> Result<Vec<EventRule>, ServerFnError> {
    authed_client()
        .await?
        .list_event_rules()
        .await
        .map_err(api_error("Failed to fetch event rules"))
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
    let request = stormchaser_model::api::CreateEventRuleRequest {
        name,
        description,
        webhook_id: stormchaser_model::WebhookId::new(super::parse_id(&webhook_id)?),
        event_type_pattern,
        condition_expr,
        workflow_name,
        connection,
        workflow_path,
        git_ref,
        input_mappings,
    };
    authed_client()
        .await?
        .create_event_rule(&request)
        .await
        .map_err(api_error("Failed"))?;
    Ok(())
}

#[server(input = Json, output = Json)]
pub async fn delete_event_rule(id: String) -> Result<(), ServerFnError> {
    let id = stormchaser_model::RuleId::new(super::parse_id(&id)?);
    authed_client()
        .await?
        .delete_event_rule(id)
        .await
        .map_err(api_error("Failed"))
}
