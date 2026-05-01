//! Storage backend and artifact registry models.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Supported storage backend types for artifacts and engine data.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "backend_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BackendType {
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
}

/// Represents a configured storage backend instance.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct StorageBackend {
    /// Unique identifier for the storage backend.
    pub id: Uuid,
    /// Logical name of the backend.
    pub name: String,
    /// Optional description of the backend's purpose.
    pub description: Option<String>,
    /// The type of storage backend.
    pub backend_type: BackendType,
    /// JSON configuration specific to the backend type.
    pub config: Value,
    /// Optional AWS role ARN to assume via STS (primarily for S3).
    pub aws_assume_role_arn: Option<String>,
    /// Whether this backend is the default Stormchaser File System (SFS).
    pub is_default_sfs: bool,
    /// Optional custom CA certificate for mTLS.
    pub ca_cert: Option<String>,
    /// Optional client certificate for mTLS.
    pub client_cert: Option<String>,
    /// Optional client private key for mTLS.
    pub client_key: Option<String>,
    /// Timestamp when the backend was registered.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Timestamp when the backend configuration was last updated.
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Registry of workflow artifacts stored in backends.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ArtifactRegistry {
    /// Unique identifier for the artifact record.
    pub id: Uuid,
    /// Associated workflow run ID.
    pub run_id: Uuid,
    /// Associated step instance ID that produced the artifact.
    pub step_instance_id: Uuid,
    /// Logical name of the artifact.
    pub artifact_name: String,
    /// Identifier of the storage backend where the artifact is located.
    pub backend_id: Uuid,
    /// Path or locator within the storage backend.
    pub remote_path: String,
    /// Additional metadata about the artifact.
    pub metadata: Value,
    /// Timestamp when the artifact was registered.
    pub created_at: chrono::DateTime<chrono::Utc>,
}
