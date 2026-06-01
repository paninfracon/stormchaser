use chrono::Utc;
use futures::StreamExt;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;

use stormchaser_engine::handler;
use stormchaser_engine::handler::runner::handle_runner_heartbeat;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::events::RunnerHeartbeatEvent;
use stormchaser_model::runner::RunnerStatus;
use stormchaser_model::step::StepInstance;
use stormchaser_model::RunId;
use stormchaser_tls::{TlsConfig, TlsReloader};

async fn setup() -> (
    sqlx::PgPool,
    async_nats::Client,
    Arc<TlsReloader>,
    Arc<OpaClient>,
) {
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
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));
    let tls_config = TlsConfig::default();
    let tls_reloader = Arc::new(TlsReloader::new(tls_config).await.unwrap());

    stormchaser_engine::workers::start_outbox_relay_worker(pool.clone(), nats_client.clone());

    (pool, nats_client, tls_reloader, opa_client)
}

#[tokio::test]
async fn test_step_lifecycle_emits_workflow_succeeded() {
    let (pool, nats_client, tls_reloader, opa_client) = setup().await;
    let run_id = RunId::new_v4();

    let mut subscriber = nats_client.subscribe("stormchaser.v1.>").await.unwrap();

    let dsl = r#"
        stormchaser_dsl_version = "v1"
        workflow "single-step" {
            steps {
                step "generate" "RunContainer" {
                    image = "alpine"
                }
            }
        }
    "#;

    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {},
        "initiating_user": "test"
    });

    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        opa_client.clone(),
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

    let instances: Vec<StepInstance> = sqlx::query_as(r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1"#)
        .bind(run_id)
        .fetch_all(&pool)
        .await
        .unwrap();

    assert_eq!(instances.len(), 1);
    let step_id = instances[0].id;

    let fencing_token: i64 =
        sqlx::query_scalar("SELECT fencing_token FROM workflow_runs WHERE id = $1")
            .bind(run_id)
            .fetch_one(&pool)
            .await
            .expect("workflow run fencing token should be queryable");

    let completed_payload = json!({
        "run_id": run_id,
        "step_id": step_id,
        "event_type": "StepCompletedEvent",
        "fencing_token": fencing_token,
        "timestamp": Utc::now(),
        "exit_code": 0
    });

    let log_backend = Arc::new(None);
    handler::handle_step_completed(
        serde_json::from_value(completed_payload).unwrap(),
        pool.clone(),
        nats_client.clone(),
        log_backend.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();

    // Verify WorkflowSucceeded NATS emission
    let mut workflow_succeeded_found = false;
    // Consume messages until we find the WorkflowSucceededEvent or timeout
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(msg) = subscriber.next().await {
            if msg.subject.to_string().contains("run.completed") {
                let response: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
                if response["data"]["run_id"].as_str() == Some(run_id.to_string().as_str()) {
                    workflow_succeeded_found = true;
                    break;
                }
            }
        }
    })
    .await;

    assert!(
        timeout.is_ok(),
        "Timed out waiting for WorkflowCompleted NATS emission"
    );
    assert!(
        workflow_succeeded_found,
        "WorkflowCompleted event was not found"
    );
}

#[tokio::test]
async fn test_workflow_timeout_emits_workflow_failed() {
    let (pool, nats_client, tls_reloader, opa_client) = setup().await;
    let run_id = RunId::new_v4();

    let mut subscriber = nats_client.subscribe("stormchaser.v1.>").await.unwrap();

    let dsl = r#"
        stormchaser_dsl_version = "v1"
        workflow "timeout-test" {
            steps {
                step "generate" "RunContainer" {
                    image = "alpine"
                }
            }
        }
    "#;

    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {},
        "initiating_user": "test"
    });

    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        opa_client.clone(),
        nats_client.clone(),
    )
    .await
    .unwrap();

    handler::handle_workflow_timeout(
        run_id,
        pool.clone(),
        nats_client.clone(),
        tls_reloader.clone(),
    )
    .await
    .unwrap();

    // Verify WorkflowAborted NATS emission
    let mut workflow_aborted_found = false;
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(msg) = subscriber.next().await {
            if msg.subject.to_string().contains("run.aborted") {
                let response: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
                if response["data"]["run_id"].as_str() == Some(run_id.to_string().as_str()) {
                    workflow_aborted_found = true;
                    break;
                }
            }
        }
    })
    .await;

    assert!(
        timeout.is_ok(),
        "Timed out waiting for WorkflowAborted NATS emission"
    );
    assert!(
        workflow_aborted_found,
        "WorkflowAborted event was not found"
    );
}

#[tokio::test]
async fn test_runner_heartbeat_updates_db() {
    let (pool, _nats_client, _tls_reloader, _opa_client) = setup().await;

    let runner_id = uuid::Uuid::new_v4().to_string();

    // First we must have the runner registered in the DB
    sqlx::query("INSERT INTO runners (id, runner_type, protocol_version, nats_subject, capabilities, status, last_heartbeat_at, registered_at) VALUES ($1, 'docker', 'v1', 'subject', '{}', 'online', NOW(), NOW())")
        .bind(&runner_id)
        .execute(&pool)
        .await
        .unwrap();

    let heartbeat = RunnerHeartbeatEvent {
        runner_id: runner_id.clone(),
        version: "1.0.0".to_string(),
        state: RunnerStatus::Online,
    };

    // Simulate ingest
    handle_runner_heartbeat(serde_json::to_value(heartbeat).unwrap(), pool.clone())
        .await
        .unwrap();

    // Assert DB
    let status: String = sqlx::query_scalar("SELECT status::text FROM runners WHERE id = $1")
        .bind(&runner_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(status, "online");
}
