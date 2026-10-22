use axum::extract::connect_info::ConnectInfo;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::collections::HashMap;
use std::env::var;
use stormchaser_opa::OpaClient;

use std::net::SocketAddr;
use std::sync::Arc;
use stormchaser_api::{app, AppState};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn test_webhook_trigger() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),

        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    };

    let app = app(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    // 1. Create a Webhook
    let webhook_id = Uuid::new_v4();
    sqlx::query("INSERT INTO webhooks (id, name, source_type, secret_token, is_active) VALUES ($1, $2, $3, $4, $5)")
        .bind(webhook_id)
        .bind(format!("test-webhook-{}", webhook_id))
        .bind("generic")
        .bind("delivery-secret")
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
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .header("Authorization", "Bearer delivery-secret")
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
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),

        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
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
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
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
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
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

#[tokio::test]
async fn test_generic_webhook_secret() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();

    let state = AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),

        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    };

    let app = app(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    let webhook_id = Uuid::new_v4();
    let secret = "generic-secret";
    sqlx::query(
        "INSERT INTO webhooks (id, name, source_type, secret_token, is_active) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(webhook_id)
    .bind(format!("generic-webhook-{}", webhook_id))
    .bind("generic")
    .bind(secret)
    .bind(true)
    .execute(&pool)
    .await
    .unwrap();

    let payload = json!({
        "action": "trigger"
    });
    let body_bytes = serde_json::to_vec(&payload).unwrap();

    // 1. Valid Token
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/webhooks/{}", webhook_id))
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .header("Authorization", format!("Bearer {}", secret))
                .extension(ConnectInfo(addr))
                .body(Body::from(body_bytes.clone()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 2. Invalid Token
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/webhooks/{}", webhook_id))
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .header("Authorization", "Bearer invalid-secret")
                .extension(ConnectInfo(addr))
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_webhook_without_a_secret_accepts_no_deliveries() {
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let Ok(nats_client) = stormchaser_model::nats::connect(nats_url).await else {
        return;
    };
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let Ok(pool) = PgPoolOptions::new().connect(&db_url).await else {
        return;
    };
    let app = app(AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    });
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    // Rows like this exist from before a secret was required
    for source_type in ["generic", "github"] {
        let webhook_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO webhooks (id, name, source_type, is_active) VALUES ($1, $2, $3, true)",
        )
        .bind(webhook_id)
        .bind(format!("no-secret-{}", webhook_id))
        .bind(source_type)
        .execute(&pool)
        .await
        .unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/webhooks/{}", webhook_id))
                    .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                    .extension(ConnectInfo(addr))
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{source_type}");

        sqlx::query("DELETE FROM webhooks WHERE id = $1")
            .bind(webhook_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_delivery_is_acted_on_once_however_often_it_is_sent() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let Ok(nats_client) = stormchaser_model::nats::connect(nats_url).await else {
        return;
    };
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let Ok(pool) = PgPoolOptions::new().connect(&db_url).await else {
        return;
    };
    let app = app(AppState {
        pool: pool.clone(),
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        log_backend: None,
        api_base_url: "http://localhost:3000".to_string(),
        mcp_allowed_hosts: Vec::new(),
    });
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let secret = "replay-test-secret";

    // A webhook of each signed kind, each with a rule that fires on everything
    let mut webhooks = Vec::new();
    for source_type in ["github", "signed"] {
        let webhook_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO webhooks (id, name, source_type, secret_token, is_active) VALUES ($1, $2, $3, $4, true)",
        )
        .bind(webhook_id)
        .bind(format!("replay-{}-{}", source_type, webhook_id))
        .bind(source_type)
        .bind(secret)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO event_rules (
                id, name, webhook_id, event_type_pattern,
                workflow_name, repo_url, workflow_path, input_mappings
            ) VALUES ($1, $2, $3, '.*', $4, 'https://github.com/org/repo', 'workflow.storm', '{}')
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(format!("replay-rule-{}", webhook_id))
        .bind(webhook_id)
        .bind(format!("replay-workflow-{}", webhook_id))
        .execute(&pool)
        .await
        .unwrap();
        webhooks.push((source_type, webhook_id));
    }

    let deliver = |source_type: &'static str, webhook_id: Uuid, body: Vec<u8>, message_id: &str| {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/api/v1/webhooks/{}", webhook_id))
            .header("Content-Type", stormchaser_model::APPLICATION_JSON)
            .extension(ConnectInfo(addr));
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        if source_type == "github" {
            mac.update(&body);
            request = request
                .header("X-GitHub-Event", "push")
                // A replayer is free to change this header; it must make no difference
                .header("X-GitHub-Delivery", Uuid::new_v4().to_string())
                .header(
                    "X-Hub-Signature-256",
                    format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
                );
        } else {
            let timestamp = chrono::Utc::now().timestamp();
            mac.update(format!("{message_id}.{timestamp}.").as_bytes());
            mac.update(&body);
            request = request
                .header("webhook-id", message_id)
                .header("webhook-timestamp", timestamp.to_string())
                .header(
                    "webhook-signature",
                    format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes())),
                );
        }
        let app = app.clone();
        async move {
            let response = app
                .oneshot(request.body(Body::from(body)).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        }
    };

    for (source_type, webhook_id) in webhooks {
        let body = serde_json::to_vec(&json!({ "type": "deploy", "n": 1 })).unwrap();

        let first = deliver(source_type, webhook_id, body.clone(), "msg_1").await;
        assert_eq!(first["status"], "ok", "{source_type}");
        assert_eq!(first["triggered_rules"], 1, "{source_type}");

        let replay = deliver(source_type, webhook_id, body.clone(), "msg_1").await;
        assert_eq!(replay["status"], "duplicate", "{source_type}");
        assert_eq!(replay["triggered_rules"], 0, "{source_type}");

        // A different delivery is a different event
        let other = serde_json::to_vec(&json!({ "type": "deploy", "n": 2 })).unwrap();
        let second = deliver(source_type, webhook_id, other, "msg_2").await;
        assert_eq!(second["status"], "ok", "{source_type}");
        assert_eq!(second["triggered_rules"], 1, "{source_type}");

        // Once the record is older than the retention window the delivery counts again
        sqlx::query("UPDATE webhook_deliveries SET received_at = NOW() - INTERVAL '25 hours' WHERE webhook_id = $1")
            .bind(webhook_id)
            .execute(&pool)
            .await
            .unwrap();
        let later = deliver(source_type, webhook_id, body, "msg_1").await;
        assert_eq!(later["status"], "ok", "{source_type}");

        let runs: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM workflow_runs WHERE workflow_name = $1")
                .bind(format!("replay-workflow-{}", webhook_id))
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(runs, 3, "{source_type}: one run per distinct delivery");

        sqlx::query("DELETE FROM workflow_runs WHERE workflow_name = $1")
            .bind(format!("replay-workflow-{}", webhook_id))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM webhooks WHERE id = $1")
            .bind(webhook_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
