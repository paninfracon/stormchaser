use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct OutboxMessage {
    pub id: i64,
    pub run_id: Uuid,
    pub subject: String,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
}
