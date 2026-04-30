//! Rule-based event processing and webhook configuration models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Configuration for an external webhook integration.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WebhookConfig {
    /// Unique identifier for the webhook configuration.
    pub id: Uuid,
    /// Descriptive name for the webhook.
    pub name: String,
    /// Optional detailed description.
    pub description: Option<String>,
    /// System generating the webhook (e.g., 'github', 'generic').
    pub source_type: String, // 'github', 'generic', etc.
    /// Secret token used to validate incoming requests.
    pub secret_token: Option<String>,
    /// Whether the webhook is actively receiving events.
    pub is_active: bool,
    /// Optional custom CA certificate for mTLS.
    pub ca_cert: Option<String>,
    /// Optional client certificate for mTLS.
    pub client_cert: Option<String>,
    /// Optional client private key for mTLS.
    pub client_key: Option<String>,
    /// Timestamp when the webhook was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the webhook was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Defines a rule for triggering workflows based on incoming events.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct EventRule {
    /// Unique identifier for the event rule.
    pub id: Uuid,
    /// Name of the event rule.
    pub name: String,
    /// Optional description of the rule's behavior.
    pub description: Option<String>,
    /// Optional ID of the webhook this rule applies to.
    pub webhook_id: Option<Uuid>,
    /// Pattern matching the type of event to handle.
    pub event_type_pattern: String,
    /// CEL expression used to filter matching events.
    pub condition_expr: Option<String>,
    /// Name of the workflow to trigger when the rule matches.
    pub workflow_name: String,
    /// Git repository URL containing the workflow.
    pub repo_url: String,
    /// Path to the workflow file within the repository.
    pub workflow_path: String,
    /// Git reference (branch, tag, or commit) to execute.
    pub git_ref: String,
    /// JSON mapping of workflow inputs to CEL expressions evaluated against the event.
    pub input_mappings: Value, // Map of name -> CEL expr
    /// Whether this rule is actively being evaluated.
    pub is_active: bool,
    /// Timestamp when the rule was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the rule was last updated.
    pub updated_at: DateTime<Utc>,
}

impl EventRule {
    /// Returns the parsed input mappings as a hash map of string keys to CEL expressions.
    pub fn get_input_mappings(&self) -> HashMap<String, String> {
        serde_json::from_value(self.input_mappings.clone()).unwrap_or_default()
    }
}
