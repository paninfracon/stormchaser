//! A step with a `retry` policy that fails is put back to pending and run again once
//! its delay is over, until it succeeds or its retries run out.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use serde_json::json;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_engine::workers::dispatch_due_retries;
use stormchaser_model::events::{
    EventType, StepCompletedEvent, StepEventType, StepFailedEvent, StepInitializingEvent,
    StepRunningEvent,
};
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::{RunId, StepInstanceId};
use stormchaser_opa::OpaClient;

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
    fail_attempt(pool, nats_client, run_id, step_id, None).await;
}

/// The runner's report that `attempt` failed (`None`: a runner that does not number
/// attempts)
async fn fail_attempt(
    pool: &sqlx::PgPool,
    nats_client: &async_nats::Client,
    run_id: RunId,
    step_id: StepInstanceId,
    attempt: Option<i32>,
) {
    let mut event = StepFailedEvent::new(run_id, step_id, 0, "exit status 1".to_string());
    event.runner_id = Some(RUNNER.to_string());
    event.attempt = attempt;
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

/// The runner's report that it has started `attempt`
async fn report_running(
    pool: &sqlx::PgPool,
    run_id: RunId,
    step_id: StepInstanceId,
    attempt: Option<i32>,
) {
    handler::handle_step_running(
        StepRunningEvent {
            run_id,
            step_id,
            attempt,
            event_type: EventType::Step(StepEventType::Running),
            runner_id: Some(RUNNER.to_string()),
            timestamp: chrono::Utc::now(),
        },
        pool.clone(),
    )
    .await
    .unwrap();
}

/// The runner the step is with
async fn step_runner(pool: &sqlx::PgPool, step_id: StepInstanceId) -> Option<String> {
    sqlx::query_scalar("SELECT runner_id FROM step_instances WHERE id = $1")
        .bind(step_id)
        .fetch_one(pool)
        .await
        .unwrap()
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

#[tokio::test]
async fn a_late_start_report_from_the_failed_attempt_leaves_the_retry_waiting() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client).await;

    let flaky = running_step(&pool, run_id).await;
    fail_attempt(&pool, &nats_client, run_id, flaky, Some(1)).await;

    // Its reports arrive on different subjects, so the first attempt's start can come
    // after its failure, from a runner that numbers attempts or one that does not
    for attempt in [Some(1), None] {
        report_running(&pool, run_id, flaky, attempt).await;
        handler::handle_step_initializing(
            StepInitializingEvent {
                run_id,
                step_id: flaky,
                attempt,
                event_type: EventType::Step(StepEventType::Initializing),
                runner_id: Some(RUNNER.to_string()),
                timestamp: chrono::Utc::now(),
            },
            pool.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            step_state(&pool, flaky).await,
            (StepStatus::Pending, 2, true),
            "attempt {attempt:?}'s late report took the step"
        );
        assert_eq!(step_runner(&pool, flaky).await, None);
    }

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_report_on_the_failed_attempt_leaves_the_next_one_alone() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client).await;

    let flaky = running_step(&pool, run_id).await;
    fail_attempt(&pool, &nats_client, run_id, flaky, Some(1)).await;
    retry_now(&pool, &nats_client, flaky).await;
    assert_eq!(
        step_state(&pool, flaky).await,
        (StepStatus::Initializing, 2, false)
    );

    // The first failure delivered again once the second attempt has been handed out,
    // and a completion of the first attempt, change nothing
    fail_attempt(&pool, &nats_client, run_id, flaky, Some(1)).await;
    let mut late = StepCompletedEvent::new(run_id, flaky, 0);
    late.runner_id = Some(RUNNER.to_string());
    late.attempt = Some(1);
    handler::handle_step_completed(
        late,
        pool.clone(),
        nats_client.clone(),
        Arc::new(None),
        tls_reloader().await,
    )
    .await
    .unwrap();
    assert_eq!(
        step_state(&pool, flaky).await,
        (StepStatus::Initializing, 2, false)
    );
    assert_eq!(run_status(&pool, run_id).await, RunStatus::Running);

    // The second attempt's own reports are taken
    report_running(&pool, run_id, flaky, Some(2)).await;
    assert_eq!(step_state(&pool, flaky).await.0, StepStatus::Running);
    fail_attempt(&pool, &nats_client, run_id, flaky, Some(2)).await;
    assert_eq!(
        step_state(&pool, flaky).await,
        (StepStatus::Pending, 3, true)
    );

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn the_runner_that_starts_a_claimed_step_is_recorded() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client).await;

    let flaky: StepInstanceId =
        sqlx::query_scalar("SELECT id FROM step_instances WHERE run_id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(step_state(&pool, flaky).await.0, StepStatus::Initializing);
    assert_eq!(step_runner(&pool, flaky).await, None);

    report_running(&pool, run_id, flaky, Some(1)).await;

    assert_eq!(step_state(&pool, flaky).await.0, StepStatus::Running);
    assert_eq!(
        step_runner(&pool, flaky).await.as_deref(),
        Some(RUNNER),
        "the liveness check and the result check go by the step's runner"
    );

    delete_run(&pool, run_id).await;
}
