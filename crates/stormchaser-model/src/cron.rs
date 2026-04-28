use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct CronWorkflow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub cronspec: String,
    pub workflow_name: String,
    pub repo_url: String,
    pub workflow_path: String,
    pub git_ref: String,
    pub inputs: serde_json::Value,
    pub secret_token: String,
    pub is_active: bool,
    pub external_job_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
