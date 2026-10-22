//! A run whose starting event the engine gives up on is recorded as failed, with why,
//! instead of staying queued or, for a direct run, never existing.

mod common;

use common::{delete_run, setup_db};
use serde_json::json;
use stormchaser_engine::handler;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;

async fn insert_run(pool: &sqlx::PgPool, run_id: RunId, status: &str) {
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'test-abandon', 'test', $2::run_status, 1, 'https://invalid.example/repo.git', 'test.storm', 'main')")
        .bind(run_id)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
}

/// The run's status and error, wherever it is now
async fn outcome(pool: &sqlx::PgPool, run_id: RunId) -> Option<(RunStatus, Option<String>)> {
    sqlx::query_as(
        "SELECT status, error FROM workflow_runs WHERE id = $1 UNION ALL SELECT status, error FROM archived_workflow_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_queued_run_given_up_on_is_failed_with_why() {
    let pool = setup_db().await;
    let run_id = RunId::new_v4();
    insert_run(&pool, run_id, "queued").await;

    handler::abandon_run_event(
        "stormchaser.v1.run.queued",
        &json!({ "run_id": run_id }),
        &pool,
        "connection refused",
    )
    .await;

    let (status, error) = outcome(&pool, run_id).await.unwrap();
    assert_eq!(status, RunStatus::Failed);
    assert_eq!(
        error.as_deref(),
        Some("The engine gave up starting this run: connection refused")
    );

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_run_that_has_started_is_left_alone() {
    let pool = setup_db().await;
    let run_id = RunId::new_v4();
    insert_run(&pool, run_id, "running").await;

    handler::abandon_run_event(
        "stormchaser.v1.run.start_pending",
        &json!({ "run_id": run_id }),
        &pool,
        "connection refused",
    )
    .await;

    assert_eq!(
        outcome(&pool, run_id).await,
        Some((RunStatus::Running, None))
    );

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_direct_run_given_up_on_is_recorded_as_failed() {
    let pool = setup_db().await;
    let run_id = RunId::new_v4();
    let payload = json!({
        "run_id": run_id,
        "dsl": "workflow \"never-parsed\" {}",
        "inputs": { "x": 1 },
        "initiating_user": "test-user",
    });

    handler::abandon_run_event("stormchaser.v1.run.direct", &payload, &pool, "timed out").await;

    let (status, error) = outcome(&pool, run_id).await.unwrap();
    assert_eq!(status, RunStatus::Failed);
    assert_eq!(
        error.as_deref(),
        Some("The engine gave up starting this run: timed out")
    );

    // Given up on again, it is not recorded twice
    handler::abandon_run_event("stormchaser.v1.run.direct", &payload, &pool, "timed out").await;

    delete_run(&pool, run_id).await;
}
