use axum::extract::connect_info::ConnectInfo;
use axum::{
    body::Body,
    http::{self, Request, StatusCode},
};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use stormchaser_api::{app, AppState, Claims, JWT_SECRET};
use stormchaser_model::OpaClient;
use tower::ServiceExt;
use uuid::Uuid;

use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::RunStatus;

fn get_token() -> String {
    let claims = Claims {
        sub: "test-user".to_string(),
        email: Some("test-user@paninfracon.net".to_string()),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(JWT_SECRET),
    )
    .unwrap()
}

#[tokio::test]
async fn test_storage_backend_crud() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = match async_nats::connect(nats_url).await {
        Ok(c) => c,
        Err(_) => return,
    };
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = match PgPoolOptions::new().connect(&db_url).await {
        Ok(p) => p,
        Err(_) => return,
    };

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client.clone(),
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    };

    let app = app(state.clone());
    let token = get_token();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    // 1. Create
    let create_payload = json!({
        "name": "test-s3",
        "description": "Test S3 Backend",
        "backend_type": "s3",
        "config": {
            "bucket": "test-bucket",
            "endpoint": "http://localhost:9000"
        },
        "is_default_sfs": false
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(http::Method::POST)
                .uri("/api/v1/storage-backends")
                .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
                .header(http::header::CONTENT_TYPE, "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(serde_json::to_vec(&create_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // 2. List
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(http::Method::GET)
                .uri("/api/v1/storage-backends")
                .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let backends: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert!(backends.iter().any(|b| b["name"] == "test-s3"));

    let backend_id = backends.iter().find(|b| b["name"] == "test-s3").unwrap()["id"]
        .as_str()
        .unwrap();

    // 3. Update
    let update_payload = json!({
        "description": "Updated Description"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(http::Method::PATCH)
                .uri(format!("/api/v1/storage-backends/{}", backend_id))
                .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
                .header(http::header::CONTENT_TYPE, "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 4. Delete
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(http::Method::DELETE)
                .uri(format!("/api/v1/storage-backends/{}", backend_id))
                .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_artifact_listing() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = match async_nats::connect(nats_url).await {
        Ok(c) => c,
        Err(_) => return,
    };
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = match PgPoolOptions::new().connect(&db_url).await {
        Ok(p) => p,
        Err(_) => return,
    };

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client.clone(),
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    };

    let app = app(state.clone());
    let token = get_token();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    // 1. Insert a dummy run and artifact
    let run_id = Uuid::new_v4();
    let workflow_name = format!("test-workflow-{}", run_id);
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
        .bind(run_id)
        .bind(&workflow_name)
        .bind("test-user")
        .bind(RunStatus::Running)
        .bind(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0))
        .bind("http://example.com")
        .bind("test.storm")
        .bind("main")
        .execute(&pool)
        .await
        .unwrap();

    let backend_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO storage_backends (id, name, backend_type, config) VALUES ($1, $2, $3::backend_type, $4)",
    )
    .bind(backend_id)
    .bind(format!("test-backend-{}", backend_id))
    .bind("s3")
    .bind(json!({}))
    .execute(&pool)
    .await
    .unwrap();

    let step_id = Uuid::new_v4();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params) VALUES ($1, $2, $3, $4, $5, $6, $7)")
        .bind(step_id)
        .bind(run_id)
        .bind("test-step")
        .bind("RunContainer")
        .bind(StepStatus::Succeeded)
        .bind(json!({}))
        .bind(json!({}))
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO artifact_registry (run_id, step_instance_id, artifact_name, backend_id, remote_path) VALUES ($1, $2, $3, $4, $5)")
        .bind(run_id)
        .bind(step_id)
        .bind("test-artifact")
        .bind(backend_id)
        .bind("path/to/artifact")
        .execute(&pool)
        .await
        .unwrap();

    // 2. List artifacts for run
    let response = app
        .oneshot(
            Request::builder()
                .method(http::Method::GET)
                .uri(format!("/api/v1/runs/{}/artifacts", run_id))
                .header(http::header::AUTHORIZATION, format!("Bearer {}", token))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let artifacts: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["artifact_name"], "test-artifact");

    // 3. Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("DELETE FROM storage_backends WHERE id = $1")
        .bind(backend_id)
        .execute(&pool)
        .await
        .unwrap();
}
