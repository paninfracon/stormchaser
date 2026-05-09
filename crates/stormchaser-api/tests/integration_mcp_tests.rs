#![cfg(feature = "mcp")]

use axum::extract::connect_info::ConnectInfo;
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use rustls::crypto::ring;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::str;
use std::sync::Arc;
use std::sync::Once;
use stormchaser_api::{app, AppState, Claims, JWT_SECRET};
use stormchaser_model::auth::OpaClient;
use tokio::sync::RwLock;
use tower::ServiceExt;

static INIT: Once = Once::new();

fn init_test() {
    INIT.call_once(|| {
        ring::default_provider()
            .install_default()
            .expect("Failed to install default crypto provider");
    });
}

async fn setup_app() -> Option<axum::Router> {
    init_test();
    env::set_var("CRON_ENGINE", "none");
    env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");

    let nats_url = env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.ok()?;

    let db_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .ok()?;

    Some(app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
    }))
}

fn get_token() -> String {
    let claims = Claims {
        sub: "test-user".to_string(),
        email: Some("test-user@paninfracon.net".to_string()),
        exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET),
    )
    .unwrap()
}

#[tokio::test]
async fn test_mcp_unauthenticated_post() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "1.0.0"
            }
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mcp")
                .header(header::HOST, "localhost")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream")
                .extension(ConnectInfo(addr))
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    println!("Response status: {}", status);
    println!("Response body: {:?}", str::from_utf8(&body_bytes).unwrap());

    // In this test environment, OPA is not configured so it allows the request.
    // We expect 200 OK because the mcp service handles the POST request.
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_mcp_authenticated_post_returns_session_response() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let request_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "test-client",
                "version": "1.0.0"
            }
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mcp")
                .header(header::HOST, "localhost")
                .header(header::AUTHORIZATION, format!("Bearer {}", get_token()))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream")
                .extension(ConnectInfo(addr))
                .body(Body::from(serde_json::to_vec(&request_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    println!("Response status: {}", status);
    println!("Response body: {:?}", str::from_utf8(&body_bytes).unwrap());

    // With a valid token, we expect the request to be accepted by the handler and return 200 OK
    assert_eq!(status, StatusCode::OK);
}
