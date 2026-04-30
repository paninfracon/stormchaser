use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use sqlx::PgPool;
use stormchaser_engine::db;
use stormchaser_engine::handler::runner::*;
use stormchaser_model::runner::RunnerStatus;

async fn mock_pool() -> PgPool {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD").unwrap_or_else(|_| "stormchaser".to_string())
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

#[tokio::test]
async fn test_handle_runner_registration() {
    let pool = mock_pool().await;
    let runner_id = format!("test-runner-{}", Uuid::new_v4());
    let payload = json!({
        "runner_id": runner_id,
        "runner_type": "docker",
        "protocol_version": "1.0",
        "nats_subject": "test.subject",
        "capabilities": ["docker", "linux"],
        "step_types": [
            {
                "step_type": "shell",
                "schema": {},
                "documentation": "Run shell commands"
            }
        ]
    });

    handle_runner_registration(payload, pool.clone())
        .await
        .unwrap();

    let runner = crate::db::get_runner(&pool, &runner_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(runner.runner_type, "docker");
}

#[tokio::test]
async fn test_handle_runner_heartbeat() {
    let pool = mock_pool().await;
    let runner_id = format!("test-runner-{}", Uuid::new_v4());

    sqlx::query("INSERT INTO runners (id, runner_type, protocol_version, capabilities, nats_subject, status) VALUES ($1, 'test', '1.0', '{}', 'subj', 'online')")
            .bind(&runner_id).execute(&pool).await.unwrap();

    let payload = json!({ "runner_id": runner_id });
    handle_runner_heartbeat(payload, pool).await.unwrap();
}

#[tokio::test]
async fn test_handle_runner_offline() {
    let pool = mock_pool().await;
    let runner_id = format!("test-runner-{}", Uuid::new_v4());

    sqlx::query("INSERT INTO runners (id, runner_type, protocol_version, capabilities, nats_subject, status) VALUES ($1, 'test', '1.0', '{}', 'subj', 'online')")
            .bind(&runner_id).execute(&pool).await.unwrap();

    let payload = json!({ "runner_id": runner_id });
    handle_runner_offline(payload, pool.clone()).await.unwrap();

    let runner = crate::db::get_runner(&pool, &runner_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(runner.status, RunnerStatus::Offline);
}
