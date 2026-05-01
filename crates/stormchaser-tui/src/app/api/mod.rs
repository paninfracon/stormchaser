pub mod auth;
pub mod runs;
pub mod storage_backends;
pub mod webhooks;

use super::*;
use crate::AppEvent;
use anyhow::Result;
use serde_json::Value;

impl<'a> App<'a> {
    /// Helper method to make an API request to the backend.
    pub async fn api_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let client = reqwest::Client::new();
        let mut req = client.request(method, format!("{}{}", self.url, path));
        if let Some(token) = &self.token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        if let Some(body) = body {
            req = req.json(&body);
        }
        Ok(req.send().await?)
    }
}
