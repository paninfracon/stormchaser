use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::step::StepInstance;
use uuid::Uuid;

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test]
async fn test_artifact_persistence_on_completion() {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    // 1. Setup a default SFS backend
    let backend_id = Uuid::new_v4();
    let backend_name = format!("test-minio-{}", backend_id);
    sqlx::query(
        r#"
        INSERT INTO storage_backends (id, name, backend_type, config, is_default_sfs)
        VALUES ($1, $2, 's3', $3, FALSE)
        "#,
    )
    .bind(backend_id)
    .bind(&backend_name)
    .bind(json!({"bucket": "test-bucket", "endpoint": "http://localhost:9000"}))
    .execute(&pool)
    .await
    .unwrap();

    let run_id = Uuid::new_v4();
    let dsl = format!(
        r#"
        stormchaser_dsl_version = "v1"
        workflow "artifact-test" {{
            storage "workspace" {{
                size = "1Gi"
                backend = "{}"
                artifact "app-bin" {{
                    path = "bin/app"
                    retention = "7d"
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
        backend_name
    );

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    // 2. Initialize workflow
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
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    let step: StepInstance = sqlx::query_as(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1"#
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 3. Mock completion with artifacts
    let completed_payload = json!({
        "run_id": run_id,
        "step_id": step.id,
        "status": "succeeded",
        "exit_code": 0,
        "artifacts": {
            "app-bin": {
                "hash": "abc123hash",
                "size": 1024,
                "content_type": "application/octet-stream"
            }
        }
    });

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

    // 4. Verify artifact was registered (check archived table because it completes and archives)
    let artifact_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM archived_artifact_registry WHERE run_id = $1 AND artifact_name = $2)",
    )
    .bind(run_id)
    .bind("app-bin")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        artifact_exists,
        "Artifact should have been registered in archived_artifact_registry"
    );

    let artifact: (String, Uuid, String) = sqlx::query_as(
        "SELECT artifact_name, backend_id, remote_path FROM archived_artifact_registry WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(artifact.0, "app-bin");
    assert_eq!(artifact.1, backend_id);
    assert_eq!(
        artifact.2,
        format!("artifacts/{}/workspace/app-bin", run_id)
    );
}

#[tokio::test]
async fn test_test_report_persistence_on_completion() {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
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
        workflow "report-test" {
            steps {
                step "test" "RunContainer" {
                    image = "alpine"
                    reports {
                        report "unit-tests" {
                            path = "target/results/*.xml"
                            format = "junit"
                        }
                    }
                }
            }
        }
    "#;

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

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
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    let step: StepInstance = sqlx::query_as(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1"#
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // 2. Mock completion with test reports
    let completed_payload = json!({
        "run_id": run_id,
        "step_id": step.id,
        "status": "succeeded",
        "exit_code": 0,
        "test_reports": {
            "unit-tests_results.xml": {
                "name": "unit-tests",
                "file_name": "results.xml",
                "format": "junit",
                "content": "<testsuite>...</testsuite>",
                "hash": "hash123"
            }
        }
    });

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

    // 3. Verify report was registered (check archived table)
    let report_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM archived_step_test_reports WHERE run_id = $1 AND report_name = $2)",
    )
    .bind(run_id)
    .bind("unit-tests")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        report_exists,
        "Test report should have been registered in archived_step_test_reports"
    );

    let report: (String, String, String, String) = sqlx::query_as(
        "SELECT report_name, file_name, format, checksum FROM archived_step_test_reports WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(report.0, "unit-tests");
    assert_eq!(report.1, "results.xml");
    assert_eq!(report.2, "junit");
    assert_eq!(report.3, "hash123");
}
