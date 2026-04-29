use futures::StreamExt;
use serde_json::json;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use tokio::time::sleep;
use uuid::Uuid;

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test]
async fn test_jq_step_execution() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD").unwrap_or_else(|_| "stormchaser".to_string())
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let run_id = Uuid::new_v4();
    let dsl = r#"
        stormchaser_dsl_version = "v1"
        workflow "jq-test" {
            steps {
                step "transform" "JQ" {
                    spec = {
                        program = ".items | map(.name)"
                        input = {
                            items = [
                                { name = "foo", val = 1 },
                                { name = "bar", val = 2 }
                            ]
                        }
                    }
                    outputs = [
                        { name = "jq_result", source = "result" }
                    ]
                }
            }
        }
    "#;

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    // Subscribe to completion events before dispatching
    let mut completion_sub = nats_client
        .subscribe("stormchaser.step.completed")
        .await
        .unwrap();

    // 1. Initialize workflow
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

    // Start it
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    // 2. Wait for the intrinsic JQ step to execute and publish its completion
    let step_completed_payload;

    // Timeout after 10 seconds
    let timeout = sleep(Duration::from_secs(10));
    tokio::pin!(timeout);

    loop {
        tokio::select! {
            Some(msg) = completion_sub.next() => {
                if let Ok(payload) = serde_json::from_slice::<Value>(&msg.payload) {
                    if payload["run_id"].as_str() == Some(&run_id.to_string()) {
                        step_completed_payload = payload;
                        break;
                    }
                }
            }
            _ = &mut timeout => {
                panic!("Timed out waiting for JQ step to complete");
            }
        }
    }

    let completed_payload = step_completed_payload;
    assert_eq!(completed_payload["event_type"], "step_completed");

    let expected_output = json!(["foo", "bar"]);
    assert_eq!(completed_payload["outputs"]["result"], expected_output);

    // 3. Process the completion through the engine to ensure outputs are saved
    let log_backend = Arc::new(None);
    handler::handle_step_completed(
        completed_payload,
        pool.clone(),
        nats_client.clone(),
        log_backend.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    // 4. Verify outputs are in the database (or archived outputs since step completes and archives)
    let output_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM archived_step_outputs so JOIN archived_step_instances si ON so.step_instance_id = si.id WHERE si.run_id = $1 AND so.key = $2)"
    )
    .bind(run_id)
    .bind("result")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        output_exists,
        "Output 'result' should be registered in archived_step_outputs"
    );

    let output_val: Value = sqlx::query_scalar(
        "SELECT so.value FROM archived_step_outputs so JOIN archived_step_instances si ON so.step_instance_id = si.id WHERE si.run_id = $1 AND so.key = $2"
    )
    .bind(run_id)
    .bind("result")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(output_val, expected_output);
}
