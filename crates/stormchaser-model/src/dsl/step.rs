use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

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
    /// How many times a failed step is run again, after its first attempt.
    pub count: u32,
    /// Backoff strategy: 'fixed', 'linear' or 'exponential'.
    pub backoff: String,
    /// Delay before the first retry (default 10s). Later retries wait longer
    /// according to `backoff`.
    #[serde(default)]
    pub delay: Option<String>,
    /// Maximum backoff delay.
    pub max_delay: Option<String>,
}

impl RetryPolicy {
    /// The most retries a step may ask for.
    pub const MAX_COUNT: u32 = 100;
    /// The longest a step waits between attempts, whatever its policy says.
    pub const MAX_DELAY: Duration = Duration::from_secs(24 * 60 * 60);
    const DEFAULT_DELAY: Duration = Duration::from_secs(10);

    /// Checks the policy can be followed: a known backoff, durations that parse and a
    /// count no larger than [`Self::MAX_COUNT`].
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.backoff.as_str(), "fixed" | "linear" | "exponential") {
            return Err(format!(
                "retry backoff '{}' is not one of 'fixed', 'linear' or 'exponential'",
                self.backoff
            ));
        }
        if self.count > Self::MAX_COUNT {
            return Err(format!(
                "retry count {} is more than the most allowed, {}",
                self.count,
                Self::MAX_COUNT
            ));
        }
        for (name, value) in [("delay", &self.delay), ("max_delay", &self.max_delay)] {
            if let Some(value) = value {
                humantime::parse_duration(value.trim())
                    .map_err(|e| format!("retry {} '{}' is not a duration: {}", name, value, e))?;
            }
        }
        Ok(())
    }

    /// How long to wait before retry number `retry` (1 for the first retry).
    /// A duration that does not parse counts as the default.
    pub fn delay_before(&self, retry: u32) -> Duration {
        let parse = |value: &Option<String>| {
            value
                .as_deref()
                .and_then(|v| humantime::parse_duration(v.trim()).ok())
        };
        let base = parse(&self.delay).unwrap_or(Self::DEFAULT_DELAY);
        let retry = retry.max(1);
        let delay = match self.backoff.as_str() {
            "linear" => base.saturating_mul(retry),
            "exponential" => base.saturating_mul(2u32.saturating_pow(retry - 1)),
            _ => base,
        };
        let max = parse(&self.max_delay)
            .unwrap_or(Self::MAX_DELAY)
            .min(Self::MAX_DELAY);
        delay.min(max)
    }
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

#[cfg(test)]
mod retry_tests {
    use super::*;

    fn policy(backoff: &str, delay: Option<&str>, max_delay: Option<&str>) -> RetryPolicy {
        RetryPolicy {
            count: 3,
            backoff: backoff.to_string(),
            delay: delay.map(str::to_string),
            max_delay: max_delay.map(str::to_string),
        }
    }

    #[test]
    fn delays_follow_the_backoff() {
        let secs = |p: &RetryPolicy, n| p.delay_before(n).as_secs();
        let fixed = policy("fixed", Some("5s"), None);
        assert_eq!((secs(&fixed, 1), secs(&fixed, 3)), (5, 5));
        let linear = policy("linear", Some("5s"), None);
        assert_eq!((secs(&linear, 1), secs(&linear, 3)), (5, 15));
        let exponential = policy("exponential", None, None);
        assert_eq!((secs(&exponential, 1), secs(&exponential, 4)), (10, 80));
    }

    #[test]
    fn delays_stop_at_max_delay_and_never_overflow() {
        let capped = policy("exponential", Some("10s"), Some("30s"));
        assert_eq!(capped.delay_before(10).as_secs(), 30);
        let uncapped = policy("exponential", Some("1h"), None);
        assert_eq!(uncapped.delay_before(100), RetryPolicy::MAX_DELAY);
        let too_high = policy("fixed", Some("1h"), Some("1000d"));
        assert_eq!(too_high.delay_before(1).as_secs(), 3600);
    }

    #[test]
    fn validation_refuses_what_cannot_be_followed() {
        assert!(policy("linear", Some("1m"), Some("5m")).validate().is_ok());
        assert!(policy("random", None, None).validate().is_err());
        assert!(policy("fixed", Some("soon"), None).validate().is_err());
        assert!(policy("fixed", None, Some("later")).validate().is_err());
        let mut many = policy("fixed", None, None);
        many.count = RetryPolicy::MAX_COUNT + 1;
        assert!(many.validate().is_err());
    }
}
