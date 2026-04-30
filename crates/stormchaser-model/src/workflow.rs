//! Core workflow run and state management types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use utoipa::ToSchema;

/// The overall status of a workflow run.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq, ToSchema)]
#[sqlx(type_name = "run_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The run has been queued but not yet processed.
    Queued,
    /// The run's source code and dependencies are being resolved.
    Resolving,
    /// The run is ready but waiting for resources to start.
    StartPending,
    /// The run is actively executing.
    Running,
    /// The run completed successfully.
    Succeeded,
    /// The run encountered a fatal error.
    Failed,
    /// The run was aborted by a user or system event.
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

/// Details of a specific workflow run execution.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WorkflowRun {
    /// Unique identifier for the workflow run.
    pub id: Uuid,
    /// The name of the workflow.
    pub workflow_name: String,
    /// Identifier of the user or system that initiated the run.
    pub initiating_user: String,
    /// The repository URL containing the workflow source.
    pub repo_url: String,
    /// The file path to the workflow definition within the repository.
    pub workflow_path: String,
    /// The Git reference (branch, tag, or commit) being executed.
    pub git_ref: String,
    /// Current status of the run.
    pub status: RunStatus,
    /// Version number used for optimistic concurrency control.
    pub version: i32, // For Optimistic Concurrency Control
    /// Monotonically increasing token for fencing state updates.
    pub fencing_token: i64, // Monotonically increasing token
    /// Timestamp when the run was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the run was last updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the run began resolving dependencies.
    pub started_resolving_at: Option<DateTime<Utc>>,
    /// Timestamp when the run actually started executing steps.
    pub started_at: Option<DateTime<Utc>>,
    /// Timestamp when the run finished execution (success, failure, or aborted).
    pub finished_at: Option<DateTime<Utc>>,
    /// Optional error message if the run failed.
    pub error: Option<String>,
}

/// Execution context for a workflow run.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct RunContext {
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Version of the DSL the workflow was written in.
    pub dsl_version: String,
    /// Full parsed abstract syntax tree of the workflow definition.
    pub workflow_definition: Value, // Full parsed AST
    /// Original unparsed workflow file content.
    pub source_code: String, // Original workflow file content
    /// JSON inputs provided at trigger-time.
    pub inputs: Value, // Trigger-time inputs
    /// Decrypted map of secrets available to this run.
    pub secrets: Value, // Decrypted secrets map
    /// Registry of sensitive values that must be redacted from logs and outputs.
    pub sensitive_values: Vec<String>, // Redaction registry
}

/// Resource quotas associated with a workflow run.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct RunQuotas {
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Maximum number of concurrent steps allowed.
    pub max_concurrency: i32,
    /// Maximum CPU limit for the overall run.
    pub max_cpu: String,
    /// Maximum memory limit for the overall run.
    pub max_memory: String,
    /// Maximum storage limit for the overall run.
    pub max_storage: String,
    /// Maximum duration before the run times out.
    pub timeout: String,
    /// Current accumulated CPU usage of the run.
    pub current_cpu_usage: f64,
    /// Current accumulated memory usage of the run.
    pub current_memory_usage: String,
}

/// Audit log entry for tracking workflow events.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AuditLog {
    /// Unique identifier for the audit log entry.
    pub id: i64,
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// The type of event that occurred (e.g., 'workflow_started', 'step_failed').
    pub event_type: String, // e.g., "workflow_started", "step_failed", "approval_granted"
    /// The identifier of the user or system process that caused the event.
    pub actor: String, // User ID or system process
    /// Additional context or metadata about the event.
    pub payload: Value,
    /// Timestamp when the event was recorded.
    pub created_at: DateTime<Utc>,
}
