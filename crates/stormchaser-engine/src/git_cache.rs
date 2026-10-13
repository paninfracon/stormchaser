use anyhow::{Context, Result};
use git2::{
    build::{CheckoutBuilder, RepoBuilder},
    Repository,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Largest file the engine will read out of a repository.
const MAX_REPO_FILE_BYTES: usize = 5 * 1024 * 1024;

/// Largest amount of data, in MiB, the engine will download for one clone or fetch.
/// Defaults to 1024.
pub const GIT_MAX_CLONE_MB_ENV: &str = "STORMCHASER_GIT_MAX_CLONE_MB";

/// Longest a clone or fetch may spend downloading, in seconds. Defaults to 300.
pub const GIT_CLONE_TIMEOUT_SECS_ENV: &str = "STORMCHASER_GIT_CLONE_TIMEOUT_SECS";

/// How long a cached branch or tag is used before it is fetched again, in seconds.
/// Defaults to 60. A revision given as a full commit id never changes and is never
/// refetched.
pub const GIT_CACHE_REFRESH_SECS_ENV: &str = "STORMCHASER_GIT_CACHE_REFRESH_SECS";

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Fetch options that stop a download once it is too large or has taken too long.
///
/// Repository URLs come from connections any API user can create, so the far end may
/// serve an endless or enormous pack. The limits apply while objects are being
/// received; a server that stalls before sending anything is not covered.
fn limited_fetch_options<'a>() -> git2::FetchOptions<'a> {
    let max_bytes = env_u64(GIT_MAX_CLONE_MB_ENV, 1024).saturating_mul(1024 * 1024);
    let deadline = Instant::now() + Duration::from_secs(env_u64(GIT_CLONE_TIMEOUT_SECS_ENV, 300));

    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.transfer_progress(move |progress| {
        progress.received_bytes() as u64 <= max_bytes && Instant::now() < deadline
    });
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(callbacks);
    options
}

/// Whether `rev` is a full commit id, which always names the same content.
fn is_commit_id(rev: &str) -> bool {
    matches!(rev.len(), 40 | 64) && rev.bytes().all(|b| b.is_ascii_hexdigit())
}

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
    /// How long a cached branch or tag is used before it is fetched again.
    refresh_after: Duration,
    /// When each cached branch or tag was last fetched.
    refreshed: Mutex<HashMap<PathBuf, Instant>>,
}

impl GitCache {
    /// New.
    pub fn new<P: AsRef<Path>>(base_dir: P) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            refresh_after: Duration::from_secs(env_u64(GIT_CACHE_REFRESH_SECS_ENV, 60)),
            refreshed: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the target directory for a repo/rev pair.
    fn get_target_dir(&self, repo_url: &str, rev: &str) -> PathBuf {
        let repo_hash = self.hash_url(repo_url);
        let rev_hash = self.hash_rev(rev);
        // "v2": entries written before clones were staged may hold a repository whose
        // HEAD is not the revision they are filed under, so they are not reused.
        self.base_dir
            .join("checkouts-v2")
            .join(&repo_hash)
            .join(&rev_hash)
    }

    /// Initializes a repository cache entry if it doesn't exist.
    /// Uses metadata-only clone (no initial checkout).
    pub fn init_repo(&self, repo_url: &str, rev: &str) -> Result<PathBuf> {
        validate_repo_url(repo_url)?;
        self.prepare(repo_url, rev)
    }

    /// Returns the cache entry for `repo_url` at `rev`, cloning or refreshing it first.
    fn prepare(&self, repo_url: &str, rev: &str) -> Result<PathBuf> {
        let target_dir = self.get_target_dir(repo_url, rev);

        if target_dir.exists() {
            if !is_commit_id(rev) && self.refresh_is_due(&target_dir) {
                // A branch or tag moves; without this a run would use whatever the
                // branch held the first time anyone ran it.
                if let Err(e) = self.refresh(&target_dir, rev) {
                    warn!(
                        "Could not refresh {} at {}; using the cached copy: {:#}",
                        repo_url, rev, e
                    );
                }
            }
            return Ok(target_dir);
        }

        info!(
            "Initializing metadata-only cache for {} at rev {} in {:?}",
            repo_url, rev, target_dir
        );

        // Clone next to the entry and move it into place only once it is complete and
        // on the right revision. A clone that fails, or a revision that does not exist,
        // then leaves nothing behind for the next run to mistake for a good entry.
        let parent = target_dir
            .parent()
            .context("Cache entry has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let staging = parent.join(format!(".staging-{}", uuid::Uuid::new_v4()));

        if let Err(e) = self.clone_into(repo_url, rev, &staging) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&staging, &target_dir) {
            let _ = std::fs::remove_dir_all(&staging);
            // Another run cloned the same revision first; its entry is as good as ours
            if !target_dir.exists() {
                return Err(e).context("Failed to move the clone into the cache");
            }
        }
        self.mark_refreshed(&target_dir);

        Ok(target_dir)
    }

    fn clone_into(&self, repo_url: &str, rev: &str, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;

        // Clone WITHOUT checkout
        // We could add depth(1) here but it can be problematic with specific refs/shas
        let mut empty_checkout = CheckoutBuilder::new();
        empty_checkout.dry_run();

        let repo = RepoBuilder::new()
            .fetch_options(limited_fetch_options())
            .with_checkout(empty_checkout) // Empty checkout
            .clone(repo_url, dir)
            .with_context(|| format!("Failed to clone metadata for {}", repo_url))?;

        // Resolve and set HEAD to the revision
        self.checkout_revision_metadata(&repo, rev)
    }

    /// Whether a cached branch or tag is old enough to fetch again. Claims the refresh,
    /// so concurrent runs of the same revision do not all fetch at once.
    fn refresh_is_due(&self, target_dir: &Path) -> bool {
        let mut refreshed = self.refreshed.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let due = refreshed
            .get(target_dir)
            .is_none_or(|at| now.duration_since(*at) >= self.refresh_after);
        if due {
            refreshed.insert(target_dir.to_path_buf(), now);
        }
        due
    }

    fn mark_refreshed(&self, target_dir: &Path) {
        self.refreshed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(target_dir.to_path_buf(), Instant::now());
    }

    /// Fetches the remote's branches and tags and moves HEAD to where `rev` now points.
    fn refresh(&self, target_dir: &Path, rev: &str) -> Result<()> {
        let repo = Repository::open(target_dir)?;
        repo.find_remote("origin")?
            .fetch(
                &[
                    "+refs/heads/*:refs/remotes/origin/*",
                    "+refs/tags/*:refs/tags/*",
                ],
                Some(&mut limited_fetch_options()),
                None,
            )
            .context("Failed to fetch")?;
        self.checkout_revision_metadata(&repo, rev)
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
        // The remote's branch first: after a fetch it is the only ref that has moved,
        // while a local branch of the same name still points at the old commit.
        let commit = [
            format!("refs/remotes/origin/{rev}"),
            format!("refs/tags/{rev}"),
            rev.to_string(),
        ]
        .iter()
        .find_map(|spec| repo.revparse_single(spec).ok())
        .with_context(|| format!("Failed to find revision {}", rev))?
        .peel_to_commit()
        .with_context(|| format!("Revision {} is not a commit", rev))?;

        // Note: we DON'T call checkout_tree here if we want metadata only.
        // We just set HEAD.
        repo.set_head_detached(commit.id())
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

    /// Commits `content` as `flow.storm` on the current branch of the repository at `dir`.
    fn commit_flow(dir: &Path, content: &str) -> String {
        let repo = match Repository::open(dir) {
            Ok(repo) => repo,
            Err(_) => Repository::init(dir).unwrap(),
        };
        std::fs::write(dir.join("flow.storm"), content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("flow.storm")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("test", "test@example.com").unwrap();
        let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, content, &tree, &parents)
            .unwrap();
        let branch = repo.head().unwrap().shorthand().unwrap().to_string();
        branch
    }

    fn entries(dir: &Path) -> Vec<String> {
        match std::fs::read_dir(dir) {
            Ok(read) => read
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    #[test]
    fn a_failed_clone_leaves_nothing_for_the_next_run_to_trust() {
        let origin = tempdir().unwrap();
        let branch = commit_flow(origin.path(), "v1");
        let url = origin.path().to_str().unwrap();
        let tmp = tempdir().unwrap();
        let cache = GitCache::new(tmp.path());

        // The repository exists but the revision does not
        assert!(cache.prepare(url, "no-such-branch").is_err());
        let bad_entry = cache.get_target_dir(url, "no-such-branch");
        assert!(!bad_entry.exists());
        assert!(
            entries(bad_entry.parent().unwrap()).is_empty(),
            "staging directories were left behind"
        );
        // Asking again is still an error, not the default branch's content
        assert!(cache.prepare(url, "no-such-branch").is_err());

        // Nor does a repository that cannot be cloned at all
        assert!(cache
            .prepare(tmp.path().join("missing").to_str().unwrap(), &branch)
            .is_err());

        let dir = cache.prepare(url, &branch).unwrap();
        assert_eq!(read_blob_at_head(&dir, "flow.storm").unwrap(), "v1");
    }

    #[test]
    fn a_cached_branch_follows_the_remote_but_a_commit_id_does_not() {
        let origin = tempdir().unwrap();
        let branch = commit_flow(origin.path(), "v1");
        let first_commit = Repository::open(origin.path())
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap()
            .to_string();
        let url = origin.path().to_str().unwrap();
        let tmp = tempdir().unwrap();
        let mut cache = GitCache::new(tmp.path());
        cache.refresh_after = Duration::ZERO;

        let by_branch = cache.prepare(url, &branch).unwrap();
        cache.prepare(url, &first_commit).unwrap();
        assert_eq!(read_blob_at_head(&by_branch, "flow.storm").unwrap(), "v1");

        commit_flow(origin.path(), "v2");

        let by_branch = cache.prepare(url, &branch).unwrap();
        assert_eq!(read_blob_at_head(&by_branch, "flow.storm").unwrap(), "v2");
        let by_commit = cache.prepare(url, &first_commit).unwrap();
        assert_eq!(read_blob_at_head(&by_commit, "flow.storm").unwrap(), "v1");

        // Inside the refresh interval the cached copy is used as it is
        cache.refresh_after = Duration::from_secs(3600);
        cache.mark_refreshed(&by_branch);
        commit_flow(origin.path(), "v3");
        let by_branch = cache.prepare(url, &branch).unwrap();
        assert_eq!(read_blob_at_head(&by_branch, "flow.storm").unwrap(), "v2");
    }

    #[test]
    fn only_full_commit_ids_are_treated_as_immutable() {
        assert!(is_commit_id("0123456789abcdef0123456789abcdef01234567"));
        assert!(!is_commit_id("0123456"));
        assert!(!is_commit_id("main"));
        assert!(!is_commit_id("zzzz456789abcdef0123456789abcdef01234567"));
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
