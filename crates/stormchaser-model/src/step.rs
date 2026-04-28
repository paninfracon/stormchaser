use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq, ToSchema)]
#[sqlx(type_name = "step_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    UnpackingSfs,
    Running,
    PackingSfs,
    Succeeded,
    Failed,
    FailedIgnored,
    Skipped,
    WaitingForEvent,
    Aborted,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct StepInstance {
    pub id: Uuid,
    pub run_id: Uuid,
    pub step_name: String,
    pub step_type: String,
    pub status: StepStatus,
    pub iteration_index: Option<i32>,
    pub runner_id: Option<String>,
    pub affinity_context: Option<String>, // For shared/locked affinity
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub spec: serde_json::Value,
    pub params: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct StepOutput {
    pub step_instance_id: Uuid,
    pub key: String,
    pub value: serde_json::Value,
    pub is_sensitive: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct StepStatusHistory {
    pub id: i64,
    pub step_instance_id: Uuid,
    pub status: StepStatus,
    pub created_at: DateTime<Utc>,
}
