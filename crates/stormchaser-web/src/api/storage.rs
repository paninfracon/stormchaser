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
