use axum::extract::connect_info::ConnectInfo;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;

use std::net::SocketAddr;
use std::sync::Arc;
use stormchaser_api::{app, AppState};
use stormchaser_model::OpaClient;
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn test_webhook_trigger() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD").unwrap_or_else(|_| "stormchaser".to_string())
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),

        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    };

    let app = app(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    // 1. Create a Webhook
    let webhook_id = Uuid::new_v4();
    sqlx::query("INSERT INTO webhooks (id, name, source_type, is_active) VALUES ($1, $2, $3, $4)")
        .bind(webhook_id)
        .bind(format!("test-webhook-{}", webhook_id))
        .bind("generic")
        .bind(true)
        .execute(&pool)
        .await
        .unwrap();

    // 2. Create an Event Rule
    let rule_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO event_rules (
            id, name, webhook_id, event_type_pattern, condition_expr,
            workflow_name, repo_url, workflow_path, input_mappings
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(rule_id)
    .bind(format!("test-rule-{}", rule_id))
    .bind(webhook_id)
    .bind("generic")
    .bind("event.action == \"trigger\"")
    .bind(format!("test-workflow-{}", rule_id))
    .bind("https://github.com/org/repo")
    .bind("workflow.storm")
    .bind(json!({ "msg": "event.message" }))
    .execute(&pool)
    .await
    .unwrap();

    // 3. Send Webhook Request
    let payload = json!({
        "action": "trigger",
        "message": "Hello Stormchaser"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/webhooks/{}", webhook_id))
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let res_json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res_json["triggered_rules"], 1);

    // 4. Verify Workflow Run was created
    let run: (String, Value) = sqlx::query_as(
        "SELECT workflow_name, inputs FROM workflow_runs wr JOIN run_contexts rc ON wr.id = rc.run_id WHERE wr.workflow_name = $1"
    )
    .bind(format!("test-workflow-{}", rule_id))
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(run.0, format!("test-workflow-{}", rule_id));
    assert_eq!(run.1["msg"], "Hello Stormchaser");

    // 5. Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE workflow_name = $1")
        .bind(format!("test-workflow-{}", rule_id))
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("DELETE FROM webhooks WHERE id = $1")
        .bind(webhook_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_github_webhook_signature() {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD").unwrap_or_else(|_| "stormchaser".to_string())
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),

        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
    };

    let app = app(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    let webhook_id = Uuid::new_v4();
    let secret = "super-secret";
    sqlx::query(
        "INSERT INTO webhooks (id, name, source_type, secret_token, is_active) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(webhook_id)
    .bind(format!("github-webhook-{}", webhook_id))
    .bind("github")
    .bind(secret)
    .bind(true)
    .execute(&pool)
    .await
    .unwrap();

    let payload = json!({
        "action": "opened",
        "pull_request": { "number": 123 }
    });
    let body_bytes = serde_json::to_vec(&payload).unwrap();

    // Calculate HMAC
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(&body_bytes);
    let result = mac.finalize();
    let signature = hex::encode(result.into_bytes());

    // 1. Valid Signature
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/webhooks/{}", webhook_id))
                .header("Content-Type", "application/json")
                .header("X-GitHub-Event", "pull_request")
                .header("X-Hub-Signature-256", format!("sha256={}", signature))
                .extension(ConnectInfo(addr))
                .body(Body::from(body_bytes.clone()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 2. Invalid Signature
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/webhooks/{}", webhook_id))
                .header("Content-Type", "application/json")
                .header("X-GitHub-Event", "pull_request")
                .header("X-Hub-Signature-256", "sha256=invalid")
                .extension(ConnectInfo(addr))
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
