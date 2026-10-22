pub mod hydration;
pub mod limits;
pub mod models;
pub mod routes;
pub mod state;

use axum::{middleware, routing::post, Router};
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;

use state::QueryState;
use stormchaser_core::auth::opa::opa_middleware;
use stormchaser_core::config::Config;

use routes::{hydrate_schema, hydrate_schema_direct};

fn main() -> anyhow::Result<()> {
    // Started as a jq worker (for a query's `jq_filter`), this process evaluates one
    // program and exits here, before any runtime exists.
    stormchaser_core::jq_sandbox::run_worker_if_requested();
    run()
}

#[tokio::main]
async fn run() -> anyhow::Result<()> {
    stormchaser_core::jq_sandbox::install_current_exe_as_worker()?;

    tracing_subscriber::fmt::init();

    tracing::info!("Stormchaser Query Microservice starting");

    let config = Config::from_env(env::vars())?;

    // The same settings, token checks and policy as the API
    let tls = stormchaser_core::listener::tls_from_config(&config).await?;
    let state = QueryState::from_config(&config).await?;

    let app = Router::new()
        .route("/healthz", axum::routing::get(|| async { "OK" }))
        .route("/api/health", axum::routing::get(|| async { "OK" }))
        .route("/api/v1/schema/hydrate", post(hydrate_schema))
        .route("/api/v1/schema/hydrate-direct", post(hydrate_schema_direct))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            opa_middleware::<QueryState>,
        ))
        .with_state(state);

    let port = env::var("PORT").unwrap_or_else(|_| "3001".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;

    tracing::info!(
        "Stormchaser Query Microservice listening on {} ({})",
        addr,
        if tls.is_some() { "https" } else { "http" }
    );
    let listener = TcpListener::bind(addr).await?;
    stormchaser_core::listener::serve(listener, app, tls, std::future::pending()).await?;

    Ok(())
}
