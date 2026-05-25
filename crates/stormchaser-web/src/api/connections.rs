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
pub async fn fetch_connections() -> Result<Vec<crate::models::Connection>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = super::http_client();
    let url = format!("{}/api/v1/connections", api_url);

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let connections = res
            .json::<Vec<crate::models::Connection>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(connections)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch connections: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn test_connection(
    connection_type: stormchaser_model::connections::ConnectionType,
    config: serde_json::Value,
) -> Result<(bool, String), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = super::http_client();
    let url = format!("{}/api/v1/connections/test", api_url);

    let res = client
        .post(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "connection_type": connection_type,
            "config": config
        }))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        #[derive(serde::Deserialize)]
        struct TestResp {
            success: bool,
            message: String,
        }
        let resp = res
            .json::<TestResp>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok((resp.success, resp.message))
    } else {
        Err(ServerFnError::new(format!(
            "Failed to test connection: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn delete_connection(id: String) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = super::http_client();
    let res = client
        .delete(format!("{}/api/v1/connections/{}", api_url, id))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err(ServerFnError::new(format!("Failed: {}", res.status())))
    }
}
