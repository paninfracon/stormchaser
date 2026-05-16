use anyhow::Result;
use stormchaser_dsl::ast;
use stormchaser_model::dsl::{OutputExtraction, RestApiSpec};
use tracing::warn;

pub async fn persist_step_outputs(
    tx: &mut sqlx::PgConnection,
    step_id: stormchaser_model::StepInstanceId,
    outputs: Option<&serde_json::Map<String, serde_json::Value>>,
    dsl_step: Option<&ast::Step>,
) -> Result<()> {
    let Some(outputs) = outputs else {
        return Ok(());
    };
    let rest_api_sensitivity = rest_api_output_sensitivity(dsl_step);

    for (key, value) in outputs {
        crate::db::upsert_step_output_with_sensitivity(
            &mut *tx,
            step_id,
            key,
            value,
            rest_api_sensitivity.get(key).copied().unwrap_or(false),
        )
        .await?;
    }

    Ok(())
}

fn rest_api_output_sensitivity(
    dsl_step: Option<&ast::Step>,
) -> std::collections::HashMap<String, bool> {
    let Some(step) = dsl_step else {
        return std::collections::HashMap::new();
    };

    if step.r#type != "RestApi" {
        return std::collections::HashMap::new();
    }

    match serde_json::from_value::<RestApiSpec>(step.spec.clone()) {
        Ok(spec) => spec
            .extractors
            .unwrap_or_default()
            .into_iter()
            .map(|extractor| (extractor.name, extractor.sensitive.unwrap_or(false)))
            .collect(),
        Err(error) => {
            warn!(
                step_name = %step.name,
                ?error,
                "Failed to parse RestApi spec while determining output sensitivity"
            );
            std::collections::HashMap::new()
        }
    }
}

pub fn setup_terraform_outputs(step: &mut ast::Step) {
    if step.r#type == "TerraformApply" {
        step.outputs.push(OutputExtraction {
            name: "terraform".to_string(),
            source: "stdout".to_string(),
            marker: Some("--- TF OUTPUTS ---".to_string()),
            format: Some("json".to_string()),
            regex: Some(r"--- TF OUTPUTS ---\s*(.*)".to_string()),
            group: Some(1),
            sensitive: Some(false),
        });
    } else if step.r#type == "TerraformPlan" {
        step.outputs.push(OutputExtraction {
            name: "plan_summary".to_string(),
            source: "stdout".to_string(),
            marker: Some("--- TF PLAN SUMMARY ---".to_string()),
            format: Some("string".to_string()),
            regex: Some(r"--- TF PLAN SUMMARY ---\s*(.*)".to_string()),
            group: Some(1),
            sensitive: Some(false),
        });
        step.outputs.push(OutputExtraction {
            name: "plan_json".to_string(),
            source: "stdout".to_string(),
            marker: Some("--- TF PLAN JSON ---".to_string()),
            format: Some("json".to_string()),
            regex: Some(r"--- TF PLAN JSON ---\s*(.*)".to_string()),
            group: Some(1),
            sensitive: Some(false),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use stormchaser_model::dsl::RestApiResponseExtractor;

    #[test]
    fn test_rest_api_output_sensitivity_reads_extractor_flags() {
        let step = ast::Step {
            name: "fetch_data".to_string(),
            r#type: "RestApi".to_string(),
            condition: None,
            params: HashMap::new(),
            spec: json!({
                "url": "https://api.example.com",
                "extractors": [
                    RestApiResponseExtractor {
                        name: "token".to_string(),
                        format: Some("json".to_string()),
                        json_pointer: Some("/data/token".to_string()),
                        regex: None,
                        group: None,
                        sensitive: Some(true),
                    },
                    RestApiResponseExtractor {
                        name: "id".to_string(),
                        format: Some("json".to_string()),
                        json_pointer: Some("/data/id".to_string()),
                        regex: None,
                        group: None,
                        sensitive: Some(false),
                    }
                ]
            }),
            strategy: None,
            aggregation: Vec::new(),
            iterate: None,
            iterate_as: None,
            steps: None,
            next: Vec::new(),
            on_failure: None,
            aliases: std::collections::HashMap::new(),
            retry: None,
            timeout: None,
            allow_failure: None,
            start_marker: None,
            end_marker: None,
            outputs: Vec::new(),
            reports: Vec::new(),
            artifacts: None,
        };

        let sensitivity = rest_api_output_sensitivity(Some(&step));

        assert_eq!(sensitivity.get("token"), Some(&true));
        assert_eq!(sensitivity.get("id"), Some(&false));
    }
}
