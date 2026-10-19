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
pub async fn create_storage_backend(
    name: String,
    description: Option<String>,
    connection_type: ConnectionType,
    config: serde_json::Value,
    aws_assume_role_arn: Option<String>,
    is_default_sfs: bool,
) -> Result<(), ServerFnError> {
    let request = stormchaser_model::api::CreateStorageBackendRequest {
        name,
        description,
        connection_type,
        config,
        aws_assume_role_arn,
        credentials: None,
        is_default_sfs,
    };
    authed_client()
        .await?
        .create_connection(&request)
        .await
        .map_err(api_error("Failed"))?;
    Ok(())
}
