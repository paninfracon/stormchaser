use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "backend_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BackendType {
    S3,
    Oci,
    Jfrog,
    Gcs,
    Azure,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct StorageBackend {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub backend_type: BackendType,
    pub config: Value,
    pub is_default_sfs: bool,
    pub ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ArtifactRegistry {
    pub id: Uuid,
    pub run_id: Uuid,
    pub step_instance_id: Uuid,
    pub artifact_name: String,
    pub backend_id: Uuid,
    pub remote_path: String,
    pub metadata: Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
