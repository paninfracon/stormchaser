use chrono::Utc;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;

use sqlx::PgPool;
use stormchaser_engine::handler;
use stormchaser_model::events::{EventType, StepEventType, StepFailedEvent};
use stormchaser_model::step::StepStatus;
use stormchaser_model::{RunId, StepInstanceId};
use stormchaser_tls::TlsReloader;

async fn mock_pool() -> PgPool {
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
async fn test_zombie_step_handler() {
    let pool = mock_pool().await;
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

    // 1. Setup minimal run and step
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_name, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, 'running', 1, 'http://git.local', 'workflow.storm', 'main')"
    )
    .bind(run_id)
    .bind("test_zombie_workflow")
    .execute(&mut *tx)
    .await
    .unwrap();

    stormchaser_engine::db::runs::insert_run_context(
        &mut *tx,
        run_id,
        "1.0",
        json!({}),
        Some("code"),
        json!({}),
    )
    .await
    .unwrap();

    stormchaser_engine::db::steps::insert_step_instance_with_spec(
        &mut *tx,
        step_id,
        run_id,
        "zombie_step",
        "shell",
        StepStatus::Running,
        None,
        json!({}),
        json!({}),
        Utc::now(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // 2. Setup mock nats and tls
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let tls_reloader = Arc::new(
        TlsReloader::new(stormchaser_tls::TlsConfig::default())
            .await
            .unwrap(),
    );

    // 3. Dispatch zombie failure event
    let event = StepFailedEvent {
        run_id,
        step_id,
        event_type: EventType::Step(StepEventType::Failed),
        error: "lost_zombie".to_string(),
        runner_id: Some("offline-runner-id".to_string()),
        exit_code: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: None,
        timestamp: Utc::now(),
    };

    handler::step::events::handle_step_failed(event, pool.clone(), nats_client, tls_reloader)
        .await
        .unwrap();

    // 4. Verify step status
    let row: (StepStatus, Option<String>) =
        sqlx::query_as("SELECT status, error FROM archived_step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0, StepStatus::LostZombie);
    assert_eq!(row.1.unwrap(), "lost_zombie");
}
