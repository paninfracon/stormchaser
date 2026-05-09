use crate::handler::fetch_step_instance;
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, StepEventType};
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;

use stormchaser_model::dsl;

/// Mutate if has files.
pub fn mutate_if_has_files(step_type: &mut String, resolved_spec: &mut Value) {
    if step_type == "JQ" {
        let jq_spec: Result<dsl::JqSpec, _> =
            serde_json::from_value(resolved_spec.get("spec").unwrap_or(&*resolved_spec).clone());

        let has_files = match &jq_spec {
            Ok(jq) => jq.input_file.is_some() || jq.output_file.is_some(),
            Err(_) => false,
        };

        if has_files {
            let jq = jq_spec.unwrap();
            let input_file = jq.input_file.unwrap_or_default();
            let output_file = jq.output_file.unwrap_or_default();

            let mut script = format!("jq -c '{}'", jq.program.replace('\'', "'\\''"));
            if !input_file.is_empty() {
                script.push_str(&format!(" {}", input_file));
            }
            if !output_file.is_empty() {
                script.push_str(&format!(" > {}", output_file));
            }

            let container_spec = dsl::CommonContainerSpec {
                image: "ghcr.io/jqlang/jq:latest".to_string(),
                command: Some(vec!["sh".to_string(), "-c".to_string(), script]),
                args: None,
                env: None,
                cpu: None,
                memory: None,
                privileged: None,
                storage_mounts: jq.storage_mounts,
            };

            *step_type = "RunContainer".to_string();
            if let Ok(val) = serde_json::to_value(container_spec) {
                *resolved_spec = val;
            }
        }
    }
}

/// Attempts to dispatch a jq step instance directly if it operates on strings (instead of files).
pub async fn try_dispatch(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    step_type: &str,
    resolved_spec: &Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if step_type == "JQ" {
        let pool = pool.clone();
        let nats_client = nats_client.clone();
        let spec = resolved_spec.clone();

        tokio::spawn(async move {
            let _ = dispatch_jq_internal(run_id, step_instance_id, spec, pool, nats_client).await;
        });
        return Ok(true);
    }

    Ok(false)
}

async fn dispatch_jq_internal(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    // 1. Transition to running
    if let Ok(instance) = fetch_step_instance(step_instance_id, &pool).await {
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
                instance,
            );
        if let Ok(mut conn) = pool.acquire().await {
            let _ = machine.start("intrinsic-jq".to_string(), &mut *conn).await;
        }
    }

    let actual_spec = spec.get("spec").unwrap_or(&spec).clone();
    let jq_spec: Result<dsl::JqSpec, _> = serde_json::from_value(actual_spec.clone());

    let result = match jq_spec {
        Ok(jq) => {
            use jaq_core::load::{Arena, File, Loader};
            use jaq_core::{Ctx, RcIter};
            use jaq_json::Val;

            let input_value = jq.input.unwrap_or(Value::Null);

            let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
            let arena = Arena::default();
            let program = File {
                code: jq.program.as_str(),
                path: (),
            };

            let modules = loader.load(&arena, program);

            match modules {
                Ok(mods) => {
                    let filter = jaq_core::Compiler::default()
                        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
                        .compile(mods);

                    match filter {
                        Ok(f) => {
                            let input = Val::from(input_value);
                            let inputs = RcIter::new(core::iter::empty());
                            let out = f.run((Ctx::new([], &inputs), input));

                            let mut results = Vec::new();
                            let mut execution_err = None;

                            for res in out {
                                match res {
                                    Ok(v) => {
                                        results.push(Value::from(v));
                                    }
                                    Err(e) => {
                                        execution_err =
                                            Some(anyhow::anyhow!("JQ execution error: {:?}", e));
                                        break;
                                    }
                                }
                            }

                            if let Some(e) = execution_err {
                                Err(e)
                            } else {
                                let final_result = if results.len() == 1 {
                                    results.remove(0)
                                } else {
                                    Value::Array(results)
                                };

                                Ok(final_result)
                            }
                        }
                        Err(e) => Err(anyhow::anyhow!("JQ compile error: {:?}", e)),
                    }
                }
                Err(e) => Err(anyhow::anyhow!("JQ load/parse error: {:?}", e)),
            }
        }
        Err(e) => Err(anyhow::anyhow!("Invalid JQ spec: {:?}", e)),
    };

    match result {
        Ok(outputs) => {
            use std::collections::HashMap;
            use stormchaser_model::events::{EventType, StepCompletedEvent, StepEventType};
            let mut outputs_map = HashMap::new();
            outputs_map.insert("result".to_string(), outputs);
            let event = StepCompletedEvent {
                run_id,
                step_id: step_instance_id,
                event_type: EventType::Step(StepEventType::Completed),
                runner_id: None,
                exit_code: Some(0),
                storage_hashes: None,
                artifacts: None,
                test_reports: None,
                outputs: Some(outputs_map),
                timestamp: Utc::now(),
            };
            let js = async_nats::jetstream::new(nats_client);
            use stormchaser_model::nats::NatsSubject;
            let _ = stormchaser_model::nats::publish_cloudevent(
                &js,
                NatsSubject::StepCompleted,
                EventType::Step(StepEventType::Completed),
                EventSource::System,
                serde_json::to_value(event).unwrap(),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
        Err(e) => {
            let event = StepFailedEvent {
                run_id,
                step_id: step_instance_id,
                event_type: EventType::Step(StepEventType::Failed),
                error: format!("JQ execution failed: {:?}", e),
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
                NatsSubject::StepFailed,
                EventType::Step(StepEventType::Failed),
                EventSource::System,
                serde_json::to_value(event).unwrap(),
                Some(SchemaVersion::new("1.0".to_string())),
                None,
            )
            .await;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutate_if_has_files_with_input() {
        let mut step_type = "JQ".to_string();
        let mut spec = serde_json::json!({
            "program": ".foo",
            "input_file": "/tmp/in.json"
        });

        mutate_if_has_files(&mut step_type, &mut spec);

        assert_eq!(step_type, "RunContainer");
        let spec_obj = spec.as_object().unwrap();
        assert_eq!(spec_obj.get("image").unwrap(), "ghcr.io/jqlang/jq:latest");
        let command = spec_obj.get("command").unwrap().as_array().unwrap();
        assert_eq!(command[0], "sh");
        assert_eq!(command[1], "-c");
        assert_eq!(command[2], "jq -c '.foo' /tmp/in.json");
    }

    #[test]
    fn test_mutate_if_has_files_with_output() {
        let mut step_type = "JQ".to_string();
        let mut spec = serde_json::json!({
            "program": ".foo",
            "output_file": "/tmp/out.json"
        });

        mutate_if_has_files(&mut step_type, &mut spec);

        assert_eq!(step_type, "RunContainer");
        let spec_obj = spec.as_object().unwrap();
        let command = spec_obj.get("command").unwrap().as_array().unwrap();
        assert_eq!(command[2], "jq -c '.foo' > /tmp/out.json");
    }

    #[test]
    fn test_mutate_if_has_files_with_both() {
        let mut step_type = "JQ".to_string();
        let mut spec = serde_json::json!({
            "program": ".foo",
            "input_file": "in.json",
            "output_file": "out.json"
        });

        mutate_if_has_files(&mut step_type, &mut spec);

        assert_eq!(step_type, "RunContainer");
        let spec_obj = spec.as_object().unwrap();
        let command = spec_obj.get("command").unwrap().as_array().unwrap();
        assert_eq!(command[2], "jq -c '.foo' in.json > out.json");
    }

    #[test]
    fn test_mutate_if_has_files_no_files() {
        let mut step_type = "JQ".to_string();
        let mut spec = serde_json::json!({
            "program": ".foo",
            "input": { "foo": "bar" }
        });

        mutate_if_has_files(&mut step_type, &mut spec);

        assert_eq!(step_type, "JQ"); // Should not change
        assert_eq!(spec.get("program").unwrap(), ".foo");
    }

    #[tokio::test]
    #[ignore]
    async fn test_dispatch_jq_internal_compiles() {
        let _f = dispatch_jq_internal;
    }
}
