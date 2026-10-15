//! A step the engine runs itself finishes the way a runner's step does: its result goes
//! through the step-completed handler, which records it and moves the run on. A step that
//! records its own result first leaves the handler nothing to do, and its run never ends.

use futures::StreamExt;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env::var;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::RunId;
use stormchaser_tls::{TlsConfig, TlsReloader};
use tokio::time::sleep;

async fn setup() -> (PgPool, async_nats::Client) {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    (pool, nats_client)
}

/// Starts a run of `dsl`, feeds each step result the engine publishes back through the
/// handlers, as the engine's consumers would, and returns the run's final status.
async fn run_to_end(dsl: &str) -> String {
    let (pool, nats_client) = setup().await;
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
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

async fn final_status(pool: &PgPool, run_id: RunId) -> Option<String> {
    sqlx::query_scalar(
        "SELECT status::text FROM archived_workflow_runs WHERE id = $1 AND status IN ('succeeded', 'failed', 'aborted')",
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
    assert_eq!(status, "succeeded");
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
    assert_eq!(status, "succeeded");
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
    assert_eq!(status, "failed");
}

/// A shared-affinity group's later steps go to the runner its first step landed on, on
/// the subject for that runner's kind; a container step can land on a Kubernetes runner.
#[tokio::test]
async fn test_shared_affinity_follows_a_kubernetes_runner() {
    use stormchaser_model::events::{EventType, StepCompletedEvent, StepEventType};
    use stormchaser_model::runner::RunnerStatus;

    let (pool, nats_client) = setup().await;
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
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

    let _ = sqlx::query("DELETE FROM runners WHERE id = $1")
        .bind(&runner_id)
        .execute(&pool)
        .await;
}
