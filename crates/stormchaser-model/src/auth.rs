use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String, // User ID
    pub email: Option<String>,
    pub exp: usize, // Expiration time
}

#[async_trait]
pub trait OpaWasmExecutor: Send + Sync {
    async fn evaluate(&self, entrypoint: &str, input: &Value) -> Result<bool>;
}

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

    pub fn with_wasm_executor(mut self, executor: Arc<dyn OpaWasmExecutor>) -> Self {
        self.wasm_executor = Some(executor);
        self
    }

    pub fn with_entrypoint(mut self, entrypoint: String) -> Self {
        self.entrypoint = entrypoint;
        self
    }

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

#[async_trait]
pub trait OpaAuthorizer: Send + Sync {
    async fn check(&self, context: ApiOpaContext<'_>) -> Result<bool>;
    fn is_configured(&self) -> bool;
}

#[async_trait]
impl OpaAuthorizer for OpaClient {
    async fn check(&self, context: ApiOpaContext<'_>) -> Result<bool> {
        self.check_context(context).await
    }
    fn is_configured(&self) -> bool {
        self.is_configured()
    }
}

/// Context for OPA checks in the API
#[derive(Debug, Serialize)]
pub struct ApiOpaContext<'a> {
    pub path: &'a str,
    pub method: &'a str,
    pub token: Option<&'a str>,
}

/// Context for OPA checks in the Engine after DSL parsing
#[derive(Debug, Serialize)]
pub struct EngineOpaContext {
    pub run_id: Uuid,
    pub initiating_user: String,
    pub workflow_ast: Value,
    pub inputs: Value,
}
