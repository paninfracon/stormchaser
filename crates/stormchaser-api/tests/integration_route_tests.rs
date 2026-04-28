use axum::extract::connect_info::ConnectInfo;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Once;
use stormchaser_api::{app, AppState, Claims, JWT_SECRET};
use stormchaser_model::auth::OpaClient;
use tower::ServiceExt;

use stormchaser_model::LogBackend;

static INIT: Once = Once::new();

fn init_test() {
    INIT.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("Failed to install default crypto provider");
    });
}

async fn setup_app() -> Option<axum::Router> {
    init_test();
    std::env::set_var("CRON_ENGINE", "none");
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.ok()?;

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = PgPoolOptions::new().connect(&db_url).await.ok()?;

    Some(app(AppState {
        pool,
        nats: nats_client,
        opa: Arc::new(OpaClient::new(None, None)),
        oidc_config: None,
        jwks: std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        log_backend: std::env::var("LOKI_URL")
            .ok()
            .map(|url| LogBackend::Loki { url }),
    }))
}

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
async fn test_list_webhooks() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/webhooks")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_list_event_rules() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/rules")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_list_cron_workflows() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/cron-workflows")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_list_storage_backends() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/storage-backends")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_create_webhook() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let name = format!("test-webhook-{}", uuid::Uuid::new_v4());
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/webhooks")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": name,
                        "source_type": "generic"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_create_cron_workflow() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let name = format!("test-cron-{}", uuid::Uuid::new_v4());
    let workflow_name = format!("test-workflow-{}", uuid::Uuid::new_v4());
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/cron-workflows")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": name,
                        "cronspec": "0 0 * * *",
                        "workflow_name": workflow_name,
                        "repo_url": "http://github.com/test",
                        "workflow_path": "test.storm",
                        "git_ref": "main",
                        "inputs": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_create_event_rule() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let webhook_id = uuid::Uuid::new_v4();
    let webhook_name = format!("test-hook-{}", webhook_id);
    sqlx::query("INSERT INTO webhooks (id, name, source_type) VALUES ($1, $2, 'generic')")
        .bind(webhook_id)
        .bind(webhook_name)
        .execute(&pool)
        .await
        .unwrap();

    let rule_name = format!("test-rule-{}", uuid::Uuid::new_v4());
    let workflow_name = format!("test-workflow-{}", uuid::Uuid::new_v4());
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rules")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": rule_name,
                        "webhook_id": webhook_id,
                        "event_type_pattern": ".*",
                        "workflow_name": workflow_name,
                        "repo_url": "url",
                        "workflow_path": "path",
                        "git_ref": "ref",
                        "input_mappings": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_stream_run_status() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let run_id = uuid::Uuid::new_v4();
    let workflow_name = format!("test-workflow-{}", run_id);
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, $2, 'user', 'url', 'path', 'ref', 'running'::run_status, 1)")
        .bind(run_id)
        .bind(&workflow_name)
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/runs/{}/status/stream", run_id))
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/event-stream"
    );

    // Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_delete_cron_workflow() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let id = uuid::Uuid::new_v4();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    sqlx::query("INSERT INTO cron_workflows (id, name, description, cronspec, workflow_name, repo_url, workflow_path, git_ref, inputs, secret_token, external_job_id) VALUES ($1, $2, '', '0 0 * * *', 'wf', 'repo', 'path', 'main', '{}', 'secret', 'ext_id')")
        .bind(id)
        .bind(format!("test-cron-{}", id))
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/cron-workflows/{}", id))
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Cleanup
    let _ = sqlx::query("DELETE FROM cron_workflows WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn test_trigger_cron_workflow() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let id = uuid::Uuid::new_v4();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let secret = "my-secret-token";

    sqlx::query("INSERT INTO cron_workflows (id, name, description, cronspec, workflow_name, repo_url, workflow_path, git_ref, inputs, secret_token, external_job_id) VALUES ($1, $2, '', '0 0 * * *', 'wf', 'repo', 'path', 'main', '{}', $3, 'ext_id')")
        .bind(id)
        .bind(format!("test-cron-{}", id))
        .bind(secret)
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));

    // Test unauthorized
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/cron-trigger/{}", id))
                .header("Authorization", "Bearer bad-token")
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Test success
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/cron-trigger/{}", id))
                .header("Authorization", format!("Bearer {}", secret))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Cleanup
    let _ = sqlx::query("DELETE FROM cron_workflows WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM workflow_runs WHERE workflow_name = 'wf'")
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn test_stream_run_logs() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let run_id = uuid::Uuid::new_v4();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/runs/{}/logs/stream", run_id))
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let expected_status = if std::env::var("LOKI_URL").is_ok() {
        StatusCode::OK
    } else {
        StatusCode::NOT_IMPLEMENTED
    };

    assert_eq!(response.status(), expected_status);
    if expected_status == StatusCode::OK {
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
    }
}

#[tokio::test]
async fn test_stream_step_logs() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let run_id = uuid::Uuid::new_v4();
    let step_id = uuid::Uuid::new_v4();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, 'wf', 'u', 'r', 'p', 'g', 'running'::run_status, 1)")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type) VALUES ($1, $2, 'test-step', 'running'::step_status, 'Wasm')")
        .bind(step_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/runs/{}/steps/test-step/logs/stream",
                    run_id
                ))
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let expected_status = if std::env::var("LOKI_URL").is_ok() {
        StatusCode::OK
    } else {
        StatusCode::NOT_IMPLEMENTED
    };

    assert_eq!(response.status(), expected_status);
    if expected_status == StatusCode::OK {
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
    }

    // Cleanup
    let _ = sqlx::query("DELETE FROM step_instances WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await;
    let _ = sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn test_list_workflow_runs() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/runs")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_get_workflow_run_not_found() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let run_id = uuid::Uuid::new_v4();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/runs/{}", run_id))
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_workflow_run() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let run_id = uuid::Uuid::new_v4();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or("postgres://stormchaser:stormchaser@localhost:5432/stormchaser".into());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, 'wf', 'u', 'r', 'p', 'g', 'running'::run_status, 1)")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/runs/{}", run_id))
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_direct_run() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/runs/direct")
                .header("Authorization", format!("Bearer {}", get_token()))
                .header("Content-Type", "application/json")
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "dsl": "stormchaser_dsl_version = \"v1\"\nworkflow \"wf\" {}",
                        "inputs": {},
                        "secrets": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_stream_workflow_runs() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/runs/stream")
                .header("Authorization", format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
}
