use crate::handler::integrations::utils::{report_step_completed, report_step_failed};
use crate::handler::step::dispatch::intrinsic::{BuiltInStep, BuiltInWork};
use crate::handler::{fetch_inputs, fetch_step_instance};
use anyhow::{Context, Result};
use futures::FutureExt;
use serde_json::Value;
use std::collections::HashMap;

/// Step types the engine and its runners implement themselves.
const BUILTIN_STEP_TYPES: [&str; 23] = [
    "Approval",
    "EmailSend",
    "GitCheckout",
    "JQ",
    "JinjaRender",
    "LambdaInvoke",
    "Parallel",
    "RestApi",
    "RunContainer",
    "RunK8sJob",
    "Sequential",
    "SlackMessage",
    "SqlExecute",
    "TeamsMessage",
    "TerraformApply",
    "TerraformApproval",
    "TerraformPlan",
    "TestReportEmail",
    "Wait",
    "WaitEvent",
    "Wasm",
    "WebhookInvoke",
    "DryRun",
];

/// True for step types that a registered WASM definition must not take over.
pub(crate) fn is_builtin_step_type(step_type: &str) -> bool {
    BUILTIN_STEP_TYPES
        .iter()
        .any(|builtin| builtin.eq_ignore_ascii_case(step_type))
}

/// The work of a Wasm step, or of a step whose type is registered with a WASM module.
pub(crate) async fn work(step: &BuiltInStep) -> Result<Option<BuiltInWork>> {
    // A registered WASM definition may add a step type but never replace a built-in one.
    let wasm_def: Option<(String, String, Value)> = if is_builtin_step_type(&step.step_type) {
        None
    } else {
        crate::db::get_wasm_step_definition(&step.pool, &step.step_type).await?
    };

    let (module, function, wasm_config) = if let Some(def) = wasm_def {
        def
    } else if step.step_type == "Wasm" {
        let module = step.spec["module"]
            .as_str()
            .context("Missing module in Wasm step spec")?
            .to_string();
        let function = step.spec["function"].as_str().unwrap_or("run").to_string();
        (module, function, Value::Null)
    } else {
        return Ok(None);
    };

    let inputs = fetch_inputs(step.run_id, &step.pool).await?;
    let input = serde_json::json!({
        "spec": step.spec,
        "params": step.params,
        "inputs": inputs,
        "config": wasm_config,
    });
    Ok(Some(
        execute_wasm_step(step.clone(), module, function, input).boxed(),
    ))
}

async fn execute_wasm_step(
    step: BuiltInStep,
    module: String,
    function: String,
    input: Value,
) -> Result<()> {
    let BuiltInStep {
        run_id,
        step_id: step_instance_id,
        fencing_token,
        pool,
        ..
    } = step;
    let executor = crate::wasm::WasmExecutor::new();
    if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await {
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
                instance,
            );
        if let Ok(mut conn) = pool.acquire().await {
            let _ = machine.start("wasm".to_string(), &mut *conn).await;
        }
    }

    match executor.execute(&module, &function, input).await {
        Ok(outputs) => {
            let outputs = match outputs {
                Value::Object(m) => m.into_iter().collect::<HashMap<String, Value>>(),
                _ => HashMap::new(),
            };
            report_step_completed(
                &mut *pool.acquire().await?,
                run_id,
                step_instance_id,
                fencing_token,
                Some(outputs),
            )
            .await
        }
        Err(e) => {
            report_step_failed(
                &mut *pool.acquire().await?,
                run_id,
                step_instance_id,
                fencing_token,
                format!("WASM execution failed: {:?}", e),
            )
            .await
        }
    }
}

#[cfg(test)]
mod builtin_tests {
    use super::is_builtin_step_type;

    #[test]
    fn builtin_step_types_cannot_be_shadowed() {
        for step_type in [
            "RunContainer",
            "Approval",
            "runcontainer",
            "RestApi",
            "Wasm",
        ] {
            assert!(is_builtin_step_type(step_type), "{}", step_type);
        }
        assert!(!is_builtin_step_type("MyCustomScanner"));
    }
}
