use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::process::Command;
use tracing::info;

mod storage;
pub use storage::*;

mod artifacts;
pub use artifacts::*;

mod reports;
pub use reports::*;

#[derive(Parser)]
#[command(author, about, long_about = None)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (rev: ", env!("VERGEN_GIT_SHA"), ", branch: ", env!("VERGEN_GIT_BRANCH"), ", built: ", env!("VERGEN_BUILD_TIMESTAMP"), ")"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
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

        /// The JSON map of report name to {put_url, remote_path, backend_id}
        #[arg(short, long, env = "STORMCHASER_REPORT_URLS")]
        report_urls: Option<String>,

        /// The user command to run
        #[arg(last = true)]
        command: Vec<String>,
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
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    run_agent(cli).await
}

pub async fn run_agent(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Unpark {
            url,
            expected_hash,
            destination,
        } => {
            unpark_storage(&url, expected_hash.as_deref(), &destination).await?;
            Ok(())
        }
        Commands::Run {
            parking_urls,
            mount_paths,
            artifact_urls,
            test_reports,
            report_urls,
            command,
        } => {
            let urls: Value = serde_json::from_str(&parking_urls)?;
            let paths: Value = serde_json::from_str(&mount_paths)?;

            if command.is_empty() {
                anyhow::bail!("No command provided to run");
            }

            info!("Running user command: {:?}", command);
            let mut child = Command::new(&command[0]).args(&command[1..]).spawn()?;

            let status = child.wait()?;
            info!("User command finished with status: {}", status);

            // Always collect reports even if command failed (test failures are common)
            if let Some(reports_json) = test_reports {
                let reports: Value = serde_json::from_str(&reports_json)?;
                let urls_val: Option<Value> =
                    report_urls.and_then(|u| serde_json::from_str(&u).ok());
                let collected_reports = collect_test_reports(reports, urls_val).await?;
                if !collected_reports.is_empty() {
                    info!("Collected test reports: {:?}", collected_reports.keys());
                    let reports_out = serde_json::to_string(&collected_reports)?;
                    info!("Collected test reports JSON: {}", reports_out);
                    std::fs::write("/tmp/stormchaser_test_reports.json", reports_out)?;
                }
            }

            if status.success() {
                let hashes = park_storage(urls, paths).await?;
                // Print hashes to stdout for the runner to capture if needed
                if !hashes.is_empty() {
                    // Write hashes to a known file for the runner to read
                    let hashes_json = serde_json::to_string(&hashes)?;
                    info!("Parked storage hashes: {}", hashes_json);
                    std::fs::write("/tmp/stormchaser_storage_hashes.json", hashes_json)?;
                }

                if let Some(artifacts_json) = artifact_urls {
                    let artifacts: Value = serde_json::from_str(&artifacts_json)?;
                    let artifact_meta = park_artifacts(artifacts).await?;
                    if !artifact_meta.is_empty() {
                        let meta_json = serde_json::to_string(&artifact_meta)?;
                        info!("Parked artifacts: {}", meta_json);
                        std::fs::write("/tmp/stormchaser_artifact_meta.json", meta_json)?;
                    }
                }
            } else {
                info!("User command failed, skipping storage and artifact parking");
            }

            std::process::exit(status.code().unwrap_or(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            } => {
                assert_eq!(url, "http://example.com/data.tar.gz");
                assert_eq!(expected_hash.unwrap(), "abcdef123456");
                assert_eq!(destination, "/data");
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
