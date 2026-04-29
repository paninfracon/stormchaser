use axum::{
    body::Body,
    http::{self, Request, StatusCode},
};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::sync::Arc;
use stormchaser_api::{app, AppState};
use stormchaser_model::OpaClient;
use tower::ServiceExt;

use axum::extract::connect_info::ConnectInfo;
use std::net::SocketAddr;

#[tokio::test]
async fn test_rate_limiting() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "5");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "10");

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url)
        .await
        .expect("Failed to connect to NATS");
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD").unwrap_or_else(|_| "stormchaser".to_string())
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
    });

    let mut rng = rand::thread_rng();
    let ip_last_octet = rand::Rng::gen_range(&mut rng, 10..250);
    let addr = SocketAddr::from(([127, 0, 0, ip_last_octet], 12345));

    // We configured: per_second(5), burst_size(10)
    // So 11th request should be rate limited if sent immediately.

    for i in 1..=10 {
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

        assert_ne!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "Request {} should not be rate limited",
            i
        );
    }

    let response = app
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

    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "11th request should be rate limited"
    );
}
