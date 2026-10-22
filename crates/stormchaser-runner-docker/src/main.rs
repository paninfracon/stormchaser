use anyhow::{Context, Result};
use bollard::Docker;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub mod backend;
pub mod container_machine;
pub mod handler;
pub mod parsing;
pub mod reaper;
pub mod traits;

#[derive(Debug, Clone)]
pub struct Config {
    pub nats_url: String,
    pub runner_id: String,
    pub encryption_key: Option<String>,
    pub loki_url: Option<String>,
    pub rust_log: String,
}

impl Config {
    /// Creates a new `Config` from an environment variable iterator.
    pub fn from_env<I, K, V>(env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut explicit_runner_id = None;
        let mut hostname = None;
        let mut encryption_key = None;
        let mut loki_url = None;
        let mut rust_log = "stormchaser_runner_docker=info".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUNNER_ID" => explicit_runner_id = Some(v.as_ref().to_string()),
                "HOSTNAME" => hostname = Some(v.as_ref().to_string()),
                // Empty, as compose passes it when no key was generated, means none
                "STORMCHASER_STATE_ENCRYPTION_KEY" if !v.as_ref().is_empty() => {
                    encryption_key = Some(v.as_ref().to_string())
                }
                "LOKI_URL" => loki_url = Some(v.as_ref().to_string()),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                _ => {}
            }
        }

        // Stable across restarts of this runner, so it comes back as itself
        let runner_id = stormchaser_model::runner::runner_id(
            "docker",
            explicit_runner_id.as_deref(),
            hostname
                .or_else(stormchaser_model::runner::kernel_hostname)
                .as_deref(),
        );

        Self {
            nats_url,
            runner_id,
            encryption_key,
            loki_url,
            rust_log,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env(std::env::vars());
    run_runner(config).await
}

/// Main entry point for the Stormchaser Docker Runner.
/// Initializes the Docker client, connects to NATS, registers the runner,
/// and begins processing tasks from the JetStream queue.
pub async fn run_runner(config: Config) -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(&config.rust_log))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!(
        "Starting Stormchaser Docker Runner {} (rev: {}, branch: {}, built: {}): {}",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_BUILD_TIMESTAMP"),
        config.runner_id
    );

    let docker =
        Docker::connect_with_local_defaults().context("Failed to connect to Docker daemon")?;
    let backend = Arc::new(backend::DockerBackend {
        docker,
        loki_url: config.loki_url,
    });
    stormchaser_runner_core::run(
        backend,
        stormchaser_runner_core::RunnerSettings {
            nats_url: config.nats_url,
            runner_id: config.runner_id,
            encryption_key: config.encryption_key,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_defaults() {
        let env: Vec<(String, String)> = vec![];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://localhost:4222");
        assert!(config.encryption_key.is_none());
        assert_eq!(config.rust_log, "stormchaser_runner_docker=info");
        // runner_id is random, just assert it's not empty
        assert!(!config.runner_id.is_empty());
    }

    #[test]
    fn test_config_from_env_custom() {
        let env = vec![
            ("NATS_URL".to_string(), "nats://remote:4222".to_string()),
            ("RUNNER_ID".to_string(), "my-runner".to_string()),
            (
                "STORMCHASER_STATE_ENCRYPTION_KEY".to_string(),
                "my-key".to_string(),
            ),
            ("RUST_LOG".to_string(), "debug".to_string()),
        ];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://remote:4222");
        assert_eq!(config.runner_id, "my-runner");
        assert_eq!(config.encryption_key.unwrap(), "my-key");
        assert_eq!(config.rust_log, "debug");
    }
}
