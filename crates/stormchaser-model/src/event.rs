use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct EventCorrelation {
    pub id: Uuid,
    pub step_instance_id: Uuid,
    pub run_id: Uuid,
    pub correlation_key: String,
    pub correlation_value: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ApprovalRegistry {
    pub id: Uuid,
    pub step_instance_id: Uuid,
    pub user_id: String,
    pub status: String, // approved, rejected
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}
