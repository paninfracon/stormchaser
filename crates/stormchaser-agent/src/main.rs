//! The Stormchaser agent. This executable runs inside task environments to facilitate integration with the engine.

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{exit, Command};
use std::time::Duration;
use tracing::info;

mod connections;
pub use connections::*;

mod artifacts;
pub use artifacts::*;

mod reports;
pub use reports::*;

mod confine;
pub use confine::*;

/// Builds the HTTP client used to talk to storage and the engine.
///
/// The agent runs inside arbitrary task images, many of which ship without a
/// system CA bundle, so the Mozilla roots are compiled in and merged with
/// whatever the platform provides.
pub fn http_client() -> Result<reqwest::Client> {
    let bundled_roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|cert| reqwest::Certificate::from_der(cert).ok());
    Ok(reqwest::Client::builder()
        .tls_certs_merge(bundled_roots)
        .build()?)
}

#[derive(Parser)]
#[command(author, about, long_about = None)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (rev: ", env!("VERGEN_GIT_SHA"), ", branch: ", env!("VERGEN_GIT_BRANCH"), ", built: ", env!("VERGEN_BUILD_TIMESTAMP"), ")"))]
/// Cli.
pub struct Cli {
    #[command(subcommand)]
    /// The command.
    pub command: Commands,
}

#[derive(Subcommand)]
/// Commands.
pub enum Commands {
    /// Runs a user command and then parks (uploads) the SFS storage
    Run {
        /// The JSON map of storage name to parking (put) URL
        #[arg(short, long)]
        parking_urls: String,

        /// The mount path for each storage (JSON map: name -> path)
        #[arg(short, long)]
        mount_paths: String,

        /// The JSON map of artifact name to {put_url, path}
        #[arg(short, long)]
        artifact_urls: Option<String>,

        /// The JSON list of test reports (name, path, format)
        #[arg(short, long, env = "STORMCHASER_TEST_REPORTS")]
        test_reports: Option<String>,

        /// The JSON map of report name to {put_url, remote_path, connection_id}
        #[arg(short, long, env = "STORMCHASER_REPORT_URLS")]
        report_urls: Option<String>,

        /// Docker client configuration file holding the credentials for OCI artifact pushes
        #[arg(long, env = "STORMCHASER_REGISTRY_CONFIG")]
        registry_config: Option<String>,

        /// Collect test reports from anywhere, not only from inside the mounts. Only for
        /// an agent running inside the step's own container, where it can read nothing
        /// the step cannot.
        #[arg(long)]
        reports_anywhere: bool,

        /// The user command to run
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Waits for a step's container to finish, then parks its storage and artifacts.
    ///
    /// Runs as a sidecar next to the step's container, sharing only its storage volumes.
    Park {
        /// File the runner fills with the step container's exit code once it has finished
        #[arg(long)]
        wait_for: String,

        /// The JSON map of storage name to parking (put) URL
        #[arg(long)]
        parking_urls: String,

        /// The mount path for each storage (JSON map: name -> path)
        #[arg(long)]
        mount_paths: String,

        /// The JSON map of artifact name to {put_url, path}
        #[arg(long)]
        artifact_urls: Option<String>,

        /// Docker client configuration file the runner provides for OCI artifact pushes
        #[arg(long)]
        registry_config: Option<String>,

        /// How long to wait for that file to appear before pushing without it
        #[arg(long, default_value_t = 120)]
        registry_config_wait_secs: u64,
    },
    /// Downloads and extracts a storage tarball with hash verification
    Unpark {
        /// The download (get) URL
        #[arg(short, long)]
        url: String,

        /// The expected SHA-256 hash (hex)
        #[arg(short, long)]
        expected_hash: Option<String>,

        /// The destination directory to extract to
        #[arg(short, long)]
        destination: String,

        /// If set, the downloaded file will not be extracted but saved directly to the destination path
        #[arg(long)]
        no_extract: bool,

        /// Optional file mode (octal string, e.g. "0755") to apply to the downloaded file.
        /// Only meaningful when `--no-extract` is set.
        #[arg(long)]
        mode: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Runners read what was parked, published and collected from INFO lines of this
    // output, so the level is fixed. `fmt::init()` would read RUST_LOG when Cargo
    // unifies tracing-subscriber's `env-filter` feature into this build (any build
    // together with the services), and then print errors only.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let cli = Cli::parse();
    run_agent(cli).await
}

/// Entry point for agent execution based on parsed CLI arguments.
pub async fn run_agent(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Unpark {
            url,
            expected_hash,
            destination,
            no_extract,
            mode,
        } => {
            unpark_storage(
                &url,
                expected_hash.as_deref(),
                &destination,
                no_extract,
                mode.as_deref(),
            )
            .await?;
            Ok(())
        }
        Commands::Park {
            wait_for,
            parking_urls,
            mount_paths,
            artifact_urls,
            registry_config,
            registry_config_wait_secs,
        } => {
            let urls: Value = serde_json::from_str(&parking_urls)?;
            let paths: Value = serde_json::from_str(&mount_paths)?;

            info!("Waiting for the step's container to finish");
            let code = tokio::select! {
                code = wait_for_exit_code(Path::new(&wait_for)) => code,
                // The pod is being deleted; there is nothing left to park
                _ = terminated() => exit(143),
            };
            if code != 0 {
                info!(
                    "Step exited with code {}, skipping storage and artifact parking",
                    code
                );
                exit(code);
            }

            if let Some(config) = &registry_config {
                wait_for_file(
                    Path::new(config),
                    Duration::from_secs(registry_config_wait_secs),
                )
                .await;
            }
            park_all(urls, paths, artifact_urls, registry_config.as_deref()).await?;
            Ok(())
        }
        Commands::Run {
            parking_urls,
            mount_paths,
            artifact_urls,
            test_reports,
            report_urls,
            registry_config,
            reports_anywhere,
            command,
        } => {
            let urls: Value = serde_json::from_str(&parking_urls)?;
            let paths: Value = serde_json::from_str(&mount_paths)?;

            if command.is_empty() {
                anyhow::bail!("No command provided to run");
            }

            // Arguments are not logged: they may hold interpolated secrets
            info!("Running user command: {}", command[0]);
            let mut child = Command::new(&command[0]).args(&command[1..]).spawn()?;

            let status = child.wait()?;
            info!("User command finished with status: {}", status);

            // Always collect reports even if command failed (test failures are common)
            if let Some(reports_json) = test_reports {
                let reports: Value = serde_json::from_str(&reports_json)?;
                let urls_val: Option<Value> =
                    report_urls.and_then(|u| serde_json::from_str(&u).ok());
                let roots = Roots::from_mount_paths(&paths);
                let roots = (!reports_anywhere).then_some(&roots);
                let collected_reports = collect_test_reports(reports, urls_val, roots).await?;
                if !collected_reports.is_empty() {
                    info!("Collected test reports: {:?}", collected_reports.keys());
                    let reports_out = serde_json::to_string(&collected_reports)?;
                    info!("Collected test reports JSON: {}", reports_out);
                    fs::write("/tmp/stormchaser_test_reports.json", reports_out)?;
                }
            }

            if status.success() {
                park_all(urls, paths, artifact_urls, registry_config.as_deref()).await?;
            } else {
                info!("User command failed, skipping storage and artifact parking");
            }

            exit(status.code().unwrap_or(1));
        }
    }
}

/// Parks storage volumes and artifacts, and logs what was parked for the runner to read.
async fn park_all(
    urls: Value,
    paths: Value,
    artifact_urls: Option<String>,
    registry_config: Option<&str>,
) -> Result<()> {
    let roots = Roots::from_mount_paths(&paths);
    let hashes = park_storage(urls, paths).await?;
    if !hashes.is_empty() {
        let hashes_json = serde_json::to_string(&hashes)?;
        info!("Parked storage hashes: {}", hashes_json);
        fs::write("/tmp/stormchaser_storage_hashes.json", hashes_json)?;
    }

    if let Some(artifacts_json) = artifact_urls {
        let artifacts: Value = serde_json::from_str(&artifacts_json)?;
        let artifact_meta = park_artifacts(artifacts, registry_config, &roots).await?;
        if !artifact_meta.is_empty() {
            let meta_json = serde_json::to_string(&artifact_meta)?;
            info!("Parked artifacts: {}", meta_json);
            fs::write("/tmp/stormchaser_artifact_meta.json", meta_json)?;
        }
    }
    Ok(())
}

/// Reads an exit code from `path`, if the file holds one yet.
fn read_exit_code(path: &Path) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Waits until the runner has written the step container's exit code to `path`.
async fn wait_for_exit_code(path: &Path) -> i32 {
    loop {
        if let Some(code) = read_exit_code(path) {
            return code;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Waits for `path` to exist, giving up after `limit`.
async fn wait_for_file(path: &Path, limit: Duration) {
    let deadline = tokio::time::Instant::now() + limit;
    while !path.is_file() {
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                "{} did not appear within {:?}; continuing without it",
                path.display(),
                limit
            );
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Resolves when the process is asked to stop.
async fn terminated() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            term.recv().await;
        }
        Err(_) => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_is_read_only_once_the_runner_has_written_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("worker-exit");
        assert_eq!(read_exit_code(&path), None);
        fs::write(&path, "").unwrap();
        assert_eq!(read_exit_code(&path), None);
        fs::write(&path, "137\n").unwrap();
        assert_eq!(read_exit_code(&path), Some(137));
        fs::write(&path, "0").unwrap();
        assert_eq!(read_exit_code(&path), Some(0));
    }

    #[tokio::test]
    async fn waiting_for_a_file_gives_up_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        wait_for_file(&dir.path().join("never"), Duration::from_millis(300)).await;
        assert!(started.elapsed() >= Duration::from_millis(300));

        let present = dir.path().join("config.json");
        fs::write(&present, "{}").unwrap();
        let started = std::time::Instant::now();
        wait_for_file(&present, Duration::from_secs(30)).await;
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn test_http_client_builds_with_bundled_roots() {
        http_client().expect("client with bundled roots should build");
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_http_client_verifies_public_tls_without_system_roots() {
        // SAFETY: only this ignored, manually run test touches the variable
        unsafe {
            std::env::set_var("SSL_CERT_FILE", "/dev/null");
            std::env::set_var("SSL_CERT_DIR", "/nonexistent");
        }
        let response = http_client()
            .unwrap()
            .get("https://example.com")
            .send()
            .await
            .expect("bundled roots should validate a public certificate");
        assert!(response.status().is_success());
    }

    #[test]
    fn test_cli_parsing_unpark() {
        let args = vec![
            "stormchaser-agent",
            "unpark",
            "--url",
            "http://example.com/data.tar.gz",
            "--expected-hash",
            "abcdef123456",
            "--destination",
            "/data",
        ];
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Unpark {
                url,
                expected_hash,
                destination,
                no_extract,
                mode,
            } => {
                assert_eq!(url, "http://example.com/data.tar.gz");
                assert_eq!(expected_hash.unwrap(), "abcdef123456");
                assert_eq!(destination, "/data");
                assert!(!no_extract);
                assert!(mode.is_none());
            }
            _ => panic!("Expected Unpark command"),
        }
    }

    #[test]
    fn test_cli_parsing_run() {
        let args = vec![
            "stormchaser-agent",
            "run",
            "--parking-urls",
            "{}",
            "--mount-paths",
            "{}",
            "--",
            "echo",
            "hello",
        ];
        let cli = Cli::parse_from(args);
        match cli.command {
            Commands::Run {
                parking_urls,
                mount_paths,
                command,
                ..
            } => {
                assert_eq!(parking_urls, "{}");
                assert_eq!(mount_paths, "{}");
                assert_eq!(command, vec!["echo", "hello"]);
            }
            _ => panic!("Expected Run command"),
        }
    }
}
