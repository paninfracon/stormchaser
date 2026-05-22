use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type RunId = Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
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

/// A summary of a workflow run, used for list views.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WorkflowRunDetail {
    /// Unique identifier for the run.
    pub id: RunId,
    /// Name of the workflow.
    pub workflow_name: String,
    /// The user who initiated the run.
    pub initiating_user: String,
    /// The current status of the run.
    pub status: RunStatus,
    /// The time the run was created.
    pub created_at: DateTime<Utc>,
    /// The time the run finished, if it has completed.
    pub finished_at: Option<DateTime<Utc>>,
}
