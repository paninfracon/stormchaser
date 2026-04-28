use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "runner_status", rename_all = "snake_case")]
pub enum RunnerStatus {
    Online,
    Offline,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Runner {
    pub id: String,
    pub runner_type: String, // e.g., "k8s", "binary", "ecs"
    pub status: RunnerStatus,
    pub protocol_version: String,
    pub capabilities: Vec<String>,
    pub nats_subject: String, // Subject used to communicate with this specific runner
    pub last_heartbeat_at: DateTime<Utc>,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct StepDefinition {
    pub step_type: String,
    pub schema: serde_json::Value,
    pub documentation: Option<String>,
    pub registered_at: DateTime<Utc>,
}
