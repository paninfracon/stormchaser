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

/// Inserts a running run with one engine-owned Approval step and returns their ids.
async fn insert_approval_step(pool: &PgPool) -> (RunId, StepInstanceId) {
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_name, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, 'running', 1, 'http://git.local', 'workflow.storm', 'main')"
    )
    .bind(run_id)
    .bind("test_completion_token_workflow")
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
        "approve_release",
        "Approval",
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

    (run_id, step_id)
}

fn rejection(
    run_id: RunId,
    step_id: StepInstanceId,
    completion_token: Option<String>,
) -> StepFailedEvent {
    StepFailedEvent {
        run_id,
        step_id,
        fencing_token: 1,
        completion_token,
        event_type: EventType::Step(StepEventType::Failed),
        error: "rejected".to_string(),
        runner_id: None,
        exit_code: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: None,
        timestamp: Utc::now(),
    }
}

async fn live_status(pool: &PgPool, step_id: StepInstanceId) -> Option<StepStatus> {
    sqlx::query_scalar("SELECT status FROM step_instances WHERE id = $1")
        .bind(step_id)
        .fetch_optional(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn engine_owned_step_needs_its_completion_token() {
    let pool = mock_pool().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let tls_reloader = Arc::new(
        TlsReloader::new(stormchaser_tls::TlsConfig::default())
            .await
            .unwrap(),
    );
    let (run_id, step_id) = insert_approval_step(&pool).await;

    // A new step is given a token the engine and API can read from the database.
    let token = stormchaser_engine::db::step_completion_token(&pool, step_id)
        .await
        .expect("a new step instance should have a completion token");

    // What a runner credential could publish: no token, or a guess. Neither finishes the step.
    for forged in [None, Some("not-the-token".to_string())] {
        handler::step::events::handle_step_failed(
            rejection(run_id, step_id, forged),
            pool.clone(),
            nats_client.clone(),
            tls_reloader.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            live_status(&pool, step_id).await,
            Some(StepStatus::Running),
            "a forged event must leave the step untouched"
        );
    }

    // The engine or API, which hold the token, can.
    handler::step::events::handle_step_failed(
        rejection(run_id, step_id, Some(token)),
        pool.clone(),
        nats_client,
        tls_reloader,
    )
    .await
    .unwrap();

    let archived: StepStatus =
        sqlx::query_scalar("SELECT status FROM archived_step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(archived, StepStatus::Failed);
}

#[tokio::test]
async fn runner_owned_step_is_reported_without_a_token() {
    let pool = mock_pool().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let tls_reloader = Arc::new(
        TlsReloader::new(stormchaser_tls::TlsConfig::default())
            .await
            .unwrap(),
    );
    let (run_id, step_id) = insert_approval_step(&pool).await;

    // Dispatching a step to a runner drops its token.
    stormchaser_engine::db::clear_step_completion_token(&pool, step_id)
        .await
        .unwrap();
    assert!(
        stormchaser_engine::db::step_completion_token(&pool, step_id)
            .await
            .is_none()
    );

    handler::step::events::handle_step_failed(
        rejection(run_id, step_id, None),
        pool.clone(),
        nats_client,
        tls_reloader,
    )
    .await
    .unwrap();

    assert_eq!(
        live_status(&pool, step_id).await,
        None,
        "the run should have finished and been archived"
    );
}
