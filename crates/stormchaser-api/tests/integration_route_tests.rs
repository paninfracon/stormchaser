use axum::extract::connect_info::ConnectInfo;
use axum::http::header::AUTHORIZATION;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;

use std::env::var;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Once;
use stormchaser_api::{app, AppState, Claims, JWT_SECRET};
use stormchaser_model::auth::OpaClient;

use stormchaser_model::RunId;
use tower::ServiceExt;
use uuid::Uuid;

static INIT: Once = Once::new();

fn init_test() {
    INIT.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("Failed to install default crypto provider");
    });
}

async fn setup_app_with_pool() -> Option<(axum::Router, sqlx::PgPool)> {
    init_test();
    std::env::set_var("CRON_ENGINE", "none");
    std::env::set_var("API_RATE_LIMIT_PER_SECOND", "1000");
    std::env::set_var("API_RATE_LIMIT_BURST_SIZE", "1000");

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.ok()?;

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .ok()?;

    Some((
        app(AppState {
            pool: pool.clone(),
            nats: nats_client,
            opa: Arc::new(OpaClient::new(None, None)),
            oidc_config: None,
            jwks: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            log_backend: None,
            api_base_url: "http://localhost:3000".to_string(),
        }),
        pool,
    ))
}

async fn setup_app() -> Option<axum::Router> {
    setup_app_with_pool().await.map(|(app, _)| app)
}

fn get_token() -> String {
    let claims = Claims {
        sub: "test-user".to_string(),
        email: Some("test-user@paninfracon.net".to_string()),
        exp: (Utc::now() + chrono::Duration::hours(1)).timestamp() as usize,
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
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_list_connections() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/connections")
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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

    let name = format!("test-webhook-{}", Uuid::new_v4());
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/webhooks")
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let conn_id = Uuid::new_v4();
    let conn_name = format!("test-conn-{}", conn_id);
    sqlx::query(
        "INSERT INTO connections (id, name, connection_type, config) VALUES ($1, $2, 'git', $3)",
    )
    .bind(conn_id)
    .bind(&conn_name)
    .bind(serde_json::json!({"repo_url": "http://github.com/test"}))
    .execute(&pool)
    .await
    .unwrap();

    let name = format!("test-cron-{}", Uuid::new_v4());
    let workflow_name = format!("test-workflow-{}", Uuid::new_v4());
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/cron-workflows")
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": name,
                        "cronspec": "0 0 * * *",
                        "workflow_name": workflow_name,
                        "connection": conn_name,
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

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let webhook_id = Uuid::new_v4();
    let webhook_name = format!("test-hook-{}", webhook_id);
    sqlx::query("INSERT INTO webhooks (id, name, source_type) VALUES ($1, $2, 'generic')")
        .bind(webhook_id)
        .bind(webhook_name)
        .execute(&pool)
        .await
        .unwrap();

    let conn_id = Uuid::new_v4();
    let conn_name = format!("test-conn-{}", conn_id);
    sqlx::query(
        "INSERT INTO connections (id, name, connection_type, config) VALUES ($1, $2, 'git', $3)",
    )
    .bind(conn_id)
    .bind(&conn_name)
    .bind(serde_json::json!({"repo_url": "url"}))
    .execute(&pool)
    .await
    .unwrap();

    let rule_name = format!("test-rule-{}", Uuid::new_v4());
    let workflow_name = format!("test-workflow-{}", Uuid::new_v4());
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rules")
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "name": rule_name,
                        "webhook_id": webhook_id,
                        "event_type_pattern": ".*",
                        "workflow_name": workflow_name,
                        "connection": conn_name,
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

    let run_id = Uuid::new_v4();
    let workflow_name = format!("test-workflow-{}", run_id);
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@127.0.0.1:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, $2, 'user', 'url', 'path', 'ref', 'running'::run_status, 1)")
        .bind(run_id)
        .bind(&workflow_name)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs) VALUES ($1, '1.0', '{}', '', '{}')")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/runs/{}/status/stream", run_id))
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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

    use futures::StreamExt;
    let mut stream = response.into_body().into_data_stream();

    let timeout = tokio::time::sleep(std::time::Duration::from_secs(5));
    tokio::pin!(timeout);

    let found = loop {
        tokio::select! {
            chunk_opt = stream.next() => {
                let chunk = chunk_opt.expect("Stream closed").unwrap();
                let text = String::from_utf8_lossy(&chunk);
                println!("RECEIVED SSE CHUNK: {}", text);
                if text.contains("run_status") && text.contains("running") {
                    break true;
                }
            }
            _ = &mut timeout => {
                break false;
            }
        }
    };
    assert!(
        found,
        "Did not receive the initial SSE run_status event via DB polling"
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

    let id = Uuid::new_v4();
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
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
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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

    let id = Uuid::new_v4();
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
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
                .header(AUTHORIZATION, "Bearer bad-token")
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
                .header(AUTHORIZATION, format!("Bearer {}", secret))
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

    let run_id = Uuid::new_v4();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/runs/{}/logs/stream", run_id))
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let expected_status = if var("LOKI_URL").is_ok() {
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

    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
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
                    "/api/v1/runs/{}/steps/{}/logs/stream",
                    run_id, step_id
                ))
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                .extension(ConnectInfo(addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let expected_status = if var("LOKI_URL").is_ok() {
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
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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

    let run_id = Uuid::new_v4();
    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/runs/{}", run_id))
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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

    let run_id = Uuid::new_v4();
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
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
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
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
async fn test_run_from_git() {
    let (app, pool) = match setup_app_with_pool().await {
        Some(a) => a,
        None => return,
    };

    let conn_id = uuid::Uuid::new_v4();
    let conn_name = format!("test-git-conn-{}", conn_id);
    sqlx::query("INSERT INTO connections (id, name, connection_type, config) VALUES ($1, $2, 'git', '{\"repo_url\": \"https://github.com/paninfracon/stormchaser\"}')")
        .bind(conn_id)
        .bind(conn_name)
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/runs")
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                .header("Content-Type", stormchaser_model::APPLICATION_JSON)
                .extension(ConnectInfo(addr))
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "workflow_name": "hello-world",
                        "connection": conn_id.to_string(),
                        "workflow_path": "tests/hello-world.storm",
                        "git_ref": "trunk",
                        "inputs": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let enqueue_resp: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let run_id = enqueue_resp["run_id"].as_str().unwrap();

    // Verify git file system cache only has the `.storm` file.
    let temp_dir = tempfile::tempdir().unwrap();
    let git_cache = stormchaser_engine::git_cache::GitCache::new(temp_dir.path());
    let repo_url = "https://github.com/paninfracon/stormchaser";
    let git_ref = "trunk";
    let workflow_path = "tests/hello-world.storm";

    let path = git_cache
        .ensure_files(repo_url, git_ref, &[workflow_path.to_string()])
        .unwrap();

    let mut storm_files = Vec::new();
    for entry in walkdir::WalkDir::new(&path) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                if ext == "storm" {
                    storm_files.push(entry.path().to_path_buf());
                }
            }
        }
    }

    assert_eq!(
        storm_files.len(),
        1,
        "Expected exactly 1 .storm file in git cache"
    );
    assert_eq!(
        storm_files[0].file_name().unwrap().to_str().unwrap(),
        "hello-world.storm"
    );

    // Mock engine execution so the workflow completes successfully.
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let opa_client = std::sync::Arc::new(OpaClient::new(None, None));
    let tls_reloader = std::sync::Arc::new(
        stormchaser_tls::TlsReloader::new(stormchaser_tls::TlsConfig::default())
            .await
            .unwrap(),
    );

    // Advance Queued -> StartPending
    if let Err(e) = stormchaser_engine::handler::workflow::handle_workflow_queued(
        uuid::Uuid::parse_str(run_id).map(RunId::new).unwrap(),
        serde_json::json!({}),
        pool.clone(),
        std::sync::Arc::new(git_cache),
        opa_client,
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    {
        if !e.to_string().contains("Optimistic concurrency") {
            panic!("handle_workflow_queued failed: {:?}", e);
        }
    }

    // Advance StartPending -> Running
    if let Err(e) = stormchaser_engine::handler::workflow::handle_workflow_start_pending(
        uuid::Uuid::parse_str(run_id).map(RunId::new).unwrap(),
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    {
        if !e.to_string().contains("Optimistic concurrency") {
            panic!("handle_workflow_start_pending failed: {}", e);
        }
    }

    // Verify the workflow completes successfully.
    let mut success = false;
    for _ in 0..60 {
        let fencing_token: Option<i64> =
            sqlx::query_scalar("SELECT fencing_token FROM workflow_runs WHERE id = $1")
                .bind(uuid::Uuid::parse_str(run_id).unwrap())
                .fetch_optional(&pool)
                .await
                .unwrap();

        let fencing_token = fencing_token.unwrap_or_default();

        let steps: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
            "SELECT id, step_name, status::text FROM step_instances WHERE run_id = $1",
        )
        .bind(uuid::Uuid::parse_str(run_id).unwrap())
        .fetch_all(&pool)
        .await
        .unwrap();
        println!("ITERATION: Steps in DB: {:?}", steps);

        if let Ok(step_id) = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM step_instances WHERE run_id = $1 AND status NOT IN ('succeeded', 'failed', 'failed_ignored', 'skipped', 'aborted', 'lost_zombie'::step_status) LIMIT 1")
            .bind(uuid::Uuid::parse_str(run_id).unwrap())
            .fetch_one(&pool)
            .await
        {
            println!("Mocking handle_step_completed for step_id: {}", step_id);
            // Mock runner completing the step
            stormchaser_engine::handler::step::events::handle_step_completed(
                serde_json::from_value(serde_json::json!({
                    "run_id": run_id,
                    "step_id": step_id.to_string(),
                    "fencing_token": fencing_token,
                    "event_type": "StepCompletedEvent",
                    "timestamp": Utc::now(),
                    "outputs": {}
                }))
                .unwrap(),
                pool.clone(),
                nats_client.clone(),
                std::sync::Arc::new(None),
                tls_reloader.clone(),
            )
            .await
            .expect("handle_step_completed failed");
            println!("handle_step_completed succeeded for step_id: {}", step_id);
        }

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/api/v1/runs/{}", run_id))
                    .header(AUTHORIZATION, format!("Bearer {}", get_token()))
                    .extension(ConnectInfo(addr))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let run_detail: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        let status = run_detail["detail"]["status"].as_str().unwrap();
        if status == "succeeded" {
            success = true;
            break;
        } else if status == "failed" || status == "aborted" {
            panic!("Workflow failed or aborted: {:?}", run_detail);
        }

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    assert!(
        success,
        "Workflow did not complete successfully within timeout"
    );
}

#[tokio::test]
async fn test_stream_workflow_runs() {
    let app = match setup_app().await {
        Some(a) => a,
        None => return,
    };

    let run_id = Uuid::new_v4();
    let workflow_name = format!("test-workflow-{}", run_id);
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@127.0.0.1:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token) VALUES ($1, $2, 'user', 'url', 'path', 'ref', 'running'::run_status, 1)")
        .bind(run_id)
        .bind(&workflow_name)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs) VALUES ($1, '1.0', '{}', '', '{}')")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let addr = SocketAddr::from(([127, 0, 0, 1], 12345));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/runs/stream")
                .header(AUTHORIZATION, format!("Bearer {}", get_token()))
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

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let nats_client = async_nats::connect(&nats_url).await.unwrap();
    let js = async_nats::jetstream::new(nats_client.clone());

    // Sleep briefly to ensure the server's SSE task has time to subscribe to NATS
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    use stormchaser_model::events::{
        EventSource, EventType, SchemaVersion, WorkflowCompletedEvent, WorkflowEventType,
    };
    use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
    let completion_event = WorkflowCompletedEvent {
        run_id: stormchaser_model::RunId::new(run_id),
        event_type: EventType::Workflow(WorkflowEventType::Completed),
        timestamp: chrono::Utc::now(),
        status: stormchaser_model::workflow::RunStatus::Succeeded,
    };

    publish_cloudevent(
        &js,
        NatsSubject::RunCompleted(Some(stormchaser_model::nats::compute_shard_id(
            &stormchaser_model::RunId::new(run_id),
        ))),
        EventType::Workflow(WorkflowEventType::Completed),
        EventSource::System,
        serde_json::to_value(completion_event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
    .unwrap();

    use futures::StreamExt;
    let mut stream = response.into_body().into_data_stream();

    let timeout = tokio::time::sleep(std::time::Duration::from_secs(5));
    tokio::pin!(timeout);

    let found = loop {
        tokio::select! {
            chunk_opt = stream.next() => {
                let chunk = chunk_opt.expect("Stream closed").unwrap();
                let text = String::from_utf8_lossy(&chunk);
                if text.contains(&run_id.to_string()) {
                    break true;
                }
            }
            _ = &mut timeout => {
                break false;
            }
        }
    };
    assert!(found, "Did not receive the SSE event via NATS routing");

    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}
