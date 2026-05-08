use crate::handler::{fetch_inputs, fetch_step_instance};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

#[allow(clippy::too_many_arguments)]
/// Attempts to dispatch a WebAssembly (WASM) module execution step instance.
pub async fn try_dispatch(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    step_type: &str,
    resolved_spec: &Value,
    resolved_params: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    let wasm_def: Option<(String, String, Value)> =
        crate::db::get_wasm_step_definition(&pool, step_type).await?;

    let (module, function, wasm_config) = if let Some((m, f, c)) = wasm_def {
        (m, f, c)
    } else if step_type == "Wasm" {
        let m = resolved_spec["module"]
            .as_str()
            .context("Missing module in Wasm step spec")?
            .to_string();
        let f = resolved_spec["function"]
            .as_str()
            .unwrap_or("run")
            .to_string();
        (m, f, Value::Null)
    } else {
        ("".to_string(), "".to_string(), Value::Null)
    };

    if !module.is_empty() {
        let pool = pool.clone();
        let nats_client = nats_client.clone();
        let spec = resolved_spec.clone();
        let params = resolved_params.clone();
        let inputs = fetch_inputs(run_id, &pool).await?;

        tokio::spawn(async move {
            let executor = crate::wasm::WasmExecutor::new();
            if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await {
                let machine = crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(instance);
                if let Ok(mut conn) = pool.acquire().await {
                    let _ = machine.start("wasm".to_string(), &mut *conn).await;
                }
            }

            let input = serde_json::json!({
                "spec": spec,
                "params": params,
                "inputs": inputs,
                "config": wasm_config,
            });

            match executor.execute(&module, &function, input).await {
                Ok(outputs) => {
                    let event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_instance_id,
                        "event_type": "step_completed",
                        "outputs": outputs,
                        "timestamp": Utc::now(),
                    });
                    let js = async_nats::jetstream::new(nats_client);
                    let _ = stormchaser_model::nats::publish_cloudevent(
                        &js,
                        "stormchaser.v1.step.completed",
                        "stormchaser.v1.step.completed",
                        "/stormchaser",
                        serde_json::to_value(event).unwrap(),
                        Some("1.0"),
                        None,
                    )
                    .await;
                }
                Err(e) => {
                    let event = StepFailedEvent {
                        run_id,
                        step_id: step_instance_id,
                        event_type: "stormchaser.v1.step.failed".to_string(),
                        error: format!("WASM execution failed: {:?}", e),
                        runner_id: None,
                        exit_code: None,
                        storage_hashes: None,
                        artifacts: None,
                        test_reports: None,
                        outputs: None,
                        timestamp: Utc::now(),
                    };
                    let js = async_nats::jetstream::new(nats_client);
                    let _ = stormchaser_model::nats::publish_cloudevent(
                        &js,
                        "stormchaser.v1.step.failed",
                        "stormchaser.v1.step.failed",
                        "/stormchaser",
                        serde_json::to_value(event).unwrap(),
                        Some("1.0"),
                        None,
                    )
                    .await;
                }
            }
        });
        return Ok(true);
    }

    Ok(false)
}
