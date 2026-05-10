use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Storage volume definition to be provisioned.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Storage {
    /// Logical name of the storage volume.
    pub name: String,
    /// Type of backend (e.g., 's3', 'gcs').
    pub backend: Option<String>,
    /// Size of the storage requested.
    pub size: String,
    /// Pre-provisioning instructions (e.g., download an artifact into it).
    pub provision: Vec<Provision>,
    /// Paths to preserve after execution.
    pub preserve: Vec<String>,
    /// Artifacts to upload.
    pub artifacts: Vec<Artifact>,
    /// Retention policy for the storage.
    pub retainment: Option<String>,
}

/// Instructions for provisioning data into storage.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Provision {
    /// Type of resource to provision.
    pub resource_type: String, // "secret", "config", "download", "artifact"
    /// Name of the provision block.
    pub name: String,
    /// Source reference.
    pub source: Option<String>,
    /// URL to download from.
    pub url: Option<String>,
    /// Destination path.
    pub destination: String,
    /// File mode/permissions.
    pub mode: Option<String>,
    /// Verification checksum.
    pub checksum: Option<String>,
    /// Optional specific origin.
    pub from: Option<String>,
}

/// Definition of an artifact to collect and upload.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Artifact {
    /// Logical name of the artifact.
    pub name: String,
    /// Local path to the artifact file or directory.
    pub path: String,
    /// Retention period.
    pub retention: String,
}

/// Mount definition for attaching storage to a container.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageMount {
    /// Name of the storage volume to mount.
    pub name: String,
    /// Path where the volume should be mounted inside the container.
    pub mount_path: String,
    /// Whether the mount should be read-only.
    pub read_only: Option<bool>,
    /// Step-specific paths to preserve when parking the volume, overriding the top-level storage config.
    #[serde(default)]
    pub preserve: Option<Vec<String>>,
}
