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
pub async fn fetch_connections() -> Result<Vec<Connection>, ServerFnError> {
    authed_client()
        .await?
        .list_connections()
        .await
        .map_err(api_error("Failed to fetch connections"))
}

#[server(input = Json, output = Json)]
pub async fn test_connection(
    connection_type: ConnectionType,
    config: serde_json::Value,
) -> Result<(bool, String), ServerFnError> {
    let request = stormchaser_model::api::TestConnectionRequest {
        connection_type,
        config,
        aws_assume_role_arn: None,
    };
    let response = authed_client()
        .await?
        .test_connection(&request)
        .await
        .map_err(api_error("Failed to test connection"))?;
    Ok((response.success, response.message))
}

#[server(input = Json, output = Json)]
pub async fn delete_connection(id: String) -> Result<(), ServerFnError> {
    let id = stormchaser_model::ConnectionId::new(super::parse_id(&id)?);
    authed_client()
        .await?
        .delete_connection(id)
        .await
        .map_err(api_error("Failed"))
}
