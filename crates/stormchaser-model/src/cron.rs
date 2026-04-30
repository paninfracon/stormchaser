//! Cron scheduling models for recurring workflow execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Represents a scheduled cron workflow configuration.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct CronWorkflow {
    /// Unique identifier for the cron workflow.
    pub id: Uuid,
    /// Name of the schedule.
    pub name: String,
    /// Optional description of the schedule's purpose.
    pub description: Option<String>,
    /// Cron expression defining the schedule.
    pub cronspec: String,
    /// Name of the workflow to run.
    pub workflow_name: String,
    /// URL of the Git repository containing the workflow.
    pub repo_url: String,
    /// Path to the workflow file within the repository.
    pub workflow_path: String,
    /// Git branch, tag, or commit to execute.
    pub git_ref: String,
    /// Default inputs to pass to the workflow run.
    pub inputs: Value,
    /// Secret token used to validate external triggers.
    pub secret_token: String,
    /// Whether the schedule is currently active and processing.
    pub is_active: bool,
    /// External job ID associated with this cron schedule.
    pub external_job_id: Option<String>,
    /// Timestamp when the schedule was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the schedule was last updated.
    pub updated_at: DateTime<Utc>,
}
