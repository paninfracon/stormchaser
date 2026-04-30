//! System event and correlation models for async execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Maps a correlation key and value to a specific step instance waiting for an event.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct EventCorrelation {
    /// Unique identifier for the correlation record.
    pub id: Uuid,
    /// Associated step instance ID waiting for the event.
    pub step_instance_id: Uuid,
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// The key used for correlation (e.g., a specific payload field).
    pub correlation_key: String,
    /// The expected value for the correlation key.
    pub correlation_value: String,
    /// Timestamp when the correlation record was created.
    pub created_at: DateTime<Utc>,
}

/// Registry of human-in-the-loop approvals.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ApprovalRegistry {
    /// Unique identifier for the approval record.
    pub id: Uuid,
    /// Associated step instance ID that requires approval.
    pub step_instance_id: Uuid,
    /// Identifier of the user who provided the approval or rejection.
    pub user_id: String,
    /// Status of the approval request (e.g., 'approved', 'rejected').
    pub status: String, // approved, rejected
    /// Optional metadata or comments provided with the approval decision.
    pub payload: Value,
    /// Timestamp when the approval record was created.
    pub created_at: DateTime<Utc>,
}
