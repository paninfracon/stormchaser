//! A step the engine runs itself finishes the way a runner's step does: its result goes
//! through the step-completed handler, which records it and moves the run on. A step that
//! records its own result first leaves the handler nothing to do, and its run never ends.

mod common;

use common::{delete_run, nats, setup_db, start_relay, tls_reloader};
use futures::StreamExt;
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use tokio::time::sleep;

/// Starts a run of `dsl`, feeds each step result the engine publishes back through the
/// handlers, as the engine's consumers would, and returns the run's final status.
async fn run_to_end(dsl: &str) -> RunStatus {
    let pool = setup_db().await;
    let nats_client = nats().await;
    // Built-in steps report their results through the outbox, which the engine's
    // relay publishes
    let _relay = start_relay(pool.clone(), nats_client.clone());
    let tls_reloader = tls_reloader().await;
    let run_id = RunId::new_v4();

    let mut results = nats_client
        .subscribe("stormchaser.v1.*.step.>")
        .await
        .unwrap();

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

    let deadline = sleep(Duration::from_secs(15));
    tokio::pin!(deadline);
    loop {
        if let Some(status) = final_status(&pool, run_id).await {
            delete_run(&pool, run_id).await;
            return status;
        }
        tokio::select! {
            Some(msg) = results.next() => {
                let Ok(ce) = serde_json::from_slice::<cloudevents::Event>(&msg.payload) else { continue };
                let Some(cloudevents::Data::Json(payload)) = ce.data() else { continue };
                if payload["run_id"].as_str() != Some(&run_id.to_string()) {
                    continue;
                }
                if msg.subject.ends_with(".step.completed") {
                    handler::handle_step_completed(
                        serde_json::from_value(payload.clone()).unwrap(),
                        pool.clone(),
                        nats_client.clone(),
                        Arc::new(None),
                        tls_reloader.clone(),
                    )
                    .await
                    .unwrap();
                } else if msg.subject.ends_with(".step.failed") {
                    handler::handle_step_failed(
                        serde_json::from_value(payload.clone()).unwrap(),
                        pool.clone(),
                        nats_client.clone(),
                        tls_reloader.clone(),
                    )
                    .await
                    .unwrap();
                }
            }
            _ = &mut deadline => {
                let step_status: Option<String> = sqlx::query_scalar(
                    "SELECT status::text || COALESCE(': ' || error, '') FROM step_instances WHERE run_id = $1 LIMIT 1",
                )
                .bind(run_id)
                .fetch_optional(&pool)
                .await
                .unwrap();
                panic!("run {run_id} never finished; its step is {step_status:?}");
            }
        }
    }
}

async fn final_status(pool: &PgPool, run_id: RunId) -> Option<RunStatus> {
    sqlx::query_scalar(
        "SELECT status FROM archived_workflow_runs WHERE id = $1 AND status IN ('succeeded', 'failed', 'aborted')",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn test_jinja_render_step_ends_its_run() {
    let status = run_to_end(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "jinja-completion" {
            steps {
                step "render" "JinjaRender" {
                    spec = {
                        template = "hello {{ name }}"
                        context = { name = "world" }
                    }
                }
            }
        }
        "#,
    )
    .await;
    assert_eq!(status, RunStatus::Succeeded);
}

#[tokio::test]
async fn test_webhook_invoke_step_ends_its_run() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .mount(&server)
        .await;

    let status = run_to_end(&format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "webhook-completion" {{
            steps {{
                step "notify" "WebhookInvoke" {{
                    spec = {{
                        url = "{}/hook"
                    }}
                }}
            }}
        }}
        "#,
        server.uri()
    ))
    .await;
    assert_eq!(status, RunStatus::Succeeded);
}

#[tokio::test]
async fn test_failing_webhook_invoke_step_fails_its_run() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let status = run_to_end(&format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "webhook-failure" {{
            steps {{
                step "notify" "WebhookInvoke" {{
                    spec = {{
                        url = "{}/hook"
                    }}
                }}
            }}
        }}
        "#,
        server.uri()
    ))
    .await;
    assert_eq!(status, RunStatus::Failed);
}
