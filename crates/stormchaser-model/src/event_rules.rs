use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WebhookConfig {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub source_type: String, // 'github', 'generic', etc.
    pub secret_token: Option<String>,
    pub is_active: bool,
    pub ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct EventRule {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub webhook_id: Option<Uuid>,
    pub event_type_pattern: String,
    pub condition_expr: Option<String>,
    pub workflow_name: String,
    pub repo_url: String,
    pub workflow_path: String,
    pub git_ref: String,
    pub input_mappings: Value, // Map of name -> CEL expr
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EventRule {
    pub fn get_input_mappings(&self) -> HashMap<String, String> {
        serde_json::from_value(self.input_mappings.clone()).unwrap_or_default()
    }
}
