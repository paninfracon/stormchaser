use axum::{
    body::Body,
    http::{self, Request, StatusCode},
};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env::var;
use std::sync::Arc;
use stormchaser_api::{app, AppState};
use stormchaser_model::OpaClient;
use tower::ServiceExt;

use axum::extract::connect_info::ConnectInfo;
use std::net::SocketAddr;

#[tokio::test]
async fn test_rate_limiting() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "2");

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url)
        .await
        .expect("Failed to connect to NATS");
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .connect(&db_url)
        .await
        .expect("Failed to connect to Postgres");

    let app = app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
    });

    let mut rng = rand::thread_rng();
    let ip_last_octet = rand::Rng::gen_range(&mut rng, 10..250);
    let addr = SocketAddr::from(([127, 0, 0, ip_last_octet], 12345));

    // We configured: per_second(1), burst_size(2)
    // Send a warmup request to initialize NATS KV bucket and avoid timeout during the measured burst
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method(http::Method::GET)
                .uri("/api/v1/auth/login")
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let mut rate_limited = false;
    for _ in 0..10 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(http::Method::GET)
                    .uri("/api/v1/auth/login")
                    .extension(ConnectInfo(addr))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            rate_limited = true;
            break;
        }
    }

    assert!(
        rate_limited,
        "Request should have been rate limited after burst"
    );
}
