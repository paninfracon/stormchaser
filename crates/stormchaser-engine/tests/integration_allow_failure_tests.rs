//! A step that sets `allow_failure` fails without failing its run: the steps after it
//! run, and the run can still succeed.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use serde_json::json;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::events::{StepCompletedEvent, StepFailedEvent};
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::{RunId, StepInstanceId};

const RUNNER: &str = "test-allow-failure-runner";

/// Starts a run whose first step, "flaky", sets `allow_failure` as given and is
/// followed by "after"
async fn start_run(pool: &sqlx::PgPool, nats_client: &async_nats::Client, allow: bool) -> RunId {
    let run_id = RunId::new_v4();
    let payload = json!({
        "run_id": run_id,
        "dsl": format!(r#"
            stormchaser_dsl_version = "1.0"
            workflow "test-allow-failure-{run_id}" {{
                steps {{
                    step "flaky" "RunContainer" {{
                        image = "alpine"
                        command = ["false"]
                        allow_failure = {allow}
                        next = ["after"]
                    }}
                    step "after" "RunContainer" {{
                        image = "alpine"
                        command = ["true"]
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
async fn running_step(pool: &sqlx::PgPool, run_id: RunId, name: &str) -> StepInstanceId {
    sqlx::query_scalar(
        "UPDATE step_instances SET status = 'running', runner_id = $3 WHERE run_id = $1 AND step_name = $2 RETURNING id",
    )
    .bind(run_id)
    .bind(name)
    .bind(RUNNER)
    .fetch_one(pool)
    .await
    .unwrap()
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

async fn run_status(pool: &sqlx::PgPool, run_id: RunId) -> RunStatus {
    sqlx::query_scalar("SELECT status FROM workflow_runs WHERE id = $1 UNION ALL SELECT status FROM archived_workflow_runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn step_statuses(pool: &sqlx::PgPool, run_id: RunId) -> Vec<(String, StepStatus)> {
    sqlx::query_as(
        "SELECT step_name, status FROM step_instances WHERE run_id = $1 UNION ALL SELECT step_name, status FROM archived_step_instances WHERE run_id = $1 ORDER BY step_name",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_step_allowed_to_fail_lets_its_run_go_on_and_succeed() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client, true).await;

    let flaky = running_step(&pool, run_id, "flaky").await;
    fail(&pool, &nats_client, run_id, flaky).await;

    assert_eq!(run_status(&pool, run_id).await, RunStatus::Running);
    let steps = step_statuses(&pool, run_id).await;
    assert_eq!(steps[0].0, "after", "the step after it is scheduled");
    assert_eq!(steps[1], ("flaky".to_string(), StepStatus::FailedIgnored));

    let after = running_step(&pool, run_id, "after").await;
    let mut event = StepCompletedEvent::new(run_id, after, 0);
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

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_step_not_allowed_to_fail_fails_its_run() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client, false).await;

    let flaky = running_step(&pool, run_id, "flaky").await;
    fail(&pool, &nats_client, run_id, flaky).await;

    assert_eq!(run_status(&pool, run_id).await, RunStatus::Failed);
    assert_eq!(
        step_statuses(&pool, run_id).await,
        [("flaky".to_string(), StepStatus::Failed)]
    );

    delete_run(&pool, run_id).await;
}
