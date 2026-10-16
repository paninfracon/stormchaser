//! Keeping what the agent reads inside the step's storage volumes.
//!
//! As a separate helper (the K8s sidecar, the Docker parking container) the agent holds
//! what the step's own container must not: the registry push credentials, and the upload
//! URLs in its arguments and environment. Every file it parks, publishes or reports on
//! was left by the step, so a symlink, a `..` or an absolute path must not lead it to
//! read anything outside the volumes it shares with the step.

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use tracing::warn;

/// The directories the agent may read from, resolved.
#[derive(Debug, Default)]
pub struct Roots(Vec<PathBuf>);

impl Roots {
    /// The mount paths of a JSON map of storage name to mount path. Mounts that do not
    /// exist are left out.
    pub fn from_mount_paths(paths: &Value) -> Self {
        let mut roots = Vec::new();
        for (name, path) in paths.as_object().into_iter().flatten() {
            let Some(path) = path.as_str() else { continue };
            match Path::new(path).canonicalize() {
                Ok(root) => roots.push(root),
                Err(e) => warn!("Mount '{}' at '{}' is unusable: {}", name, path, e),
            }
        }
        Roots(roots)
    }

    /// Resolves `path`, following any symlinks, and refuses it unless the result lies
    /// inside one of the roots.
    pub fn resolve(&self, path: &Path) -> Result<PathBuf> {
        let resolved = path
            .canonicalize()
            .with_context(|| format!("cannot resolve '{}'", path.display()))?;
        if self.0.iter().any(|root| resolved.starts_with(root)) {
            Ok(resolved)
        } else {
            bail!(
                "'{}' resolves to '{}', outside the step's storage",
                path.display(),
                resolved.display()
            )
        }
    }
}

/// Resolves `relative` inside the single directory `root`, refusing absolute paths, `..`
/// and symlinks that lead out of it.
pub fn resolve_within(root: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if !rel
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        bail!("'{}' must be a relative path without '..'", relative);
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve '{}'", root.display()))?;
    Roots(vec![root.clone()]).resolve(&root.join(rel))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    #[test]
    fn paths_inside_the_mounts_resolve() {
        let mount = tempdir().unwrap();
        fs::create_dir(mount.path().join("out")).unwrap();
        fs::write(mount.path().join("out/report.xml"), "x").unwrap();
        symlink("out/report.xml", mount.path().join("link.xml")).unwrap();

        let roots = Roots::from_mount_paths(&json!({"ws": mount.path()}));
        let real = mount.path().canonicalize().unwrap().join("out/report.xml");
        assert_eq!(
            roots.resolve(&mount.path().join("out/report.xml")).unwrap(),
            real
        );
        assert_eq!(roots.resolve(&mount.path().join("link.xml")).unwrap(), real);
    }

    #[test]
    fn symlinks_and_dot_dot_out_of_the_mounts_are_refused() {
        let mount = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("config.json"), "secret").unwrap();
        symlink(
            outside.path().join("config.json"),
            mount.path().join("leak"),
        )
        .unwrap();
        symlink(outside.path(), mount.path().join("dir")).unwrap();

        let roots = Roots::from_mount_paths(&json!({"ws": mount.path()}));
        assert!(roots.resolve(&mount.path().join("leak")).is_err());
        assert!(roots
            .resolve(&mount.path().join("dir/config.json"))
            .is_err());
        assert!(roots
            .resolve(
                &mount
                    .path()
                    .join("../")
                    .join(outside.path().file_name().unwrap())
            )
            .is_err());
        assert!(roots.resolve(Path::new("/proc/self/environ")).is_err());
        assert!(Roots::default()
            .resolve(&mount.path().join("leak"))
            .is_err());

        assert!(resolve_within(mount.path(), "leak").is_err());
        assert!(resolve_within(mount.path(), "dir").is_err());
        assert!(resolve_within(mount.path(), "../x").is_err());
        assert!(resolve_within(mount.path(), "/etc/passwd").is_err());
    }
}
