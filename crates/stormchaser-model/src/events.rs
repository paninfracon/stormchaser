use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowQueuedEvent {
    pub run_id: Uuid,
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
    pub run_id: Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowRunningEvent {
    pub run_id: Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowCompletedEvent {
    pub run_id: Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowFailedEvent {
    pub run_id: Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkflowAbortedEvent {
    pub run_id: Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepScheduledEvent {
    pub run_id: Uuid,
    pub step_id: Uuid,
    pub event_type: String,
    pub step_dsl: Value,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepRunningEvent {
    pub run_id: Uuid,
    pub step_id: Uuid,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepCompletedEvent {
    pub run_id: Uuid,
    pub step_id: Uuid,
    pub event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<HashMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepFailedEvent {
    pub run_id: Uuid,
    pub step_id: Uuid,
    pub event_type: String,
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepQueryEvent {
    pub step_id: Uuid,
    pub run_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepQueryResponseEvent {
    pub step_id: Uuid,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunnerRegisterEvent {
    pub runner_id: String,
    pub labels: HashMap<String, String>,
    pub features: Vec<String>,
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
