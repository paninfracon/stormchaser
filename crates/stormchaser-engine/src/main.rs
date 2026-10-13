use stormchaser_engine::config::Config;
use stormchaser_engine::server::run_engine;
use stormchaser_engine::telemetry::{init_telemetry, shutdown_telemetry};

fn main() -> anyhow::Result<()> {
    // Started as a jq worker, this process evaluates one program and exits here,
    // before any runtime exists.
    stormchaser_engine::jq_sandbox::run_worker_if_requested();
    run()
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    stormchaser_engine::jq_sandbox::install_current_exe_as_worker()?;

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    let config = Config::from_env(std::env::vars())?;

    init_telemetry(&config.rust_log)?;

    let result = run_engine(config).await;
    shutdown_telemetry();
    result
}
