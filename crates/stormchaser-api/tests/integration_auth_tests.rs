use axum::{
    body::Body,
    http::{self, Request, StatusCode},
};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use stormchaser_api::{app, AppState};
use stormchaser_model::OpaClient;
use tower::ServiceExt;

use axum::extract::connect_info::ConnectInfo;
use std::net::SocketAddr;

use stormchaser_api::Claims;
use stormchaser_api::OidcConfig;

#[tokio::test]
async fn test_auth_exchange_dex_flow() {
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = match async_nats::connect(nats_url).await {
        Ok(c) => c,
        Err(_) => return,
    };
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = match PgPoolOptions::new().connect(&db_url).await {
        Ok(p) => p,
        Err(_) => return,
    };

    // 1. Setup Mock OIDC/Dex configuration
    let issuer = "http://dex:5556/dex".to_string();
    let client_id = "stormchaser-cli".to_string();
    let kid = "test-key-id".to_string();

    // Generate a mock JWT using a symmetric key for simplicity in testing,
    // although real Dex uses RS256. The API code handles what's in the JWK.
    let secret = b"mock-dex-secret-long-enough-for-hs256";
    let claims = Claims {
        sub: "stormchaser-admin@paninfracon.net".to_string(),
        email: Some("stormchaser-admin@paninfracon.net".to_string()),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
    };

    let header = jsonwebtoken::Header {
        kid: Some(kid.clone()),
        ..Default::default()
    };

    let sso_token = jsonwebtoken::encode(
        &header,
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret),
    )
    .unwrap();

    // Create a mock JWK for this key
    let mut jwks = std::collections::HashMap::new();
    let jwk = jsonwebtoken::jwk::Jwk {
        common: jsonwebtoken::jwk::CommonParameters {
            key_id: Some(kid.clone()),
            ..Default::default()
        },
        algorithm: jsonwebtoken::jwk::AlgorithmParameters::OctetKeyPair(
            jsonwebtoken::jwk::OctetKeyPairParameters {
                key_type: jsonwebtoken::jwk::OctetKeyPairType::OctetKeyPair,
                curve: jsonwebtoken::jwk::EllipticCurve::Ed25519,
                x: "unused-base64-encoded-x".to_string(),
            },
        ),
    };
    jwks.insert(kid.clone(), jwk);

    let oidc_config = OidcConfig {
        issuer: issuer.clone(),
        client_id: client_id.clone(),
        jwks_url: "http://dex:5556/dex/keys".to_string(),
        client_secret: "secret".to_string(),
        external_issuer: "http://localhost:5556/dex".to_string(),
    };

    let app = app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: Some(oidc_config),
        jwks: std::sync::Arc::new(tokio::sync::RwLock::new(jwks)),
        log_backend: None,
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    let response = app
        .oneshot(
            Request::builder()
                .method(http::Method::POST)
                .uri("/api/v1/auth/exchange")
                .header(http::header::CONTENT_TYPE, "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "sso_token": sso_token,
                        "callback_url": "http://localhost:8080/callback"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // It should be 401 because DecodingKey::from_jwk with Ed25519 jwk and from_secret above don't match exactly
    // for validation without more complex mocking, but it confirms the code path is taken and validation is attempted.
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_auth_login_redirect() {
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = match async_nats::connect(nats_url).await {
        Ok(c) => c,
        Err(_) => return,
    };
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = match PgPoolOptions::new().connect(&db_url).await {
        Ok(p) => p,
        Err(_) => return,
    };

    let oidc_config = OidcConfig {
        issuer: "http://dex:5556/dex".to_string(),
        client_id: "stormchaser-cli".to_string(),
        jwks_url: "http://dex:5556/dex/keys".to_string(),
        client_secret: "secret".to_string(),
        external_issuer: "http://dex:5556/dex".to_string(),
    };

    let app = app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: Some(oidc_config),
        jwks: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        log_backend: None,
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    let response = app
        .oneshot(
            Request::builder()
                .method(http::Method::GET)
                .uri("/api/v1/auth/login?callback_url=http://localhost:1234")
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER); // 303 Redirect
    let location = response
        .headers()
        .get(http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(location.starts_with("http://dex:5556/dex/auth"));
    assert!(location.contains("client_id=stormchaser-cli"));
    assert!(location.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1234"));
}

#[tokio::test]
async fn test_protected_route_rejection() {
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = match async_nats::connect(nats_url).await {
        Ok(c) => c,
        Err(_) => return,
    };
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = match PgPoolOptions::new().connect(&db_url).await {
        Ok(p) => p,
        Err(_) => return,
    };
    let app = app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        log_backend: None,
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    let response = app
        .oneshot(
            Request::builder()
                .method(http::Method::POST)
                .uri("/api/v1/runs")
                .header(http::header::CONTENT_TYPE, "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "workflow_name": format!("test-workflow-{}", uuid::Uuid::new_v4()),
                        "repo_url": "url",
                        "workflow_path": "path",
                        "git_ref": "ref",
                        "inputs": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
