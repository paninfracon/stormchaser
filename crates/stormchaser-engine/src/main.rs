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

    // `stormchaser-engine migrate` applies the database migrations and exits. It is the
    // one thing that applies them; the services wait for it.
    if std::env::args().nth(1).as_deref() == Some("migrate") {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .init();
        let pool = stormchaser_engine::setup::connect_database(&config).await?;
        return stormchaser_core::db::migrations::run(&pool).await;
    }

    init_telemetry(&config.rust_log)?;

    let result = run_engine(config).await;
    shutdown_telemetry();
    result
}
