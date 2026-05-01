//! Authentication and authorization models and OPA client.

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

/// Extracted claims from a JWT token.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// Subject (User ID) of the token.
    pub sub: String, // User ID
    /// Optional email address of the user.
    pub email: Option<String>,
    /// Expiration time as a Unix timestamp.
    pub exp: usize, // Expiration time
}

/// Trait for executing Open Policy Agent (OPA) policies compiled to WebAssembly.
#[async_trait]
pub trait OpaWasmExecutor: Send + Sync {
    /// Evaluates a WASM-compiled OPA policy against the given input.
    async fn evaluate(&self, entrypoint: &str, input: &Value) -> Result<bool>;
}

/// Client for interacting with an Open Policy Agent (OPA) server or WASM module.
#[derive(Clone)]
pub struct OpaClient {
    url: Option<String>,
    http_client: ClientWithMiddleware,
    wasm_executor: Option<Arc<dyn OpaWasmExecutor>>,
    entrypoint: String,
}

impl std::fmt::Debug for OpaClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpaClient")
            .field("url", &self.url)
            .field("wasm_configured", &self.wasm_executor.is_some())
            .field("entrypoint", &self.entrypoint)
            .finish()
    }
}

#[derive(Debug, Serialize)]
struct OpaInput<T> {
    input: T,
}

#[derive(Debug, Deserialize)]
struct OpaResponse {
    result: bool,
}

impl OpaClient {
    /// Creates a new OpaClient with the given URL and TLS configuration.
    pub fn new(url: Option<String>, tls_config: Option<Arc<rustls::ClientConfig>>) -> Self {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
        let mut builder = reqwest::Client::builder();

        if let Some(config) = tls_config {
            builder = builder.use_preconfigured_tls(config);
        }

        let http_client =
            ClientBuilder::new(builder.build().unwrap_or_else(|_| reqwest::Client::new()))
                .with(RetryTransientMiddleware::new_with_policy(retry_policy))
                .build();

        Self {
            url,
            http_client,
            wasm_executor: None,
            entrypoint: "stormchaser/allow".to_string(),
        }
    }

    /// Configures the client to use a WASM executor.
    pub fn with_wasm_executor(mut self, executor: Arc<dyn OpaWasmExecutor>) -> Self {
        self.wasm_executor = Some(executor);
        self
    }

    /// Sets the entrypoint for the OPA policy.
    pub fn with_entrypoint(mut self, entrypoint: String) -> Self {
        self.entrypoint = entrypoint;
        self
    }

    /// Returns true if the client is configured with either a URL or a WASM executor.
    pub fn is_configured(&self) -> bool {
        self.url.is_some() || self.wasm_executor.is_some()
    }

    /// Flexible check that sends any serializable context to OPA
    pub async fn check_context<T: Serialize>(&self, context: T) -> Result<bool> {
        // 1. Try WASM executor first if configured
        if let Some(executor) = &self.wasm_executor {
            let context_val = serde_json::to_value(&context)?;
            return executor.evaluate(&self.entrypoint, &context_val).await;
        }

        // 2. Fallback to HTTP OPA if configured
        let url = match &self.url {
            Some(url) => url,
            None => return Ok(true), // No-op if not configured
        };

        debug!("Checking OPA policy at {} with custom context", url);

        let input = OpaInput { input: context };

        let response = self
            .http_client
            .post(url)
            .json(&input)
            .send()
            .await
            .context("Failed to reach OPA server")?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "OPA server returned error: {}",
                response.status()
            ));
        }

        let opa_res: OpaResponse = response
            .json()
            .await
            .context("Failed to parse OPA response")?;

        Ok(opa_res.result)
    }
}

/// Trait for authorizing requests using an Open Policy Agent (OPA).
#[async_trait]
pub trait OpaAuthorizer: Send + Sync {
    /// Checks the given context against the OPA policy.
    async fn check(&self, context: ApiOpaContext<'_>) -> Result<bool>;
    /// Checks an approval context against the OPA policy.
    async fn check_approval(&self, context: ApprovalOpaContext<'_>) -> Result<bool>;
    /// Returns true if the authorizer is properly configured.
    fn is_configured(&self) -> bool;
}

#[async_trait]
impl OpaAuthorizer for OpaClient {
    async fn check(&self, context: ApiOpaContext<'_>) -> Result<bool> {
        self.check_context(context).await
    }
    async fn check_approval(&self, context: ApprovalOpaContext<'_>) -> Result<bool> {
        self.check_context(context).await
    }
    fn is_configured(&self) -> bool {
        self.is_configured()
    }
}

/// Context for OPA checks in the API
#[derive(Debug, Serialize)]
pub struct ApiOpaContext<'a> {
    /// The requested path.
    pub path: &'a str,
    /// The HTTP method.
    pub method: &'a str,
    /// The optional authentication token.
    pub token: Option<&'a str>,
}

/// Context for OPA checks in the Engine after DSL parsing
#[derive(Debug, Serialize)]
pub struct EngineOpaContext {
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Identifier of the user who initiated the run.
    pub initiating_user: String,
    /// Full parsed abstract syntax tree of the workflow.
    pub workflow_ast: Value,
    /// JSON inputs for the workflow run.
    pub inputs: Value,
}

/// Context for OPA checks during HITL Approvals
#[derive(Debug, Serialize)]
pub struct ApprovalOpaContext<'a> {
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Identifier of the user who initiated the run.
    pub initiating_user: String,
    /// The parsed abstract syntax tree of the approval step.
    pub step_ast: Value,
    /// JSON inputs for the workflow run.
    pub inputs: Value,
    /// All outputs generated by previous steps in the run.
    pub run_outputs: Value,
    /// The optional authentication token of the approver.
    pub token: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opa_client_config() {
        let client = OpaClient::new(None, None);
        assert!(!client.is_configured());
        assert!(!OpaAuthorizer::is_configured(&client));

        let client = OpaClient::new(Some("http://localhost:8181".to_string()), None);
        assert!(client.is_configured());
        assert_eq!(client.entrypoint, "stormchaser/allow");

        let client = client.with_entrypoint("custom/allow".to_string());
        assert_eq!(client.entrypoint, "custom/allow");
    }

    #[test]
    fn test_opa_client_debug() {
        let client = OpaClient::new(Some("http://localhost:8181".to_string()), None);
        let debug_str = format!("{:?}", client);
        assert!(debug_str.contains("OpaClient"));
        assert!(debug_str.contains("url: Some(\"http://localhost:8181\")"));
        assert!(debug_str.contains("wasm_configured: false"));
    }
}
