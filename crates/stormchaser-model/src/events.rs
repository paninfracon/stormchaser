use crate::id::{RunId, StepInstanceId};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowQueuedEvent {
    pub run_id: RunId,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dsl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiating_user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowStartPendingEvent {
    pub run_id: RunId,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowRunningEvent {
    pub run_id: RunId,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowCompletedEvent {
    pub run_id: RunId,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowFailedEvent {
    pub run_id: RunId,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowAbortedEvent {
    pub run_id: RunId,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepScheduledEvent {
    pub run_id: RunId,
    pub step_id: StepInstanceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_type: Option<String>,
    pub event_type: String,
    pub step_dsl: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_report_urls: Option<HashMap<String, Value>>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepRunningEvent {
    pub run_id: RunId,
    pub step_id: StepInstanceId,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepCompletedEvent {
    pub run_id: RunId,
    pub step_id: StepInstanceId,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_hashes: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_reports: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepFailedEvent {
    pub run_id: RunId,
    pub step_id: StepInstanceId,
    pub event_type: String,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_hashes: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_reports: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<HashMap<String, Value>>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepQueryEvent {
    pub step_id: StepInstanceId,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepQueryResponseEvent {
    pub step_id: StepInstanceId,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunnerStepTypeSchema {
    pub step_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunnerRegisterEvent {
    pub runner_id: String,
    pub runner_type: String,
    pub protocol_version: String,
    pub nats_subject: String,
    pub capabilities: Vec<String>,
    pub step_types: Vec<RunnerStepTypeSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunnerHeartbeatEvent {
    pub runner_id: String,
    pub version: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunnerOfflineEvent {
    pub runner_id: String,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_runner_register_event_deserializes_from_wire_format() {
        let wire_json = serde_json::json!({
            "runner_id": "runner-abc-123",
            "runner_type": "docker",
            "protocol_version": "v1",
            "nats_subject": "stormchaser.v1.runner.docker.runner-abc-123",
            "capabilities": ["docker", "linux", "container"],
            "step_types": [
                {
                    "step_type": "RunContainer",
                    "documentation": "Runs a container using Docker."
                }
            ]
        });

        let event: RunnerRegisterEvent = serde_json::from_value(wire_json).unwrap();
        assert_eq!(event.runner_id, "runner-abc-123");
        assert_eq!(event.runner_type, "docker");
        assert_eq!(event.protocol_version, "v1");
        assert_eq!(
            event.nats_subject,
            "stormchaser.v1.runner.docker.runner-abc-123"
        );
        assert_eq!(event.capabilities, vec!["docker", "linux", "container"]);
        assert_eq!(event.step_types.len(), 1);
        assert_eq!(event.step_types[0].step_type, "RunContainer");
        assert!(event.step_types[0].schema.is_none());
    }

    #[test]
    fn test_runner_register_event_serializes_correctly() {
        let event = RunnerRegisterEvent {
            runner_id: "runner-xyz".to_string(),
            runner_type: "k8s".to_string(),
            protocol_version: "v1".to_string(),
            nats_subject: "stormchaser.v1.runner.k8s.runner-xyz".to_string(),
            capabilities: vec!["k8s".to_string()],
            step_types: vec![RunnerStepTypeSchema {
                step_type: "RunK8sJob".to_string(),
                schema: None,
                documentation: Some("Runs a Kubernetes Job.".to_string()),
            }],
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["runner_id"], "runner-xyz");
        assert_eq!(json["runner_type"], "k8s");
        assert_eq!(json["protocol_version"], "v1");
        assert_eq!(json["step_types"][0]["step_type"], "RunK8sJob");
        // schema is None, so it should not appear in the output
        assert!(json["step_types"][0].get("schema").is_none());
    }

    #[test]
    fn test_step_query_event_has_only_step_id() {
        let event = StepQueryEvent {
            step_id: StepInstanceId::new(Uuid::nil()),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert!(json.get("step_id").is_some());
        assert!(json.get("run_id").is_none());
    }
}
