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
pub async fn fetch_webhooks() -> Result<Vec<crate::models::WebhookConfig>, ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

    let client = reqwest::Client::new();
    let url = format!("{}/api/v1/webhooks", api_url);

    let res = client
        .get(&url)
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .send()
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    if res.status().is_success() {
        let webhooks = res
            .json::<Vec<crate::models::WebhookConfig>>()
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(webhooks)
    } else {
        Err(ServerFnError::new(format!(
            "Failed to fetch webhooks: {}",
            res.status()
        )))
    }
}

#[server(input = Json, output = Json)]
pub async fn create_webhook(
    name: String,
    description: Option<String>,
    source_type: String,
    secret_token: Option<String>,
) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/api/v1/webhooks", api_url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", cookie))
        .json(&serde_json::json!({
            "name": name,
            "description": description,
            "source_type": source_type,
            "secret_token": secret_token
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

#[server(input = Json, output = Json)]
pub async fn delete_webhook(id: String) -> Result<(), ServerFnError> {
    let cookie = require_auth().await?;
    let api_url = std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let client = reqwest::Client::new();
    let res = client
        .delete(format!("{}/api/v1/webhooks/{}", api_url, id))
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
