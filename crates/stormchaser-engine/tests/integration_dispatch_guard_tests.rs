//! A step of a built-in type that no runner runs is failed with the reason when it is
//! dispatched, instead of being published for a runner that never takes it.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use serde_json::json;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::RunId;
use stormchaser_opa::OpaClient;

async fn start_run(pool: &sqlx::PgPool, nats_client: &async_nats::Client, step: &str) -> RunId {
    let run_id = RunId::new_v4();
    let payload = json!({
        "run_id": run_id,
        "dsl": format!(r#"
            stormchaser_dsl_version = "1.0"
            workflow "test-dispatch-guard-{run_id}" {{
                steps {{
                    {step}
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

/// The error of the step failure queued for the run, if one was
async fn queued_failure(pool: &sqlx::PgPool, run_id: RunId) -> Option<String> {
    let payload: Option<String> = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE subject LIKE '%.step.failed' AND payload LIKE '%' || $1 || '%'",
    )
    .bind(run_id.to_string())
    .fetch_optional(pool)
    .await
    .unwrap();
    let event: serde_json::Value = serde_json::from_str(&payload?).unwrap();
    event["data"]["error"]
        .as_str()
        .or_else(|| event["error"].as_str())
        .map(str::to_string)
}

#[tokio::test]
async fn a_step_type_nothing_runs_yet_fails_with_the_reason() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(
        &pool,
        &nats_client,
        r#"step "group" "Parallel" {
            steps {
                step "inner" "RunContainer" {
                    image = "alpine"
                }
            }
        }"#,
    )
    .await;

    let error = queued_failure(&pool, run_id)
        .await
        .expect("the step failed");
    assert!(
        error.contains("Parallel steps are not supported yet"),
        "{error}"
    );

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_git_checkout_the_engine_cannot_rewrite_fails_with_the_reason() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let run_id = start_run(&pool, &nats_client, r#"step "checkout" "GitCheckout" {}"#).await;

    let error = queued_failure(&pool, run_id)
        .await
        .expect("the step failed");
    assert!(error.contains("GitCheckout step needs a spec"), "{error}");

    delete_run(&pool, run_id).await;
}
