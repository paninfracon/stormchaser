use anyhow::Result;
use async_trait::async_trait;
use axum::{body::Body, http::Request, http::StatusCode, routing::get, Router};
use std::collections::HashMap;
use std::sync::Arc;
use stormchaser_api::auth::opa::opa_middleware;
use stormchaser_api::AppState;
use stormchaser_model::auth::ApprovalOpaContext;
use stormchaser_model::auth::{ApiOpaContext, OpaAuthorizer};
use tower::ServiceExt;

struct MockAuthorizer {
    configured: bool,
    result: Result<bool>,
}

#[async_trait]
impl OpaAuthorizer for MockAuthorizer {
    async fn check(&self, _context: ApiOpaContext<'_>) -> anyhow::Result<bool> {
        match &self.result {
            Ok(b) => Ok(*b),
            Err(_) => anyhow::bail!("error"),
        }
    }
    async fn check_approval(&self, _context: ApprovalOpaContext<'_>) -> anyhow::Result<bool> {
        match &self.result {
            Ok(b) => Ok(*b),
            Err(_) => anyhow::bail!("error"),
        }
    }
    fn is_configured(&self) -> bool {
        self.configured
    }
}

async fn mock_state(auth: MockAuthorizer) -> AppState {
    use sqlx::postgres::PgPoolOptions;
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = async_nats::connect(nats_url).await.unwrap();

    AppState {
        pool,
        nats,
        opa: Arc::new(auth),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
    }
}

#[tokio::test]
async fn test_opa_middleware_not_configured() {
    let state = mock_state(MockAuthorizer {
        configured: false,
        result: Ok(true),
    })
    .await;
    let app = Router::new().route("/", get(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(state.clone(), opa_middleware),
    );

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_opa_middleware_allowed() {
    let state = mock_state(MockAuthorizer {
        configured: true,
        result: Ok(true),
    })
    .await;

    let app = Router::new().route("/", get(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(state.clone(), opa_middleware),
    );

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_opa_middleware_denied() {
    let state = mock_state(MockAuthorizer {
        configured: true,
        result: Ok(false),
    })
    .await;

    let app = Router::new().route("/", get(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(state.clone(), opa_middleware),
    );

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_opa_middleware_error() {
    let state = mock_state(MockAuthorizer {
        configured: true,
        result: Err(anyhow::anyhow!("opa fail")),
    })
    .await;

    let app = Router::new().route("/", get(|| async { "ok" })).layer(
        axum::middleware::from_fn_with_state(state.clone(), opa_middleware),
    );

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
