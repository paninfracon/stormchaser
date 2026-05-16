use crate::secrets::SharedSecretBackend;
use anyhow::Result;
use hcl::eval::{Context as HclContext, FuncDef, ParamType};
use hcl::Value as HclValue;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde_json::Value;
use stormchaser_model::RunId;

pub use stormchaser_model::hcl_eval::{
    evaluate_raw_expr, evaluate_string, hcl_to_json, json_to_hcl, resolve_expressions,
};

static SECRETS_BACKEND: Lazy<RwLock<Option<SharedSecretBackend>>> = Lazy::new(|| RwLock::new(None));

/// Sets the global secrets backend used for resolving `secrets.*` references in HCL expressions.
pub fn set_secrets_backend(backend: SharedSecretBackend) {
    let mut lock = SECRETS_BACKEND.write();
    *lock = Some(backend);
}

fn secret_lookup(args: hcl::eval::FuncArgs) -> Result<HclValue, String> {
    if args.len() != 1 {
        return Err("secret() expects exactly 1 argument".to_string());
    }

    let path_and_key_str = match &args[0] {
        HclValue::String(s) => s.to_string(),
        _ => return Err("secret() expects a string argument".to_string()),
    };

    let backend = {
        let lock = SECRETS_BACKEND.read();
        lock.clone()
    };

    if let Some(backend) = backend {
        let handle = tokio::runtime::Handle::current();

        let (path, key) = match path_and_key_str.split_once('#') {
            Some((p, k)) => (p.to_string(), k.to_string()),
            None => {
                return Err(format!(
                    "Invalid secret lookup format (expected path#key): {}",
                    path_and_key_str
                ));
            }
        };

        let val = match handle.runtime_flavor() {
            tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(move || {
                handle.block_on(async move { backend.get_secret(&path, &key).await })
            }),
            _ => handle.block_on(async move { backend.get_secret(&path, &key).await }),
        };

        match val {
            Ok(val) => Ok(HclValue::String(val)),
            Err(e) => Err(format!(
                "Secret lookup failed for {}: {:?}",
                path_and_key_str, e
            )),
        }
    } else {
        Err("No secrets backend configured".to_string())
    }
}

/// Create context.
pub fn create_context(
    inputs: Value,
    run_id: RunId,
    secrets: Value,
    steps: Value,
    workflow: Option<&stormchaser_dsl::ast::Workflow>,
    step: Option<&stormchaser_dsl::ast::Step>,
) -> HclContext<'static> {
    let mut ctx = HclContext::new();
    ctx.declare_var("inputs", json_to_hcl(inputs));
    ctx.declare_var("secrets", json_to_hcl(secrets));
    ctx.declare_var(
        "run",
        json_to_hcl(serde_json::json!({"id": run_id.to_string()})),
    );
    ctx.declare_var("steps", json_to_hcl(steps));

    ctx.declare_func("secret", FuncDef::new(secret_lookup, [ParamType::Any]));
    crate::stdlib::register_stdlib(&mut ctx);

    // Apply workflow aliases
    if let Some(wf) = workflow {
        for (alias, expr_str) in &wf.aliases {
            let val = match evaluate_string(expr_str, &ctx) {
                Ok(Some(v)) => v,
                Ok(None) => Value::String(expr_str.clone()),
                Err(e) => {
                    tracing::warn!("Failed to evaluate workflow alias '{}': {:?}", alias, e);
                    Value::String(expr_str.clone())
                }
            };
            ctx.declare_var(alias.clone(), json_to_hcl(val));
        }
    }

    // Apply step aliases
    if let Some(st) = step {
        for (alias, expr_str) in &st.aliases {
            let val = match evaluate_string(expr_str, &ctx) {
                Ok(Some(v)) => v,
                Ok(None) => Value::String(expr_str.clone()),
                Err(e) => {
                    tracing::warn!("Failed to evaluate step alias '{}': {:?}", alias, e);
                    Value::String(expr_str.clone())
                }
            };
            ctx.declare_var(alias.clone(), json_to_hcl(val));
        }
    }

    ctx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MockBackend;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[test]
    fn test_secret_function() {
        let mut secrets = HashMap::new();
        secrets.insert("kv/db:password".to_string(), "secret-password".to_string());
        let backend = Arc::new(MockBackend::new(secrets)) as SharedSecretBackend;
        set_secrets_backend(backend);

        let ctx = create_context(
            serde_json::json!({}),
            RunId::new_v4(),
            serde_json::json!({}),
            serde_json::json!({}),
            None,
            None,
        );

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let result = evaluate_raw_expr("secret(\"kv/db#password\")", &ctx).unwrap();
            assert_eq!(result, serde_json::json!("secret-password"));
        });
    }
}
