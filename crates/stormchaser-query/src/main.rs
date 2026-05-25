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

use routes::hydrate_schema;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
