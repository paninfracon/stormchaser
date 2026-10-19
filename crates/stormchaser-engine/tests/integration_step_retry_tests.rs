//! A step with a `retry` policy that fails is put back to pending and run again once
//! its delay is over, until it succeeds or its retries run out.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use serde_json::json;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_engine::workers::dispatch_due_retries;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::events::{StepCompletedEvent, StepFailedEvent};
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::{RunId, StepInstanceId};

const RUNNER: &str = "test-step-retry-runner";

/// Starts a run of one step, "flaky", that is retried twice, a minute apart
async fn start_run(pool: &sqlx::PgPool, nats_client: &async_nats::Client) -> RunId {
    let run_id = RunId::new_v4();
    let payload = json!({
        "run_id": run_id,
        "dsl": format!(r#"
            stormchaser_dsl_version = "1.0"
            workflow "test-step-retry-{run_id}" {{
                steps {{
                    step "flaky" "RunContainer" {{
                        image = "alpine"
                        command = ["false"]
                        retry {{
                            count = 2
                            delay = "1m"
                        }}
                    }}
                }}
            }}
        "#),
        "inputs": {},
        "initiating_user": "test-user"
    });
    handler::handle_workflow_direct(
        payload,
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
        tls_reloader().await,
    )
    .await
    .unwrap();
    run_id
}

/// The step's id, once a runner has it
async fn running_step(pool: &sqlx::PgPool, run_id: RunId) -> StepInstanceId {
    sqlx::query_scalar(
        "UPDATE step_instances SET status = 'running', runner_id = $2 WHERE run_id = $1 AND status = 'initializing' RETURNING id",
    )
    .bind(run_id)
    .bind(RUNNER)
    .fetch_one(pool)
    .await
    .expect("the step has been dispatched")
}

async fn fail(
    pool: &sqlx::PgPool,
    nats_client: &async_nats::Client,
    run_id: RunId,
    step_id: StepInstanceId,
) {
    let mut event = StepFailedEvent::new(run_id, step_id, 0, "exit status 1".to_string());
    event.runner_id = Some(RUNNER.to_string());
    handler::handle_step_failed(
        event,
        pool.clone(),
        nats_client.clone(),
        tls_reloader().await,
    )
    .await
    .unwrap();
}

/// Ends the step's wait for its retry now, and lets the retry worker dispatch it
async fn retry_now(pool: &sqlx::PgPool, nats_client: &async_nats::Client, step_id: StepInstanceId) {
    sqlx::query("UPDATE step_instances SET retry_at = NOW() WHERE id = $1")
        .bind(step_id)
        .execute(pool)
        .await
        .unwrap();
    dispatch_due_retries(pool, nats_client, &tls_reloader().await)
        .await
        .unwrap();
}

async fn run_status(pool: &sqlx::PgPool, run_id: RunId) -> RunStatus {
    sqlx::query_scalar("SELECT status FROM workflow_runs WHERE id = $1 UNION ALL SELECT status FROM archived_workflow_runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The step's status, attempt and whether it is waiting for a retry
async fn step_state(pool: &sqlx::PgPool, step_id: StepInstanceId) -> (StepStatus, i32, bool) {
    sqlx::query_as(
        "SELECT status, attempt, retry_at IS NOT NULL FROM step_instances WHERE id = $1 UNION ALL SELECT status, attempt, false FROM archived_step_instances WHERE id = $1",
    )
    .bind(step_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_failed_step_waits_for_its_delay_and_runs_again() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client).await;

    let flaky = running_step(&pool, run_id).await;
    fail(&pool, &nats_client, run_id, flaky).await;

    assert_eq!(run_status(&pool, run_id).await, RunStatus::Running);
    assert_eq!(
        step_state(&pool, flaky).await,
        (StepStatus::Pending, 2, true)
    );

    // Not before its delay is over
    handler::dispatch_pending_steps(
        run_id,
        pool.clone(),
        nats_client.clone(),
        tls_reloader().await,
    )
    .await
    .unwrap();
    assert_eq!(step_state(&pool, flaky).await.0, StepStatus::Pending);

    retry_now(&pool, &nats_client, flaky).await;
    assert_eq!(
        step_state(&pool, flaky).await,
        (StepStatus::Initializing, 2, false)
    );

    // The second attempt succeeds, and so does the run
    let again = running_step(&pool, run_id).await;
    assert_eq!(again, flaky, "the same step runs again");
    let mut event = StepCompletedEvent::new(run_id, flaky, 0);
    event.runner_id = Some(RUNNER.to_string());
    handler::handle_step_completed(
        event,
        pool.clone(),
        nats_client.clone(),
        Arc::new(None),
        tls_reloader().await,
    )
    .await
    .unwrap();

    assert_eq!(run_status(&pool, run_id).await, RunStatus::Succeeded);
    assert_eq!(step_state(&pool, flaky).await.0, StepStatus::Succeeded);

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_step_that_runs_out_of_retries_fails_its_run() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client).await;

    let flaky = running_step(&pool, run_id).await;
    for _ in 0..2 {
        fail(&pool, &nats_client, run_id, flaky).await;
        retry_now(&pool, &nats_client, flaky).await;
        running_step(&pool, run_id).await;
    }
    fail(&pool, &nats_client, run_id, flaky).await;

    assert_eq!(run_status(&pool, run_id).await, RunStatus::Failed);
    assert_eq!(
        step_state(&pool, flaky).await,
        (StepStatus::Failed, 3, false)
    );

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_failure_delivered_again_does_not_use_up_a_retry() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client).await;

    let flaky = running_step(&pool, run_id).await;
    fail(&pool, &nats_client, run_id, flaky).await;
    fail(&pool, &nats_client, run_id, flaky).await;

    assert_eq!(
        step_state(&pool, flaky).await,
        (StepStatus::Pending, 2, true)
    );
    assert_eq!(run_status(&pool, run_id).await, RunStatus::Running);

    delete_run(&pool, run_id).await;
}
