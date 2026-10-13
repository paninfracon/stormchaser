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
        None,
        json!({}),
        json!({}),
        Utc::now(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    // The step is running on a runner, which is when the engine drops its completion token.
    stormchaser_engine::db::clear_step_completion_token(&pool, step_id)
        .await
        .unwrap();

    // 2. Setup mock nats and tls
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let tls_reloader = Arc::new(
        TlsReloader::new(stormchaser_tls::TlsConfig::default())
            .await
            .unwrap(),
    );

    // 3. Dispatch zombie failure event
    let event = StepFailedEvent {
        run_id,
        step_id,
        fencing_token: 1, // Matches the inserted run's fencing_token
        completion_token: None,
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

/// Inserts a runner whose last heartbeat was `seconds_ago`.
async fn insert_runner(pool: &PgPool, id: &str, seconds_ago: i32) {
    sqlx::query(
        "INSERT INTO runners (id, runner_type, protocol_version, nats_subject, last_heartbeat_at) VALUES ($1, 'docker', 'v1', $2, NOW() - make_interval(secs => $3))",
    )
    .bind(id)
    .bind(format!("stormchaser.v1.runner.docker.{id}"))
    .bind(seconds_ago as f64)
    .execute(pool)
    .await
    .unwrap();
}

async fn is_zombie(pool: &PgPool, step_id: StepInstanceId) -> bool {
    // Rolled back: the query locks the rows it returns
    let mut tx = pool.begin().await.unwrap();
    let zombies = stormchaser_engine::db::steps::fetch_zombie_steps(&mut *tx)
        .await
        .unwrap();
    zombies.iter().any(|zombie| zombie.id == step_id)
}

#[tokio::test]
async fn a_step_adopted_by_another_runner_is_no_longer_lost_with_the_old_one() {
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

    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();
    let old_runner = format!("old-{}", uuid::Uuid::new_v4());
    let new_runner = format!("new-{}", uuid::Uuid::new_v4());
    insert_runner(&pool, &old_runner, 120).await;
    insert_runner(&pool, &new_runner, 0).await;

    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_name, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'adoption-test', 'running', 1, 'http://git.local', 'workflow.storm', 'main')"
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type, runner_id, started_at) VALUES ($1, $2, 'long', 'running', 'RunContainer', $3, NOW())")
        .bind(step_id)
        .bind(run_id)
        .bind(&old_runner)
        .execute(&pool)
        .await
        .unwrap();

    // The runner that started the step has stopped heartbeating
    assert!(is_zombie(&pool, step_id).await);

    // Another runner adopts the step's container and says so
    handler::step::events::handle_step_running(
        stormchaser_model::events::StepRunningEvent {
            run_id,
            step_id,
            event_type: EventType::Step(StepEventType::Running),
            runner_id: Some(new_runner.clone()),
            timestamp: Utc::now(),
        },
        pool.clone(),
    )
    .await
    .unwrap();

    let (status, assigned): (StepStatus, Option<String>) =
        sqlx::query_as("SELECT status, runner_id FROM step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, StepStatus::Running);
    assert_eq!(assigned.as_deref(), Some(new_runner.as_str()));
    assert!(!is_zombie(&pool, step_id).await);

    // A finished step does not move
    sqlx::query("UPDATE step_instances SET status = 'succeeded' WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();
    handler::step::events::handle_step_running(
        stormchaser_model::events::StepRunningEvent {
            run_id,
            step_id,
            event_type: EventType::Step(StepEventType::Running),
            runner_id: Some(old_runner.clone()),
            timestamp: Utc::now(),
        },
        pool.clone(),
    )
    .await
    .unwrap();
    let assigned: Option<String> =
        sqlx::query_scalar("SELECT runner_id FROM step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(assigned.as_deref(), Some(new_runner.as_str()));

    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM runners WHERE id = ANY($1)")
        .bind(vec![old_runner, new_runner])
        .execute(&pool)
        .await
        .unwrap();
}
