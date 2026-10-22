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
/// Hydrates a run form. `schema` is what to show if the service sends nothing back; the
/// service works from the workflow that `source` names, not from anything sent here.
pub async fn hydrate_schema_complete(
    schema: serde_json::Value,
    inputs: serde_json::Value,
    source: HydrationSource,
) -> Result<(serde_json::Value, String, Vec<String>), ServerFnError> {
    use futures::stream::StreamExt;
    use stormchaser_model::api::{HydrateDirectRequest, HydrateSchemaRequest, HydrationStatus};

    let client = authed_client().await?;
    let events = match source {
        HydrationSource::Git {
            connection,
            workflow_path,
            git_ref,
        } => client
            .hydrate(&HydrateSchemaRequest {
                connection,
                workflow_path,
                git_ref,
                inputs,
            })
            .await
            .map(|events| events.boxed()),
        HydrationSource::Direct { dsl } => client
            .hydrate_direct(&HydrateDirectRequest { dsl, inputs })
            .await
            .map(|events| events.boxed()),
    };
    let mut events = events.map_err(api_error("Hydration failed"))?;

    let mut final_schema = schema;
    let mut final_status = "Update pending".to_string();
    let mut final_errors = vec![];
    // Events that cannot be read are skipped, as is everything after the stream breaks
    while let Some(event) = events.next().await {
        let Ok(event) = event else { continue };
        final_status = status_name(&event.status);
        final_schema = event.hydrated_schema;
        final_errors = event.validation_errors;
        if event.status != HydrationStatus::UpdatePending {
            break;
        }
    }
    Ok((final_schema, final_status, final_errors))
}

/// A hydration status as the query service writes it, such as `Incomplete Input`.
#[cfg(feature = "ssr")]
fn status_name(status: &stormchaser_model::api::HydrationStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}
