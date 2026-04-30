//! Outbox pattern models for reliable event publishing.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Represents a message scheduled for reliable delivery via the outbox pattern.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct OutboxMessage {
    /// Unique identifier for the outbox message.
    pub id: i64,
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Event subject or topic.
    pub subject: String,
    /// JSON-encoded message payload.
    pub payload: Value,
    /// Timestamp when the message was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the message was successfully processed, if applicable.
    pub processed_at: Option<DateTime<Utc>>,
}
