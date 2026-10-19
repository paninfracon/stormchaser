mod backend;
mod cancel;
mod cluster;
mod config;
mod job_machine;
mod reaper;
mod scanner;
mod task;

use anyhow::{Context, Result};
use kube::Client;
use serde_json::Value;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::cluster::ClusterPool;
use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env(std::env::vars());
    run_runner(config).await
}

/// Run runner.
pub async fn run_runner(config: Config) -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(&config.rust_log))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let step_namespace = config.resolve_step_namespace()?;

    info!(
        "Starting Stormchaser K8s Runner {} (rev: {}, branch: {}, built: {}): {}",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_BUILD_TIMESTAMP"),
        config.runner_id
    );

    let cluster_pool = Arc::new(ClusterPool::new());

    let local_client = Client::try_default()
        .await
        .context("Failed to initialize local Kubernetes client")?;

    let version_resp = local_client
        .request_text(http::Request::builder().uri("/version").body(vec![])?)
        .await?;
    let version_data: Value = serde_json::from_str(&version_resp)?;
    let major = version_data["major"].as_str().unwrap_or("0");
    let minor = version_data["minor"].as_str().unwrap_or("0");
    let local_version = format!("{}.{}", major, minor.replace('+', ""));

    cluster_pool.add_client("local", local_client, local_version);
    info!("Default 'local' Kubernetes client initialized");

    let backend = Arc::new(backend::K8sBackend {
        cluster_pool,
        step_namespace,
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
