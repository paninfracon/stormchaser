//! OPA WebAssembly Integration Module.
//!
//! Provides execution capabilities for OPA policies compiled to WebAssembly, and chooses
//! the policy a service enforces.

use anyhow::{Context, Result};
use async_trait::async_trait;
use opa_wasm::Runtime;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use stormchaser_model::auth::{OpaClient, OpaWasmExecutor};
use wasmtime::*;

/// The shipped policy, `deploy/opa/policy.rego`, compiled by `scripts/build-opa-policy.sh`.
/// Services enforce it when no other policy is configured.
pub const BUILTIN_POLICY: &[u8] = include_bytes!("../policy/stormchaser.wasm");

/// Represents an instantiated OPA WASM engine ready for evaluation.
pub struct OpaWasmInstance {
    engine: Engine,
    module: Module,
    data: Value,
}

impl OpaWasmInstance {
    /// Creates a new `OpaWasmInstance` from the given WASM module bytes.
    pub fn new(module_bytes: &[u8]) -> Result<Self> {
        let config = Config::new();
        // config.async_support(true); // No longer needed in wasmtime 43.0
        let engine = Engine::new(&config)?;
        let module = Module::new(&engine, module_bytes)?;
        Ok(Self {
            engine,
            module,
            data: Value::Object(Default::default()),
        })
    }

    /// Sets the policy's `data` document, which holds its deployment settings.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

#[async_trait]
impl OpaWasmExecutor for OpaWasmInstance {
    async fn evaluate(&self, entrypoint: &str, input: &Value) -> Result<bool> {
        let mut store = Store::new(&self.engine, ());
        let runtime = Runtime::new(&mut store, &self.module)
            .await
            .context("Failed to initialize OPA WASM runtime")?;

        let policy = runtime.with_data(&mut store, &self.data).await?;

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

/// Where a service's authorization policy comes from.
#[derive(Debug, Clone, Default)]
pub struct PolicyConfig {
    /// `OPA_URL`: an OPA server to ask.
    pub url: Option<String>,
    /// `OPA_WASM_PATH`: a policy compiled to WebAssembly. Used in preference to `url`.
    pub wasm_path: Option<PathBuf>,
    /// `OPA_DATA_PATH`: a JSON file with the `data` document for a WebAssembly policy,
    /// such as `deploy/opa/settings.json`. An OPA server loads its own.
    pub data_path: Option<PathBuf>,
    /// `OPA_ENTRYPOINT`: the rule to evaluate, `stormchaser/allow` by default.
    pub entrypoint: Option<String>,
    /// `STORMCHASER_AUTHZ_DISABLED`: enforce no policy at all. Debug builds only.
    pub disabled: bool,
}

/// Builds the policy client a service enforces: a WebAssembly policy from
/// `wasm_path`, else the OPA server at `url`, else the built-in policy.
///
/// There is always a policy. Only a debug build started with `disabled` runs without
/// one; a release build refuses to start that way.
pub fn policy_client(
    config: &PolicyConfig,
    tls_config: Option<Arc<rustls::ClientConfig>>,
) -> Result<OpaClient> {
    if config.disabled {
        anyhow::ensure!(
            cfg!(debug_assertions),
            "STORMCHASER_AUTHZ_DISABLED is honoured only by debug builds; a release build always enforces a policy"
        );
        tracing::warn!(
            "Authorization is disabled (STORMCHASER_AUTHZ_DISABLED=true): any authenticated user may do anything"
        );
        return Ok(OpaClient::new(None, tls_config));
    }

    let mut client = OpaClient::new(config.url.clone(), tls_config);

    if config.wasm_path.is_some() || config.url.is_none() {
        let instance = match &config.wasm_path {
            Some(path) => {
                tracing::info!("Loading OPA WASM policy from {:?}", path);
                let bytes = std::fs::read(path)
                    .with_context(|| format!("Failed to read OPA WASM policy {:?}", path))?;
                OpaWasmInstance::new(&bytes)?
            }
            None => {
                tracing::info!("No OPA_URL or OPA_WASM_PATH is set; enforcing the built-in policy");
                OpaWasmInstance::new(BUILTIN_POLICY)
                    .context("Failed to load the built-in policy")?
            }
        };
        client = client.with_wasm_executor(Arc::new(instance.with_data(policy_data(config)?)));
    } else if config.data_path.is_some() {
        tracing::warn!("OPA_DATA_PATH is ignored with OPA_URL: the OPA server loads its own data");
    }

    if let Some(entrypoint) = &config.entrypoint {
        client = client.with_entrypoint(entrypoint.clone());
    }

    Ok(client)
}

fn policy_data(config: &PolicyConfig) -> Result<Value> {
    let Some(path) = &config.data_path else {
        return Ok(Value::Object(Default::default()));
    };
    let bytes =
        std::fs::read(path).with_context(|| format!("Failed to read OPA data {:?}", path))?;
    let data: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("OPA data {:?} is not valid JSON", path))?;
    anyhow::ensure!(
        data.is_object(),
        "OPA data {:?} must be a JSON object",
        path
    );
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};

    #[test]
    fn test_opa_wasm_instance_new_invalid_bytes() {
        let result = OpaWasmInstance::new(b"not a wasm module");
        let _err = result.err().unwrap();
    }

    #[test]
    fn test_builtin_policy_is_built_from_the_shipped_policy() {
        let source = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../deploy/opa/policy.rego"
        ))
        .unwrap();
        let built_from = include_str!("../policy/stormchaser.wasm.source-sha256");
        assert_eq!(
            hex::encode(Sha256::digest(&source)),
            built_from.trim(),
            "deploy/opa/policy.rego changed; run scripts/build-opa-policy.sh"
        );
    }

    async fn builtin_allows(data: Value, input: Value) -> bool {
        OpaWasmInstance::new(BUILTIN_POLICY)
            .unwrap()
            .with_data(data)
            .evaluate("stormchaser/allow", &input)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_builtin_policy_lets_only_a_verified_user_write() {
        let alice = json!({"sub": "alice-id", "email": "alice@paninfracon.net", "groups": []});
        let write =
            |subject: Value| json!({"method": "POST", "path": "/api/v1/runs", "subject": subject});

        assert!(builtin_allows(json!({}), write(alice)).await);
        assert!(!builtin_allows(json!({}), write(Value::Null)).await);
        assert!(!builtin_allows(json!({}), json!({})).await);
    }

    #[tokio::test]
    async fn test_builtin_policy_reads_its_settings_from_data() {
        let bob =
            json!({"sub": "bob-id", "email": "bob@paninfracon.net", "groups": ["platform-admins"]});
        let delete_alices_run = json!({
            "action": "delete",
            "resource": {"type": "run", "id": "1", "owner": "alice@paninfracon.net"},
            "subject": bob,
        });

        assert!(!builtin_allows(json!({}), delete_alices_run.clone()).await);
        let settings = json!({"stormchaser_settings": {"admin_groups": ["platform-admins"]}});
        assert!(builtin_allows(settings, delete_alices_run).await);
    }

    #[test]
    fn test_policy_client_defaults_to_the_builtin_policy() {
        let client = policy_client(&PolicyConfig::default(), None).unwrap();
        assert!(client.is_configured());

        let client = policy_client(
            &PolicyConfig {
                url: Some("http://127.0.0.1:8181/v1/data/stormchaser/allow".into()),
                ..Default::default()
            },
            None,
        )
        .unwrap();
        assert!(client.is_configured());
    }

    #[test]
    fn test_policy_client_disabled_only_in_debug_builds() {
        let config = PolicyConfig {
            disabled: true,
            ..Default::default()
        };
        let result = policy_client(&config, None);
        assert_eq!(result.is_ok(), cfg!(debug_assertions));
        if let Ok(client) = result {
            assert!(!client.is_configured());
        }
    }

    #[test]
    fn test_policy_data_must_be_a_json_object() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "[]").unwrap();
        let config = PolicyConfig {
            data_path: Some(path),
            ..Default::default()
        };
        policy_client(&config, None).unwrap_err();
    }
}
