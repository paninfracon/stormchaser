pub mod hydration;
pub mod models;
pub mod routes;

use axum::{middleware, routing::post, Router};
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;

// Import Config, AppState, and setup from stormchaser_api to maintain identical configuration
use stormchaser_api::auth::opa::opa_middleware;
use stormchaser_api::config::Config;
use stormchaser_api::setup::build_app_state;

use routes::{hydrate_schema, hydrate_schema_direct};

fn main() -> anyhow::Result<()> {
    // Started as a jq worker (for a query's `jq_filter`), this process evaluates one
    // program and exits here, before any runtime exists.
    stormchaser_engine::jq_sandbox::run_worker_if_requested();
    run()
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    stormchaser_engine::jq_sandbox::install_current_exe_as_worker()?;

    tracing_subscriber::fmt::init();

    tracing::info!("Stormchaser Query Microservice starting");

    let config = Config::from_env(env::vars())?;

    // Uses the exact same state builder as stormchaser-api to guarantee TLS, DB, NATS,
    // OIDC, and OPA configuration parity.
    let state = build_app_state(config).await?;

    let app = Router::new()
        .route("/healthz", axum::routing::get(|| async { "OK" }))
        .route("/api/health", axum::routing::get(|| async { "OK" }))
        .route("/api/v1/schema/hydrate", post(hydrate_schema))
        .route("/api/v1/schema/hydrate-direct", post(hydrate_schema_direct))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            opa_middleware, // Reusing identical authentication and OPA gates
        ))
        .with_state(state);

    let port = env::var("PORT").unwrap_or_else(|_| "3001".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;

    tracing::info!("Stormchaser Query Microservice listening on {}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
