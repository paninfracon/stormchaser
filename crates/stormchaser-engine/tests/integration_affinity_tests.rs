//! A step in a shared-affinity group is dispatched to the runner its group's first step
//! landed on.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use futures::StreamExt;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::handler;
use stormchaser_model::RunId;
use stormchaser_opa::OpaClient;

/// A shared-affinity group's later steps go to the runner its first step landed on, on
/// the subject for that runner's kind; a container step can land on a Kubernetes runner.
#[tokio::test]
async fn test_shared_affinity_follows_a_kubernetes_runner() {
    use stormchaser_model::events::{EventType, StepCompletedEvent, StepEventType};
    use stormchaser_model::runner::RunnerStatus;

    let pool = setup_db().await;
    let nats_client = nats().await;
    let tls_reloader = tls_reloader().await;
    let run_id = RunId::new_v4();
    let runner_id = format!("k8s-affinity-{}", run_id);

    stormchaser_engine::db::upsert_runner(
        &pool,
        &runner_id,
        "k8s",
        RunnerStatus::Online,
        "v1",
        &[],
        &format!("stormchaser.v1.runner.k8s.{runner_id}"),
    )
    .await
    .unwrap();
    let mut pinned = nats_client
        .subscribe(format!("stormchaser.v1.runner.k8s.{runner_id}"))
        .await
        .unwrap();

    let dsl = r#"
        stormchaser_dsl_version = "v1"
        workflow "affinity-k8s" {
            steps {
                step "first" "RunContainer" {
                    strategy {
                        affinity = "shared"
                    }
                    spec {
                        image = "alpine:3"
                    }
                    next = ["second"]
                }
                step "second" "RunContainer" {
                    strategy {
                        affinity = "shared"
                    }
                    spec {
                        image = "alpine:3"
                    }
                }
            }
        }
    "#;
    handler::handle_workflow_direct(
        json!({ "run_id": run_id, "dsl": dsl, "inputs": {}, "initiating_user": "test" }),
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

    // The Kubernetes runner picks up the first step and finishes it.
    let first: stormchaser_model::StepInstanceId = sqlx::query_scalar(
        "UPDATE step_instances SET runner_id = $2, status = 'running' WHERE run_id = $1 AND step_name = 'first' RETURNING id",
    )
    .bind(run_id)
    .bind(&runner_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    handler::handle_step_completed(
        StepCompletedEvent {
            run_id,
            step_id: first,
            attempt: None,
            fencing_token: 0,
            completion_token: None,
            event_type: EventType::Step(StepEventType::Completed),
            runner_id: Some(runner_id.clone()),
            exit_code: Some(0),
            storage_hashes: None,
            artifacts: None,
            test_reports: None,
            outputs: None,
            timestamp: chrono::Utc::now(),
        },
        pool.clone(),
        nats_client.clone(),
        Arc::new(None),
        tls_reloader.clone(),
    )
    .await
    .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(10), pinned.next())
        .await
        .expect("the second step was not sent to the Kubernetes runner")
        .unwrap();
    let ce: cloudevents::Event = serde_json::from_slice(&msg.payload).unwrap();
    let Some(cloudevents::Data::Json(payload)) = ce.data() else {
        panic!("task is not JSON");
    };
    assert_eq!(payload["run_id"], json!(run_id));

    delete_run(&pool, run_id).await;
    sqlx::query("DELETE FROM runners WHERE id = $1")
        .bind(&runner_id)
        .execute(&pool)
        .await
        .unwrap();
}
