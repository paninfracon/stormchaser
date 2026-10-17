use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::Strategy;

/// Event handler mapping an event to an action.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Handler {
    /// Name of the handler.
    pub name: String,
    /// Type of event to match.
    pub event_type: String,
    /// Condition expression to filter events.
    pub condition: Option<String>,
    /// Action to execute when the event occurs.
    pub action: String,
}

/// Execution step definition within a workflow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Step {
    /// Name of the step.
    pub name: String,
    /// Type of the step.
    pub r#type: String, // e.g., "GitCheckout", "RunContainer", "Parallel", etc.
    /// HCL condition controlling whether the step runs.
    pub condition: Option<String>,
    /// Evaluated parameters for the step.
    pub params: HashMap<String, String>, // HCL Expressions

    /// Step-specific specification.
    /// This allows each step type to define its own structured parameters.
    /// For "RunContainer" on K8s, this would be a K8sJobSpec.
    #[serde(default)]
    pub spec: Value,

    /// Execution strategy overrides for this step.
    pub strategy: Option<Strategy>,
    /// Values to aggregate from iterated steps.
    pub aggregation: Vec<Aggregation>,
    /// Iteration expression (e.g., a list of items to map over).
    pub iterate: Option<String>,
    /// Variable name for the current iteration item.
    pub iterate_as: Option<String>,
    /// Child steps if this is a group (e.g., Parallel).
    pub steps: Option<Vec<Step>>, // For Parallel/Sequential
    /// Next steps to execute after this one.
    pub next: Vec<String>,
    /// Steps to execute if this step fails.
    pub on_failure: Option<Box<Step>>,
    /// Aliases defined explicitly for this step.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// Retry policy for this step.
    pub retry: Option<RetryPolicy>,
    /// Timeout duration.
    pub timeout: Option<String>,
    /// Whether failure of this step should be ignored.
    pub allow_failure: Option<bool>,
    /// Log marker indicating the start of a section.
    pub start_marker: Option<String>,
    /// Log marker indicating the end of a section.
    pub end_marker: Option<String>,
    /// Extraction rules to capture output values.
    pub outputs: Vec<OutputExtraction>,
    /// Test reports to collect.
    #[serde(default)]
    pub reports: Vec<TestReport>,
    /// Specific artifacts to collect for this step.
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
}

/// Definition for capturing a test report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TestReport {
    /// Name of the report.
    pub name: String,
    /// Path to the report file.
    pub path: String,
    /// Format of the report (e.g., 'junit').
    pub format: String, // e.g., "junit"
}

/// Aggregation rule for map/reduce patterns.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Aggregation {
    /// Name of the aggregation.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// HCL Expression for aggregation.
    pub value: String, // HCL Expression
}

/// Rule for extracting outputs from logs or files.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutputExtraction {
    /// Output key name.
    pub name: String,
    /// Source to extract from (e.g., 'stdout', 'file').
    pub source: String,
    /// Optional marker string.
    pub marker: Option<String>,
    /// Data format (e.g., 'json', 'regex').
    pub format: Option<String>,
    /// Regular expression to match.
    pub regex: Option<String>,
    /// Regex capture group index.
    pub group: Option<u32>,
    /// Whether the extracted output is sensitive.
    pub sensitive: Option<bool>,
}

/// Step retry policy definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryPolicy {
    /// Number of retry attempts.
    pub count: u32,
    /// Backoff strategy ('fixed', 'exponential').
    pub backoff: String,
    /// Maximum backoff delay.
    pub max_delay: Option<String>,
}

impl Step {
    /// A copy of this step with the values a runner must not leave readable removed:
    /// environment values and parameter values, which is where resolved secrets and
    /// injected connection credentials are.
    ///
    /// Runners keep a step's definition on the Job or container they start for it, so a
    /// restarted runner can pick the step up again. That copy can be read by anyone who
    /// may look at Jobs or containers. Picking a step up needs its name, type, storage,
    /// reports and artifacts, and none of the values removed here.
    pub fn without_secret_values(&self) -> Step {
        let mut step = self.clone();
        for value in step.params.values_mut() {
            value.clear();
        }
        if let Some(env) = step.spec.get_mut("env").and_then(|env| env.as_array_mut()) {
            for entry in env {
                if let Some(value) = entry.get_mut("value") {
                    *value = serde_json::Value::String(String::new());
                }
            }
        }
        step
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::*;
    use crate::dsl::CommonContainerSpec;
    use serde_json::json;

    #[test]
    fn a_persisted_step_keeps_its_shape_and_loses_its_secret_values() {
        let step: Step = serde_json::from_value(json!({
            "name": "deploy",
            "type": "RunContainer",
            "params": { "db_password": "hunter2" },
            "spec": {
                "image": "alpine",
                "env": [
                    { "name": "TOKEN", "value": "tok-123" },
                    { "name": "STORMCHASER_CONN_DB_PASSWORD", "value": "pw-456" }
                ],
                "storage_mounts": [{ "name": "ws", "mount_path": "/ws" }]
            },
            "aggregation": [],
            "next": [],
            "outputs": [],
            "reports": []
        }))
        .unwrap();

        let kept = step.without_secret_values();
        let text = serde_json::to_string(&kept).unwrap();
        for secret in ["hunter2", "tok-123", "pw-456"] {
            assert!(!text.contains(secret), "{secret} survived: {text}");
        }
        // Names, and everything a runner needs to pick the step up again, are intact
        assert_eq!(kept.name, "deploy");
        assert!(kept.params.contains_key("db_password"));
        assert_eq!(kept.spec["env"][0]["name"], "TOKEN");
        assert_eq!(kept.spec["storage_mounts"][0]["mount_path"], "/ws");
        // And it still parses as the spec it was
        let spec: CommonContainerSpec = serde_json::from_value(kept.spec).unwrap();
        assert_eq!(spec.env.unwrap().len(), 2);
    }
}
