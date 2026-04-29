use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            std::env::var("STORMCHASER_DEV_PASSWORD").unwrap_or_else(|_| "stormchaser".to_string())
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test]
async fn test_direct_run_inserts_quotas() {
    let pool = setup_db().await;
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let run_id = Uuid::new_v4();
    let workflow_name = format!("test-direct-run-{}", run_id);
    let dsl = format!(
        r#"
        stormchaser_dsl_version = "1.0"
        workflow "{}" {{
            steps {{
                step "hello" "RunContainer" {{
                    image = "alpine"
                    command = ["echo", "hello"]
                }}
            }}
        }}
    "#,
        workflow_name
    );

    let payload = json!({
        "run_id": run_id,
        "dsl": dsl,
        "inputs": {},
        "initiating_user": "test-user"
    });

    // Clean up before test
    sqlx::query("DELETE FROM run_quotas WHERE run_id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    handler::handle_workflow_direct(
        payload,
        pool.clone(),
        opa_client.clone(),
        nats_client.clone(),
    )
    .await
    .unwrap();

    // Verify quotas were inserted
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_quotas WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        count, 1,
        "Default run quotas should be inserted for direct runs"
    );

    // Final cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_dispatch_pending_steps_column_created_at() {
    let pool = setup_db().await;
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let run_id = Uuid::new_v4();
    let workflow_name = format!("test-dispatch-{}", run_id);

    // 1. Setup minimal run and quotas
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, 'test', 'running', $3, 'http://example.com', 'test.storm', 'main')")
        .bind(run_id)
        .bind(&workflow_name)
        .bind(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0))
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO run_quotas (run_id, max_concurrency, max_cpu, max_memory, max_storage, timeout) VALUES ($1, 10, '1', '1Gi', '10Gi', '1h')")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    let workflow_def = serde_json::json!({
        "dsl_version": "1.0",
        "name": workflow_name,
        "steps": [],
        "libraries": [],
        "storage": [],
        "event_rules": [],
        "inputs": [],
        "outputs": [],
        "handlers": []
    });

    sqlx::query("INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs) VALUES ($1, '1.0', $2, '', '{}')")
        .bind(run_id)
        .bind(workflow_def)
        .execute(&pool)
        .await
        .unwrap();

    // 2. Insert a pending step
    let step_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params, created_at)
        VALUES ($1, $2, 'test-step', 'RunContainer', 'pending', $3, $4, NOW())
        "#
    )
    .bind(step_id)
    .bind(run_id)
    .bind(json!({}))
    .bind(json!({}))
    .execute(&pool)
    .await
    .unwrap();

    // 3. Call dispatch_pending_steps - this should NOT fail with "no column found for name: created_at"
    handler::dispatch_pending_steps(
        run_id,
        pool.clone(),
        nats_client.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .expect("dispatch_pending_steps should succeed with created_at column present");

    // Cleanup
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}
