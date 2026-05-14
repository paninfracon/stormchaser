//! Storage backend and artifact registry models.

use crate::id::{ArtifactId, ConnectionId, RunId, StepInstanceId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

/// Supported connection types for storage, databases, APIs, and VCS.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq, ToSchema)]
#[sqlx(type_name = "connection_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType {
    /// Amazon S3 or compatible storage.
    S3,
    /// OCI-compliant registry.
    Oci,
    /// JFrog Artifactory.
    Jfrog,
    /// Google Cloud Storage.
    Gcs,
    /// Azure Blob Storage.
    Azure,
    /// PostgreSQL database.
    Postgres,
    /// MySQL database.
    Mysql,
    /// Generic HTTP API.
    HttpApi,
    /// Git Repository.
    Git,
}

/// Represents a configured connection instance.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct Connection {
    /// Unique identifier for the connection.
    pub id: ConnectionId,
    /// Logical name of the connection.
    pub name: String,
    /// Optional description of the connection's purpose.
    pub description: Option<String>,
    /// The type of connection.
    pub connection_type: ConnectionType,
    /// JSON configuration specific to the connection type.
    pub config: Value,
    /// Encrypted credentials (e.g. passwords, tokens).
    pub encrypted_credentials: Option<String>,
    /// Optional AWS role ARN to assume via STS (primarily for S3).
    pub aws_assume_role_arn: Option<String>,
    /// Whether this connection is the default Stormchaser File System (SFS).
    pub is_default_sfs: bool,
    /// Optional custom CA certificate for mTLS.
    pub ca_cert: Option<String>,
    /// Optional client certificate for mTLS.
    pub client_cert: Option<String>,
    /// Optional client private key for mTLS.
    pub client_key: Option<String>,
    /// Timestamp when the connection was registered.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Timestamp when the connection configuration was last updated.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Registry of workflow artifacts stored in connections.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow, ToSchema)]
pub struct ArtifactRegistry {
    /// Unique identifier for the artifact record.
    pub id: ArtifactId,
    /// Associated workflow run ID.
    pub run_id: RunId,
    /// Associated step instance ID that produced the artifact.
    pub step_instance_id: StepInstanceId,
    /// Logical name of the artifact.
    pub artifact_name: String,
    /// Identifier of the connection where the artifact is located.
    pub connection_id: ConnectionId,
    /// Path or locator within the connection.
    pub remote_path: String,
    /// Additional metadata about the artifact.
    pub metadata: Value,
    /// Timestamp when the artifact was registered.
    pub created_at: chrono::DateTime<chrono::Utc>,
}
