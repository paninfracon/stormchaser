pub mod auth;
pub mod connections;
pub mod cron;
pub mod event_rules;
pub mod runs;
pub mod webhooks;

use super::*;
use crate::AppEvent;
use anyhow::Result;
use serde_json::Value;

use std::sync::OnceLock;

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_client() -> reqwest::Client {
    HTTP_CLIENT.get_or_init(reqwest::Client::new).clone()
}

impl<'a> App<'a> {
    /// Helper method to make an API request to the backend.
    pub async fn api_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let client = get_client();
        let base_url = if path == "/api/v1/schema/hydrate" {
            &self.query_url
        } else {
            &self.url
        };
        let mut req = client.request(method, format!("{}{}", base_url, path));
        if let Some(token) = &self.token {
            req = req.header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token));
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        Ok(req.send().await?)
    }
}
