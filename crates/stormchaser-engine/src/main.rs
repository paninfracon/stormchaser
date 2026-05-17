use stormchaser_engine::config::Config;
use stormchaser_engine::server::run_engine;
use stormchaser_engine::telemetry::{init_telemetry, shutdown_telemetry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    let config = Config::from_env(std::env::vars())?;

    init_telemetry(&config.rust_log)?;

    let result = run_engine(config).await;
    shutdown_telemetry();
    result
}
