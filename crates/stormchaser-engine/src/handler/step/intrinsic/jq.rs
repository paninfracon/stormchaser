use crate::handler::integrations::utils::{report_step_completed, report_step_failed};
use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

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
                registry_connection: None,
                connections: None,
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

/// Runs a JQ step, reporting its result.
pub(crate) async fn run_jq(
    run_id: RunId,
    step_instance_id: StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
) -> Result<()> {
    // 1. Transition to running
    crate::handler::integrations::utils::start_built_in(&pool, step_instance_id, "intrinsic-jq")
        .await?;

    let actual_spec = spec.get("spec").unwrap_or(&spec).clone();
    let jq_spec: Result<dsl::JqSpec, _> = serde_json::from_value(actual_spec.clone());

    let result = match jq_spec {
        // In a child process that is killed if the program runs too long or grows too
        // large (see `jq_sandbox`)
        Ok(jq) => crate::jq_sandbox::evaluate(&jq.program, jq.input.unwrap_or(Value::Null))
            .await
            .map(|mut results| {
                if results.len() == 1 {
                    results.remove(0)
                } else {
                    Value::Array(results)
                }
            }),
        Err(e) => Err(anyhow::anyhow!("Invalid JQ spec: {:?}", e)),
    };

    match result {
        Ok(outputs) => {
            let outputs = HashMap::from([("result".to_string(), outputs)]);
            report_step_completed(
                &pool,
                run_id,
                step_instance_id,
                fencing_token,
                Some(outputs),
            )
            .await
        }
        Err(e) => {
            report_step_failed(
                &pool,
                run_id,
                step_instance_id,
                fencing_token,
                format!("JQ execution failed: {:?}", e),
            )
            .await
        }
    }
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
}
