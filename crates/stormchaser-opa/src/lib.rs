//! OPA WebAssembly Integration Module.
//!
//! Provides execution capabilities for OPA policies compiled to WebAssembly.

use anyhow::{Context, Result};
use async_trait::async_trait;
use opa_wasm::Runtime;
use serde_json::Value;
use stormchaser_model::auth::OpaWasmExecutor;
use wasmtime::*;

/// Represents an instantiated OPA WASM engine ready for evaluation.
pub struct OpaWasmInstance {
    engine: Engine,
    module: Module,
}

impl OpaWasmInstance {
    /// Creates a new `OpaWasmInstance` from the given WASM module bytes.
    pub fn new(module_bytes: &[u8]) -> Result<Self> {
        let config = Config::new();
        // config.async_support(true); // No longer needed in wasmtime 43.0
        let engine = Engine::new(&config)?;
        let module = Module::new(&engine, module_bytes)?;
        Ok(Self { engine, module })
    }
}

#[async_trait]
impl OpaWasmExecutor for OpaWasmInstance {
    async fn evaluate(&self, entrypoint: &str, input: &Value) -> Result<bool> {
        let mut store = Store::new(&self.engine, ());
        let runtime = Runtime::new(&mut store, &self.module)
            .await
            .context("Failed to initialize OPA WASM runtime")?;

        let policy = runtime.without_data(&mut store).await?;

        let result: Value = policy
            .evaluate(&mut store, entrypoint, input)
            .await
            .context("Failed to evaluate OPA policy")?;

        tracing::debug!("OPA WASM result: {:?}", result);

        if let Some(arr) = result.as_array() {
            if let Some(first) = arr.first() {
                if let Some(b) = first.as_bool() {
                    return Ok(b);
                }
                if let Some(obj) = first.as_object() {
                    if let Some(res) = obj.get("result").and_then(|v| v.as_bool()) {
                        return Ok(res);
                    }
                }
            }
        }

        if let Some(b) = result.as_bool() {
            return Ok(b);
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opa_wasm_instance_new_invalid_bytes() {
        let result = OpaWasmInstance::new(b"not a wasm module");
        assert!(result.is_err());
    }
}
