use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env::var;
use std::sync::Arc;
use stormchaser_api::{app, AppState, EnqueueResponse};
use stormchaser_model::auth::{Claims, OpaClient};
use tower::ServiceExt;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    std::env::set_var("CRON_ENGINE", "none");

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

#[tokio::test]
async fn test_api_enqueue_inserts_quotas() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .try_init();
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
    };

    let app = app(state);

    // Generate token
    let claims = Claims {
        sub: "test-user".to_string(),
        email: Some("test-user@paninfracon.net".to_string()),
        exp: (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(b"stormchaser-secret-dev-only"),
    )
    .unwrap();

    let run_id_marker = Uuid::new_v4();

    let payload = json!({
        "workflow_name": format!("test-quota-api-{}", run_id_marker),
        "repo_url": "http://example.com",
        "workflow_path": "test.storm",
        "git_ref": "main",
        "inputs": {}
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/runs")
                .header(
                    axum::http::header::AUTHORIZATION,
                    format!("Bearer {}", token),
                )
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();

    if status != StatusCode::OK {
        panic!(
            "API request failed with status {}: {}",
            status,
            String::from_utf8_lossy(&body)
        );
    }
    let resp_data: EnqueueResponse = serde_json::from_slice(&body).unwrap();
    let run_id = resp_data.run_id;

    // Verify quotas were inserted
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_quotas WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        count, 1,
        "Default run quotas should be inserted by API enqueue"
    );

    // Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}
