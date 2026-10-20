//! A built-in step runs in the engine process that dispatched it. When that process
//! dies the step will never finish, and the engine fails it, as it does a step whose
//! runner has gone, instead of leaving it to the run's timeout; or, for a step with no
//! effect but its output, runs it again.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_engine::engine_instance::ENGINE_ID;
use stormchaser_engine::handler;
use stormchaser_engine::workers::fail_lost_engine_steps;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::step::StepStatus;
use stormchaser_model::{RunId, StepInstanceId};
use stormchaser_opa::OpaClient;
use uuid::Uuid;

use stormchaser_engine::engine_instance::LOST_AFTER;

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

/// Starts a run of one JinjaRender step and returns it, dispatched, with the engine
/// that runs it moved to one of the test's own, whose heartbeats the test controls. No
/// outbox relay runs here: the step's own result stays queued, as if its engine had
/// died before the result reached the stream.
async fn step_on_test_engine(pool: &PgPool, run_id: RunId) -> (StepInstanceId, Uuid) {
    let nats_client = nats().await;
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
    handler::handle_workflow_start_pending(run_id, pool.clone(), nats_client, tls_reloader().await)
        .await
        .unwrap();

    let (step_id, engine_id): (StepInstanceId, Option<Uuid>) =
        sqlx::query_as("SELECT id, engine_id FROM step_instances WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(engine_id, Some(*ENGINE_ID), "the step records its engine");

    // The step runs in this process. Its result in the outbox says it is done, and will
    // not write the step again under the changes the test makes.
    let mut done = false;
    for _ in 0..50 {
        done = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM outbox_events WHERE subject LIKE '%.step.completed' AND payload LIKE '%' || $1 || '%')",
        )
        .bind(step_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap();
        if done {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(done, "the step did not finish");

    let engine = Uuid::new_v4();
    heartbeat_at(pool, engine, 0.0).await;
    sqlx::query("UPDATE step_instances SET engine_id = $2 WHERE id = $1")
        .bind(step_id)
        .bind(engine)
        .execute(pool)
        .await
        .unwrap();
    (step_id, engine)
}

async fn delete_engine(pool: &PgPool, engine: Uuid) {
    sqlx::query("DELETE FROM engine_instances WHERE id = $1")
        .bind(engine)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_built_in_step_whose_engine_died_fails_and_ends_its_run() {
    let pool = setup_db().await;
    let run_id = RunId::new_v4();
    let (step_id, engine) = step_on_test_engine(&pool, run_id).await;
    // A step with effects beyond its output, which must not run twice
    sqlx::query("UPDATE step_instances SET step_type = 'WebhookInvoke' WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();

    // While its engine sends heartbeats the step is left alone
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();
    assert!(queued_failures(&pool, step_id).await.is_empty());

    // Its engine stops
    heartbeat_at(&pool, engine, 2.0 * LOST_AFTER.as_secs_f64()).await;
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();
    let failures = queued_failures(&pool, step_id).await;
    assert_eq!(failures.len(), 1, "the step is failed once");

    handler::handle_step_failed(
        failures[0].clone(),
        pool.clone(),
        nats().await,
        tls_reloader().await,
    )
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

    delete_run(&pool, run_id).await;
    delete_engine(&pool, engine).await;
}

#[tokio::test]
async fn a_side_effect_free_step_whose_engine_died_runs_again() {
    let pool = setup_db().await;
    let run_id = RunId::new_v4();
    let (step_id, engine) = step_on_test_engine(&pool, run_id).await;

    heartbeat_at(&pool, engine, 2.0 * LOST_AFTER.as_secs_f64()).await;
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();

    assert!(queued_failures(&pool, step_id).await.is_empty());
    let (status, engine_id): (StepStatus, Option<Uuid>) =
        sqlx::query_as("SELECT status, engine_id FROM step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, StepStatus::Pending, "put back for dispatch");
    assert_eq!(engine_id, None);

    delete_run(&pool, run_id).await;
    delete_engine(&pool, engine).await;
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
    handler::handle_workflow_start_pending(run_id, pool.clone(), nats_client, tls_reloader().await)
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

    delete_run(&pool, run_id).await;
}

/// A side-effect-free step that becomes lost after the sweep has put back the ones it
/// found is left for the next sweep, not failed: the sweep's second query sees rows
/// other transactions committed after its first.
#[tokio::test]
async fn a_side_effect_free_step_lost_between_the_sweeps_queries_is_not_failed() {
    let pool = setup_db().await;
    let run_id = RunId::new_v4();
    let (step_id, engine) = step_on_test_engine(&pool, run_id).await;
    heartbeat_at(&pool, engine, 2.0 * LOST_AFTER.as_secs_f64()).await;

    // The second query alone, as if the step became lost after the first ran
    let mut tx = pool.begin().await.unwrap();
    let taken =
        stormchaser_engine::db::take_lost_engine_steps(&mut tx, LOST_AFTER, &["JQ", "JinjaRender"])
            .await
            .unwrap();
    tx.commit().await.unwrap();
    assert!(
        taken.iter().all(|step| step.id != step_id),
        "a JinjaRender step was taken to be failed"
    );

    // The next sweep runs it again
    fail_lost_engine_steps(&pool, LOST_AFTER).await.unwrap();
    assert!(queued_failures(&pool, step_id).await.is_empty());
    let status: StepStatus = sqlx::query_scalar("SELECT status FROM step_instances WHERE id = $1")
        .bind(step_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, StepStatus::Pending);

    delete_run(&pool, run_id).await;
    delete_engine(&pool, engine).await;
}
