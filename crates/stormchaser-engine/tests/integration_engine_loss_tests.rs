//! A built-in step runs in the engine process that dispatched it. When that process
//! dies the step will never finish, and the engine fails it, as it does a step whose
//! runner has gone, instead of leaving it to the run's timeout.

use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env::var;
use std::sync::Arc;
use stormchaser_engine::engine_instance::ENGINE_ID;
use stormchaser_engine::handler;
use stormchaser_engine::workers::fail_lost_engine_steps;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::step::StepStatus;
use stormchaser_model::{RunId, StepInstanceId};
use stormchaser_tls::{TlsConfig, TlsReloader};
use uuid::Uuid;

use stormchaser_engine::engine_instance::LOST_AFTER;

async fn setup_db() -> PgPool {
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

async fn nats() -> async_nats::Client {
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    stormchaser_model::nats::connect(nats_url).await.unwrap()
}

/// Records a heartbeat for `engine_id` made `seconds_ago`
async fn heartbeat_at(pool: &PgPool, engine_id: Uuid, seconds_ago: f64) {
    sqlx::query(
        "INSERT INTO engine_instances (id, last_heartbeat_at) VALUES ($1, NOW() - make_interval(secs => $2))
         ON CONFLICT (id) DO UPDATE SET last_heartbeat_at = EXCLUDED.last_heartbeat_at",
    )
    .bind(engine_id)
    .bind(seconds_ago)
    .execute(pool)
    .await
    .unwrap();
}

/// The step-failed events queued in the outbox for `step_id`
async fn queued_failures(pool: &PgPool, step_id: StepInstanceId) -> Vec<StepFailedEvent> {
    let payloads: Vec<String> = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE subject LIKE '%.step.failed' AND payload LIKE '%' || $1 || '%'",
    )
    .bind(step_id.to_string())
    .fetch_all(pool)
    .await
    .unwrap();
    payloads
        .iter()
        .map(|payload| {
            let event: serde_json::Value = serde_json::from_str(payload).unwrap();
            serde_json::from_value(event["data"].clone()).unwrap()
        })
        .collect()
}

async fn delete_queued_events(pool: &PgPool, run_id: RunId) {
    sqlx::query("DELETE FROM outbox_events WHERE payload LIKE '%' || $1 || '%'")
        .bind(run_id.to_string())
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_built_in_step_whose_engine_died_fails_and_ends_its_run() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
    let run_id = RunId::new_v4();

    // No outbox relay runs here: the step's own result stays queued, as if its engine
    // had died before the result reached the stream
    handler::handle_workflow_direct(
        json!({
            "run_id": run_id,
            "dsl": r#"
                stormchaser_dsl_version = "v1"
                workflow "engine-loss" {
                    steps {
                        step "render" "JinjaRender" {
                            spec = { template = "hello" }
                        }
                    }
                }
            "#,
            "inputs": {},
            "initiating_user": "test"
        }),
        pool.clone(),
        Arc::new(OpaClient::new(None, None)),
        nats_client.clone(),
    )
    .await
    .unwrap();
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();

    let (step_id, engine_id): (StepInstanceId, Option<Uuid>) =
        sqlx::query_as("SELECT id, engine_id FROM step_instances WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(engine_id, Some(*ENGINE_ID), "the step records its engine");

    // While its engine sends heartbeats the step is left alone
    heartbeat_at(&pool, *ENGINE_ID, 0.0).await;
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();
    assert!(queued_failures(&pool, step_id).await.is_empty());

    // Its engine stops
    heartbeat_at(&pool, *ENGINE_ID, 2.0 * LOST_AFTER.as_secs_f64()).await;
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();
    let failures = queued_failures(&pool, step_id).await;
    assert_eq!(failures.len(), 1, "the step is failed once");

    handler::handle_step_failed(failures[0].clone(), pool.clone(), nats_client, tls_reloader)
        .await
        .unwrap();

    let (status, run_status): (StepStatus, String) = sqlx::query_as(
        "SELECT s.status, r.status::text FROM archived_step_instances s JOIN archived_workflow_runs r ON r.id = s.run_id WHERE s.id = $1",
    )
    .bind(step_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, StepStatus::Failed);
    assert_eq!(run_status, "failed");

    delete_queued_events(&pool, run_id).await;
    sqlx::query("DELETE FROM engine_instances WHERE id = $1")
        .bind(*ENGINE_ID)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_engine_long_gone_is_forgotten_once_it_runs_no_step() {
    let pool = setup_db().await;
    let gone = Uuid::new_v4();
    // Twice as long as an engine is kept
    heartbeat_at(&pool, gone, 2.0 * 86400.0).await;

    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();

    let kept: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM engine_instances WHERE id = $1)")
            .bind(gone)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!kept);
}

#[tokio::test]
async fn a_runner_step_never_records_an_engine() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = RunId::new_v4();

    handler::handle_workflow_direct(
        json!({
            "run_id": run_id,
            "dsl": r#"
                stormchaser_dsl_version = "v1"
                workflow "engine-loss-runner" {
                    steps {
                        step "hello" "RunContainer" {
                            image = "alpine"
                            command = ["echo", "hello"]
                        }
                    }
                }
            "#,
            "inputs": {},
            "initiating_user": "test"
        }),
        pool.clone(),
        Arc::new(OpaClient::new(None, None)),
        nats_client.clone(),
    )
    .await
    .unwrap();
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client,
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    // Were it to, its loss would fail it as a built-in step's, not retry its dispatch
    let (dispatched, engine_id): (bool, Option<Uuid>) = sqlx::query_as(
        "SELECT dispatched_at IS NOT NULL, engine_id FROM step_instances WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(dispatched);
    assert_eq!(engine_id, None);

    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}
