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

/// Refuses a storage mount path that would cover part of a runner's helper container.
///
/// The helpers that unpack, park and publish a step's storage run the agent image with
/// the step's volumes mounted at the same paths the step uses. A volume holds whatever
/// earlier steps wrote to it, so one mounted over the agent binary, a shell, the loader or
/// its configuration, or over the place the runner puts registry credentials, would run
/// the workflow author's code as the helper or hand it those credentials.
pub fn ensure_mount_path_clear_of_helpers(mount_path: &str) -> anyhow::Result<()> {
    use std::path::{Component, Path};

    /// Directories the helpers execute, load or read credentials from: nothing may be
    /// mounted on, above or inside them.
    const HELPER_TREES: &[&str] = &[
        "/bin",
        "/sbin",
        "/lib",
        "/lib32",
        "/lib64",
        "/libx32",
        "/usr/bin",
        "/usr/sbin",
        "/usr/lib",
        "/usr/lib32",
        "/usr/lib64",
        "/usr/libexec",
        "/usr/local/bin",
        "/proc",
        "/sys",
        "/dev",
        "/stormchaser",
    ];

    let path = Path::new(mount_path);
    let plain = path.is_absolute()
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)));
    if !plain {
        anyhow::bail!(
            "Storage mount path '{}' must be absolute and free of '..'",
            mount_path
        );
    }

    // `/etc` holds the loader's preload list; mounts below it are harmless
    let covers_helper = Path::new("/etc").starts_with(path)
        || HELPER_TREES
            .iter()
            .any(|tree| path.starts_with(tree) || Path::new(tree).starts_with(path));
    if covers_helper {
        anyhow::bail!(
            "Storage mount path '{}' is reserved: it would cover system files of the runner's helper containers",
            mount_path
        );
    }
    Ok(())
}

#[cfg(test)]
mod mount_path_tests {
    use super::ensure_mount_path_clear_of_helpers;

    #[test]
    fn storage_mount_paths_that_cover_a_helper_are_refused() {
        for path in [
            "/",
            "/usr",
            "/usr/local",
            "/usr/local/bin",
            "/usr/local/bin/stormchaser-agent",
            "/bin",
            "/lib/x86_64-linux-gnu",
            "/etc",
            "/dev/shm",
            "/stormchaser",
            "/stormchaser/auth",
            "/workspace/../bin",
            "relative/path",
            "",
        ] {
            assert!(
                ensure_mount_path_clear_of_helpers(path).is_err(),
                "{path:?} should be refused"
            );
        }
    }

    #[test]
    fn ordinary_storage_mount_paths_are_accepted() {
        for path in [
            "/workspace",
            "/app",
            "/data/",
            "/usr/src/app",
            "/usr/local/share/data",
            "/etc/myapp",
            "/binaries",
            "/tmp/cache",
        ] {
            assert!(
                ensure_mount_path_clear_of_helpers(path).is_ok(),
                "{path:?} should be accepted"
            );
        }
    }
}
