use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env::var;
use std::sync::Arc;
use stormchaser_api::{app, AppState};
use stormchaser_model::auth::OpaClient;
use tower::ServiceExt;
use uuid::Uuid;

async fn mock_state() -> AppState {
    // These tests send many requests in a burst; the limiter sits in front of
    // authentication and would otherwise answer 429 first.
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "100000");

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = stormchaser_model::nats::connect(nats_url).await.unwrap();

    AppState {
        pool,
        nats,
        // OPA is deliberately unconfigured: authentication must not depend on policy.
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    }
}

/// Every route in the protected group, including the ones whose handlers do not ask for
/// the caller's claims themselves.
fn protected_routes() -> Vec<(Method, String)> {
    let id = Uuid::new_v4();
    vec![
        (Method::GET, "/api/v1/runs".into()),
        (Method::POST, "/api/v1/runs".into()),
        (Method::GET, "/api/v1/runs/stream".into()),
        (Method::GET, format!("/api/v1/runs/{id}")),
        (Method::DELETE, format!("/api/v1/runs/{id}")),
        (
            Method::POST,
            format!("/api/v1/runs/{id}/steps/{id}/approve"),
        ),
        (Method::POST, format!("/api/v1/runs/{id}/steps/{id}/reject")),
        (Method::POST, "/api/v1/events/correlate".into()),
        (Method::GET, format!("/api/v1/runs/{id}/artifacts")),
        (Method::GET, format!("/api/v1/runs/{id}/reports")),
        (Method::GET, format!("/api/v1/runs/{id}/summaries")),
        (Method::GET, format!("/api/v1/runs/{id}/reports/{id}")),
        (
            Method::GET,
            format!("/api/v1/runs/{id}/steps/{id}/logs/stream"),
        ),
        (Method::GET, format!("/api/v1/runs/{id}/steps/{id}/logs")),
        (Method::GET, format!("/api/v1/runs/{id}/logs/stream")),
        (Method::GET, format!("/api/v1/runs/{id}/status/stream")),
        (Method::POST, "/api/v1/runs/direct".into()),
        (Method::GET, "/api/v1/webhooks".into()),
        (Method::POST, "/api/v1/webhooks".into()),
        (Method::GET, format!("/api/v1/webhooks/{id}")),
        (Method::PATCH, format!("/api/v1/webhooks/{id}")),
        (Method::DELETE, format!("/api/v1/webhooks/{id}")),
        (Method::GET, "/api/v1/cron-workflows".into()),
        (Method::POST, "/api/v1/cron-workflows".into()),
        (Method::DELETE, format!("/api/v1/cron-workflows/{id}")),
        (Method::GET, "/api/v1/rules".into()),
        (Method::POST, "/api/v1/rules".into()),
        (Method::DELETE, format!("/api/v1/rules/{id}")),
        (Method::GET, "/api/v1/connections".into()),
        (Method::POST, "/api/v1/connections".into()),
        (Method::GET, format!("/api/v1/connections/{id}")),
        (Method::PATCH, format!("/api/v1/connections/{id}")),
        (Method::DELETE, format!("/api/v1/connections/{id}")),
        (Method::POST, "/api/v1/connections/test".into()),
        (Method::POST, "/api/v1/schema/parse-git".into()),
    ]
}

async fn status_of(
    router: &axum::Router,
    method: &Method,
    path: &str,
    authorization: Option<&str>,
) -> StatusCode {
    let mut request = Request::builder()
        .method(method.clone())
        .uri(path)
        .header("content-type", "application/json");
    if let Some(value) = authorization {
        request = request.header("authorization", value);
    }
    router
        .clone()
        .oneshot(request.body(Body::from("{}")).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn protected_routes_reject_missing_and_invalid_tokens() {
    let router = app(mock_state().await);

    for (method, path) in protected_routes() {
        for authorization in [None, Some("Bearer x"), Some("Basic dXNlcjpwYXNz")] {
            assert_eq!(
                status_of(&router, &method, &path, authorization).await,
                StatusCode::UNAUTHORIZED,
                "{method} {path} with authorization {authorization:?}"
            );
        }
    }
}

#[tokio::test]
async fn public_routes_stay_reachable_without_a_token() {
    let router = app(mock_state().await);

    for path in ["/healthz", "/api/health", "/api/v1/schema"] {
        assert_eq!(
            status_of(&router, &Method::GET, path, None).await,
            StatusCode::OK,
            "{path}"
        );
    }
    // A webhook delivery is authenticated by its own secret, not a bearer token, so an
    // unknown id is "not found" rather than "unauthorized".
    let delivery = format!("/api/v1/webhooks/{}", Uuid::new_v4());
    assert_ne!(
        status_of(&router, &Method::POST, &delivery, None).await,
        StatusCode::UNAUTHORIZED
    );
}
