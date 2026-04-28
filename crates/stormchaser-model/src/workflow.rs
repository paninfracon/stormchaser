use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use utoipa::ToSchema;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq, ToSchema)]
#[sqlx(type_name = "run_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Resolving,
    StartPending,
    Running,
    Succeeded,
    Failed,
    Aborted,
}

impl From<String> for RunStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "resolving" => RunStatus::Resolving,
            "start_pending" => RunStatus::StartPending,
            "running" => RunStatus::Running,
            "succeeded" => RunStatus::Succeeded,
            "failed" => RunStatus::Failed,
            "aborted" => RunStatus::Aborted,
            _ => RunStatus::Queued,
        }
    }
}

impl From<RunStatus> for String {
    fn from(s: RunStatus) -> Self {
        match s {
            RunStatus::Queued => "queued".to_string(),
            RunStatus::Resolving => "resolving".to_string(),
            RunStatus::StartPending => "start_pending".to_string(),
            RunStatus::Running => "running".to_string(),
            RunStatus::Succeeded => "succeeded".to_string(),
            RunStatus::Failed => "failed".to_string(),
            RunStatus::Aborted => "aborted".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WorkflowRun {
    pub id: Uuid,
    pub workflow_name: String,
    pub initiating_user: String,
    pub repo_url: String,
    pub workflow_path: String,
    pub git_ref: String,
    pub status: RunStatus,
    pub version: i32,       // For Optimistic Concurrency Control
    pub fencing_token: i64, // Monotonically increasing token
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_resolving_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct RunContext {
    pub run_id: Uuid,
    pub dsl_version: String,
    pub workflow_definition: Value,    // Full parsed AST
    pub source_code: String,           // Original workflow file content
    pub inputs: Value,                 // Trigger-time inputs
    pub secrets: Value,                // Decrypted secrets map
    pub sensitive_values: Vec<String>, // Redaction registry
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct RunQuotas {
    pub run_id: Uuid,
    pub max_concurrency: i32,
    pub max_cpu: String,
    pub max_memory: String,
    pub max_storage: String,
    pub timeout: String,
    pub current_cpu_usage: f64,
    pub current_memory_usage: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AuditLog {
    pub id: i64,
    pub run_id: Uuid,
    pub event_type: String, // e.g., "workflow_started", "step_failed", "approval_granted"
    pub actor: String,      // User ID or system process
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}
