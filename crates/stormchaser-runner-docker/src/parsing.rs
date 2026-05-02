use crate::container_machine;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use stormchaser_model::dsl;

/// Parses a `dsl::Step` from a JSON payload received via NATS.
/// Falls back to constructing a step from basic fields if the full `step_dsl` is missing.
pub fn parse_step_from_nats_payload(payload: &Value) -> Result<dsl::Step> {
    if let Some(dsl_val) = payload.get("step_dsl") {
        if !dsl_val.is_null() {
            if let Ok(step) = serde_json::from_value(dsl_val.clone()) {
                return Ok(step);
            }
        }
    }

    let spec =
        serde_json::from_value(payload["spec"].clone()).context("Failed to parse step spec")?;

    Ok(dsl::Step {
        name: payload["step_name"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        r#type: payload["step_type"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        spec,
        params: serde_json::from_value(payload["params"].clone()).unwrap_or_default(),
        condition: None,
        strategy: None,
        aggregation: Vec::new(),
        iterate: None,
        iterate_as: None,
        steps: None,
        next: Vec::new(),
        on_failure: None,
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        outputs: Vec::new(),
        reports: Vec::new(),
        artifacts: None,
    })
}

/// Parses a `dsl::Step` from Docker container labels.
/// If the step DSL is encrypted, it attempts to decrypt it using the provided key.
/// Falls back to constructing a basic step if the DSL is missing or parsing fails.
pub fn parse_step_from_docker_labels(
    container_name: &str,
    raw_step_dsl: Option<&String>,
    is_encrypted: bool,
    encryption_key: Option<&String>,
) -> Result<dsl::Step> {
    if let Some(raw) = raw_step_dsl {
        let dsl_str = if is_encrypted {
            if let Some(key) = encryption_key {
                container_machine::crypto::decrypt_state(raw, key).context(format!(
                    "Failed to decrypt state for container {}",
                    container_name
                ))?
            } else {
                anyhow::bail!(
                    "Container {} is encrypted but no encryption key is configured",
                    container_name
                );
            }
        } else {
            raw.clone()
        };

        if let Ok(step) = serde_json::from_str(&dsl_str) {
            return Ok(step);
        }
    }

    // Fallback
    Ok(dsl::Step {
        name: container_name.to_string(),
        r#type: "RunContainer".to_string(),
        spec: Value::Null,
        params: HashMap::new(),
        condition: None,
        strategy: None,
        aggregation: Vec::new(),
        iterate: None,
        iterate_as: None,
        steps: None,
        next: Vec::new(),
        on_failure: None,
        retry: None,
        timeout: None,
        allow_failure: None,
        start_marker: None,
        end_marker: None,
        outputs: Vec::new(),
        reports: Vec::new(),
        artifacts: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_step_from_nats_payload() {
        let payload = json!({
            "step_name": "test_step",
            "step_type": "RunContainer",
            "spec": {
                "image": "alpine",
                "command": ["echo"],
                "args": ["hello"]
            },
            "params": {}
        });

        let result = parse_step_from_nats_payload(&payload).unwrap();
        assert_eq!(result.name, "test_step");
        assert_eq!(result.r#type, "RunContainer");
    }

    #[test]
    fn test_parse_step_from_docker_labels_fallback() {
        let result = parse_step_from_docker_labels("my_container", None, false, None).unwrap();
        assert_eq!(result.name, "my_container");
        assert_eq!(result.r#type, "RunContainer");
        assert!(result.spec.is_null());
    }
}
