use anyhow::{Context, Result};
use git2::{
    build::{CheckoutBuilder, RepoBuilder},
    Repository,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Largest file the engine will read out of a repository.
const MAX_REPO_FILE_BYTES: usize = 5 * 1024 * 1024;

/// Comma-separated URL schemes repositories may be cloned over. Defaults to `https,ssh`.
pub const GIT_ALLOWED_SCHEMES_ENV: &str = "STORMCHASER_GIT_ALLOWED_SCHEMES";

/// Rejects repository URLs that are local paths or use a transport other than the
/// allowed ones.
///
/// Repository URLs come from connections that any API user can create. `file://` URLs
/// and bare paths would let them clone other tenants' cached checkouts or any repository
/// on the host, and `ext::` style transports run commands.
pub fn validate_repo_url(repo_url: &str) -> Result<()> {
    let allowed = std::env::var(GIT_ALLOWED_SCHEMES_ENV).unwrap_or_else(|_| "https,ssh".into());
    let allowed: Vec<&str> = allowed.split(',').map(str::trim).collect();

    let scheme = match repo_url.split_once("://") {
        Some((scheme, _)) => scheme.to_ascii_lowercase(),
        // scp-like syntax (git@host:org/repo.git) is SSH; anything else is a local path.
        None => {
            let is_scp_like = repo_url.split_once(':').is_some_and(|(host, path)| {
                host.contains('@') && !host.contains('/') && !path.is_empty()
            });
            if is_scp_like {
                "ssh".to_string()
            } else {
                anyhow::bail!("Repository URL must use one of: {}", allowed.join(", "));
            }
        }
    };

    if !allowed.contains(&scheme.as_str()) {
        anyhow::bail!(
            "Repository URL scheme '{}' is not allowed (allowed: {})",
            scheme,
            allowed.join(", ")
        );
    }
    Ok(())
}

/// Checks that `path` is relative and never climbs out of the repository.
pub fn validate_relative_path(path: &str) -> Result<&Path> {
    let p = Path::new(path);
    if p.as_os_str().is_empty()
        || p.is_absolute()
        || p.components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        anyhow::bail!("Path {} is invalid (absolute or contains '..')", path);
    }
    Ok(p)
}

/// Reads a file from the repository's HEAD commit, straight from the git object database.
///
/// The working tree is never consulted, so a symlink committed to the repository cannot
/// be used to read files outside it (`workflow.storm -> /proc/self/environ`). Symlink
/// entries and anything that is not a regular file are refused.
pub fn read_blob_at_head(repo_dir: &Path, path: &str) -> Result<String> {
    let rel = validate_relative_path(path)?;
    let repo = Repository::open(repo_dir)
        .with_context(|| format!("Failed to open repository cache at {:?}", repo_dir))?;
    let tree = repo
        .head()
        .and_then(|head| head.peel_to_tree())
        .context("Failed to resolve the checked-out revision")?;

    let entry = tree
        .get_path(rel)
        .map_err(|_| anyhow::anyhow!("File {} not found in repo", path))?;

    // 0o120000 is a symlink; 0o160000 a submodule.
    if entry.filemode() == 0o120000 {
        anyhow::bail!("File {} is a symbolic link, which is not allowed", path);
    }
    if entry.kind() != Some(git2::ObjectType::Blob) {
        anyhow::bail!("Path {} is not a regular file", path);
    }

    let blob = repo.find_blob(entry.id())?;
    if blob.size() > MAX_REPO_FILE_BYTES {
        anyhow::bail!(
            "File {} is larger than the {} byte limit",
            path,
            MAX_REPO_FILE_BYTES
        );
    }
    String::from_utf8(blob.content().to_vec())
        .map_err(|_| anyhow::anyhow!("File {} is not valid UTF-8", path))
}

/// Resolves `path` inside a checked-out repository for a tool that needs a real file,
/// refusing symlinks at every level so the result cannot point outside `repo_dir`.
pub fn safe_worktree_path(repo_dir: &Path, path: &str) -> Result<PathBuf> {
    let rel = validate_relative_path(path)?;
    let mut current = repo_dir.to_path_buf();
    for component in rel.components() {
        current.push(component);
        let meta = std::fs::symlink_metadata(&current)
            .with_context(|| format!("File {} not found in repo", path))?;
        if meta.file_type().is_symlink() {
            anyhow::bail!(
                "Path {} goes through a symbolic link, which is not allowed",
                path
            );
        }
    }
    if !current.is_file() {
        anyhow::bail!("Path {} is not a regular file", path);
    }
    Ok(current)
}

/// Gitcache.
pub struct GitCache {
    base_dir: PathBuf,
}

impl GitCache {
    /// New.
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
        }
    }

    /// Returns the target directory for a repo/rev pair.
    fn get_target_dir(&self, repo_url: &str, rev: &str) -> PathBuf {
        let repo_hash = self.hash_url(repo_url);
        let rev_hash = self.hash_rev(rev);
        self.base_dir
            .join("checkouts")
            .join(&repo_hash)
            .join(&rev_hash)
    }

    /// Initializes a repository cache entry if it doesn't exist.
    /// Uses metadata-only clone (no initial checkout).
    pub fn init_repo(&self, repo_url: &str, rev: &str) -> Result<PathBuf> {
        validate_repo_url(repo_url)?;
        let target_dir = self.get_target_dir(repo_url, rev);

        if target_dir.exists() {
            return Ok(target_dir);
        }

        info!(
            "Initializing metadata-only cache for {} at rev {} in {:?}",
            repo_url, rev, target_dir
        );

        std::fs::create_dir_all(&target_dir)?;

        // Clone WITHOUT checkout
        let fetch_opts = git2::FetchOptions::new();
        // We could add depth(1) here but it can be problematic with specific refs/shas

        let mut empty_checkout = CheckoutBuilder::new();
        empty_checkout.dry_run();

        let repo = RepoBuilder::new()
            .fetch_options(fetch_opts)
            .with_checkout(empty_checkout) // Empty checkout
            .clone(repo_url, &target_dir)
            .with_context(|| format!("Failed to clone metadata for {}", repo_url))?;

        // Resolve and set HEAD to the revision
        self.checkout_revision_metadata(&repo, rev)?;

        Ok(target_dir)
    }

    /// Ensures specific files/folders are present in the working directory.
    /// This can be called multiple times to add more files to the same cache entry.
    pub fn ensure_files(&self, repo_url: &str, rev: &str, paths: &[String]) -> Result<PathBuf> {
        let target_dir = self.init_repo(repo_url, rev)?;
        let repo = Repository::open(&target_dir)?;

        if paths.is_empty() {
            return Ok(target_dir);
        }

        debug!("Ensuring paths {:?} are present in {:?}", paths, target_dir);

        let mut cb = CheckoutBuilder::new();
        cb.force(); // Overwrite if exists

        for path in paths {
            cb.path(path);
        }

        repo.checkout_head(Some(&mut cb))
            .with_context(|| format!("Failed to checkout paths {:?} for rev {}", paths, rev))?;

        Ok(target_dir)
    }

    /// Reads one file at `rev` without touching the working tree. See [`read_blob_at_head`].
    pub fn read_file(&self, repo_url: &str, rev: &str, path: &str) -> Result<String> {
        let target_dir = self.init_repo(repo_url, rev)?;
        read_blob_at_head(&target_dir, path)
    }

    fn hash_url(&self, url: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        hex::encode(hasher.finalize())[..16].to_string()
    }

    fn hash_rev(&self, rev: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(rev.as_bytes());
        hex::encode(hasher.finalize())[..16].to_string()
    }

    fn checkout_revision_metadata(&self, repo: &Repository, rev: &str) -> Result<()> {
        let (object, reference) = repo
            .revparse_ext(rev)
            .with_context(|| format!("Failed to find revision {}", rev))?;

        // Note: we DON'T call checkout_tree here if we want metadata only.
        // We just set HEAD.

        match reference {
            Some(ref r) if r.is_branch() => repo.set_head(r.name().unwrap()),
            _ => repo.set_head_detached(object.id()),
        }
        .with_context(|| format!("Failed to set HEAD to revision {}", rev))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_git_cache_path_generation() {
        let tmp = tempdir().unwrap();
        let cache = GitCache::new(tmp.path());

        let url = "https://github.com/example/repo.git";
        let rev = "main";

        let _path = cache.get_target_dir(url, rev);

        assert_eq!(cache.hash_url(url).len(), 16);
        assert_eq!(cache.hash_rev(rev).len(), 16);
    }

    /// Builds a repository whose HEAD commit holds a regular file and, on Unix, a symlink
    /// that points outside it.
    fn repo_with_symlink(dir: &Path) {
        let repo = Repository::init(dir).unwrap();
        std::fs::create_dir_all(dir.join("flows")).unwrap();
        std::fs::write(dir.join("flows/ok.storm"), "workflow {}").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hostname", dir.join("flows/leak.storm")).unwrap();

        let mut index = repo.index().unwrap();
        index
            .add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    #[test]
    fn test_read_blob_at_head_reads_regular_files_only() {
        let tmp = tempdir().unwrap();
        repo_with_symlink(tmp.path());

        assert_eq!(
            read_blob_at_head(tmp.path(), "flows/ok.storm").unwrap(),
            "workflow {}"
        );
        assert!(read_blob_at_head(tmp.path(), "flows/missing.storm")
            .unwrap_err()
            .to_string()
            .contains("not found"));
        assert!(read_blob_at_head(tmp.path(), "flows").is_err());
        for bad in ["../outside", "/etc/passwd", "flows/../../x", ""] {
            assert!(read_blob_at_head(tmp.path(), bad).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_symlinks_in_a_repository_are_never_followed() {
        let tmp = tempdir().unwrap();
        repo_with_symlink(tmp.path());

        let err = read_blob_at_head(tmp.path(), "flows/leak.storm").unwrap_err();
        assert!(err.to_string().contains("symbolic link"));

        let err = safe_worktree_path(tmp.path(), "flows/leak.storm").unwrap_err();
        assert!(err.to_string().contains("symbolic link"));
        assert!(safe_worktree_path(tmp.path(), "flows/ok.storm").is_ok());

        // A symlinked directory on the way to the file is refused too.
        std::os::unix::fs::symlink("/etc", tmp.path().join("etc-link")).unwrap();
        assert!(safe_worktree_path(tmp.path(), "etc-link/hostname").is_err());
    }

    #[test]
    fn test_validate_repo_url() {
        for ok in [
            "https://github.com/example/repo.git",
            "ssh://git@github.com/example/repo.git",
            "git@github.com:example/repo.git",
        ] {
            assert!(validate_repo_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "file:///var/cache/checkouts/abc",
            "/var/cache/checkouts/abc",
            "../other-repo",
            "http://git.internal/repo.git",
            "git://example.com/repo.git",
            "ext::sh -c id",
            "",
        ] {
            assert!(validate_repo_url(bad).is_err(), "{bad}");
        }
    }
}
