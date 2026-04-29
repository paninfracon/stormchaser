use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::step::{StepInstance, StepStatus};
use uuid::Uuid;

use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test]
async fn test_dynamic_parallelism_with_batching() {
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

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();
    let opa_client = Arc::new(OpaClient::new(None, None));

    let run_id = Uuid::new_v4();
    let dsl = r#"
        stormchaser_dsl_version = "v1"
        workflow "parallel-test" {
            steps {
                step "generate" "RunContainer" {
                    image = "alpine"
                    next = ["process"]
                }
                step "process" "RunContainer" {
                    iterate = "[\"a\", \"b\", \"c\", \"d\"]"
                    iterate_as = "item"
                    strategy {
                        max_parallel = 2
                    }
                    params = {
                        val = "${item}"
                    }
                    image = "alpine"
                }
            }
        }
    "#;

    // 1. Trigger handle_workflow_direct
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

    // 1.5. Handle start_pending to schedule initial steps
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    // 2. Verify "generate" step was scheduled
    let instances: Vec<StepInstance> = sqlx::query_as(r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1"#)
        .bind(run_id)
        .fetch_all(&pool)
        .await
        .unwrap();

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].step_name, "generate");
    // We are running against the shared local NATS and Postgres. If a runner is active,
    // it might immediately pick up the step and transition it from Pending to Running.
    // Thus we accept either Pending or Running.
    assert!(
        instances[0].status == StepStatus::Pending || instances[0].status == StepStatus::Running,
        "Status was {:?}",
        instances[0].status
    );

    let generate_id = instances[0].id;

    // 3. Complete "generate" step -> should trigger "process" with 4 iterations, 2 Pending, 2 Waiting
    let completed_payload = json!({
        "run_id": run_id,
        "step_id": generate_id,
        "exit_code": 0
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

    let instances: Vec<StepInstance> = sqlx::query_as(r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1 ORDER BY iteration_index ASC"#)
        .bind(run_id)
        .fetch_all(&pool)
        .await
        .unwrap();

    // 1 (generate) + 4 (process iterations) = 5
    assert_eq!(instances.len(), 5);

    let process_instances: Vec<&StepInstance> = instances
        .iter()
        .filter(|i| i.step_name == "process")
        .collect();
    assert_eq!(process_instances.len(), 4);

    // Iteration 0, 1 should be Pending or Running (max_parallel = 2)
    assert!(
        process_instances[0].status == StepStatus::Pending
            || process_instances[0].status == StepStatus::Running
    );
    assert!(
        process_instances[1].status == StepStatus::Pending
            || process_instances[1].status == StepStatus::Running
    );
    // Iteration 2, 3 should be WaitingForEvent (queued)
    assert_eq!(process_instances[2].status, StepStatus::WaitingForEvent);
    assert_eq!(process_instances[3].status, StepStatus::WaitingForEvent);

    // 4. Complete iteration 0 -> should trigger iteration 2
    let completed_payload = json!({
        "run_id": run_id,
        "step_id": process_instances[0].id,
        "exit_code": 0
    });
    handler::handle_step_completed(
        completed_payload,
        pool.clone(),
        nats_client.clone(),
        log_backend.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    let it2: StepInstance = sqlx::query_as(r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1"#)
        .bind(process_instances[2].id)
        .fetch_one(&pool)
        .await
        .unwrap();
    // It should be Pending or Running now because handle_step_completed should have promoted it
    assert!(it2.status == StepStatus::Pending || it2.status == StepStatus::Running);

    // Iteration 3 still waiting (because max_parallel is 2 and only 1 finished)
    let it3: StepInstance = sqlx::query_as(r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1"#)
        .bind(process_instances[3].id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(it3.status, StepStatus::WaitingForEvent);
}
