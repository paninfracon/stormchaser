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
pub async fn fetch_webhooks() -> Result<Vec<WebhookConfig>, ServerFnError> {
    authed_client()
        .await?
        .list_webhooks()
        .await
        .map_err(api_error("Failed to fetch webhooks"))
}

#[server(input = Json, output = Json)]
pub async fn create_webhook(
    name: String,
    description: Option<String>,
    source_type: String,
    secret_token: Option<String>,
) -> Result<(), ServerFnError> {
    let request = stormchaser_model::api::CreateWebhookRequest {
        name,
        description,
        source_type,
        secret_token,
    };
    authed_client()
        .await?
        .create_webhook(&request)
        .await
        .map_err(api_error("Failed"))?;
    Ok(())
}

#[server(input = Json, output = Json)]
pub async fn delete_webhook(id: String) -> Result<(), ServerFnError> {
    let id = stormchaser_model::WebhookId::new(super::parse_id(&id)?);
    authed_client()
        .await?
        .delete_webhook(id)
        .await
        .map_err(api_error("Failed"))
}
