use cloudevents::Data;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::RunId;
use uuid::Uuid;

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test]
async fn test_resolve_storage_provision() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

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

    // 1. Setup a test S3 backend for artifact provisioning (is_default_sfs is FALSE;
    //    provisioning resolves backends by artifact registry entry, not by default SFS flag)
    let connection_id = Uuid::new_v4();
    let backend_name = format!("test-s3-{}", connection_id);
    sqlx::query(
        r#"
        INSERT INTO connections (id, name, connection_type, config, is_default_sfs)
        VALUES ($1, $2, 's3', $3, FALSE)
        "#,
    )
    .bind(connection_id)
    .bind(&backend_name)
    .bind(json!({
        "bucket": "test-bucket",
        "endpoint": "http://localhost:9000",
        "region": "us-east-1",
        "access_key": "test",
        "secret_key": "test"
    }))
    .execute(&pool)
    .await
    .unwrap();

    let run_id = RunId::new_v4();

    let artifact_name = "test-artifact";

    let dsl = format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "provision-test" {{
            storage "workspace" {{
                size = "1Gi"
                backend = "{}"
                provision {{
                    artifact "{}" {{
                        destination = "/data"
                    }}
                }}
            }}
            steps {{
                step "build" "RunContainer" {{
                    image = "alpine"
                    storage_mounts = [
                        {{ name = "workspace", mount_path = "/workspace" }}
                    ]
                }}
            }}
        }}
    "#,
        backend_name, artifact_name
    );

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let mut sub = nats_client
        .subscribe("stormchaser.v1.*.step.scheduled.runcontainer")
        .await
        .unwrap();

    // Initialize workflow
    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {},
        "initiating_user": "test"
    });

    // This will insert the run_context and step_instances
    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        opa_client.clone(),
        nats_client.clone(),
    )
    .await
    .unwrap();

    // Now insert the dummy step and artifact registry entries
    let step_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params)
        VALUES ($1, $2, 'dummy-step', 'dummy', 'succeeded', $3, $4)
        "#,
    )
    .bind(step_id)
    .bind(run_id)
    .bind(json!({}))
    .bind(json!({}))
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO artifact_registry (run_id, step_instance_id, artifact_name, connection_id, remote_path, metadata)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(run_id)
    .bind(step_id)
    .bind(artifact_name)
    .bind(connection_id)
    .bind("path/to/artifact.tar.gz")
    .bind(json!({}))
    .execute(&pool)
    .await
    .unwrap();

    // Now start the workflow, this will enqueue steps which triggers dispatch
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    use futures::StreamExt;
    let expected_run_id = run_id.to_string();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let event_data: serde_json::Value = loop {
        let msg = tokio::time::timeout_at(deadline, sub.next())
            .await
            .expect("Timed out waiting for scheduled event")
            .expect("Subscription closed while waiting for scheduled event");

        let ce: cloudevents::Event = serde_json::from_slice(&msg.payload).unwrap();
        if let Some(Data::Json(payload)) = ce.data() {
            if payload
                .get("run_id")
                .and_then(|v| v.as_str())
                .map(|id| id == expected_run_id)
                .unwrap_or(false)
            {
                break payload.clone();
            }
        }
    };

    let storage = event_data.get("storage").unwrap().as_object().unwrap();
    let workspace = storage.get("workspace").unwrap();
    let provision = workspace.get("provision").unwrap().as_array().unwrap();
    assert_eq!(provision.len(), 1);
    let prov = &provision[0];
    assert!(prov.get("url").is_some());
    assert!(prov
        .get("url")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("test-bucket"));

    // Cleanup test-specific data inserted by this test
    sqlx::query("DELETE FROM artifact_registry WHERE connection_id = $1")
        .bind(connection_id)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("DELETE FROM connections WHERE id = $1")
        .bind(connection_id)
        .execute(&pool)
        .await
        .unwrap();
}
