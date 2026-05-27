use anyhow::{Context, Result};
use git2::{
    build::{CheckoutBuilder, RepoBuilder},
    Repository,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tracing::{debug, info};

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
}
