//! Models for workflow runner instances and step definitions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The connection status of a remote runner.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq, schemars::JsonSchema)]
#[sqlx(type_name = "runner_status", rename_all = "snake_case")]
pub enum RunnerStatus {
    /// The runner is currently online and communicating.
    Online,
    /// The runner has missed heartbeats and is considered offline.
    Offline,
}

/// Represents a registered workflow runner instance.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Runner {
    /// Unique identifier for the runner.
    pub id: String,
    /// The type of runner environment.
    pub runner_type: String, // e.g., "k8s", "binary", "ecs"
    /// The current connection status of the runner.
    pub status: RunnerStatus,
    /// The protocol version the runner uses to communicate.
    pub protocol_version: String,
    /// A list of capabilities supported by the runner.
    pub capabilities: Vec<String>,
    /// Subject used to communicate with this specific runner.
    pub nats_subject: String, // Subject used to communicate with this specific runner
    /// Timestamp of the last received heartbeat.
    pub last_heartbeat_at: DateTime<Utc>,
    /// Timestamp when the runner first registered.
    pub registered_at: DateTime<Utc>,
}

/// Defines a reusable workflow step implementation available on runners.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct StepDefinition {
    /// Identifier for the step type (e.g., 'docker', 'wasm').
    pub step_type: String,
    /// JSON schema describing the required and optional parameters for this step.
    pub schema: Value,
    /// Optional documentation or description for the step type.
    pub documentation: Option<String>,
    /// Timestamp when this step definition was registered.
    pub registered_at: DateTime<Utc>,
}
