use chrono::Utc;
use std::sync::Arc;
use stormchaser_engine::handler;
use stormchaser_model::step::StepInstance;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;

// Assumes there's a test setup function like in other tests
mod common {
    use std::env::var;
    use std::sync::Arc;
    use stormchaser_model::auth::{self, OpaClient};
    /// Get pool.
    pub async fn get_pool() -> sqlx::PgPool {
        let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
            dotenvy::dotenv().ok();
            format!(
                "postgres://stormchaser:{}@localhost:5432/stormchaser",
                var("STORMCHASER_DEV_PASSWORD")
                    .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
            )
        });
        sqlx::PgPool::connect(&db_url).await.unwrap()
    }

    /// Setup test env.
    pub async fn setup_test_env() -> (sqlx::PgPool, async_nats::Client, Arc<auth::OpaClient>) {
        let pool = get_pool().await;
        let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
        let nats_client = async_nats::connect(&nats_url).await.unwrap();

        let opa_client = OpaClient::new(None, None);

        (pool, nats_client, Arc::new(opa_client))
    }
}

use stormchaser_dsl::StormchaserParser;
use stormchaser_engine::db;
use stormchaser_model::workflow;
use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[tokio::test]
async fn test_step_library_direct_run() {
    let (pool, nats_client, _) = common::setup_test_env().await;
    let run_id = RunId::new_v4();

    let dsl = r#"
        stormchaser_dsl_version = "0.1"
        workflow "library_test" {
            step_library "ubuntu_base" {
                type = "RunContainer"
                params {
                    base_tag = "latest"
                }
                spec {
                    image = "ubuntu:22.04"
                }
            }

            steps {
                step "build" "ubuntu_base" {
                    params {
                        cmd = "echo"
                    }
                    command = ["${inputs.cmd}", "hello"]
                }
            }
        }
    "#;

    // Instead of using handle_workflow_direct which publishes to NATS and triggers
    // the system-wide stormchaser-engine daemon, we'll manually insert the run context
    let parser = StormchaserParser::new();
    let parsed_workflow = parser.parse(dsl).unwrap();
    let run = workflow::WorkflowRun {
        id: run_id,
        workflow_name: parsed_workflow.name.clone(),
        initiating_user: "test".to_string(),
        repo_url: "direct://".to_string(),
        workflow_path: "inline.storm".to_string(),
        git_ref: "HEAD".to_string(),
        status: RunStatus::StartPending,
        version: 1,
        fencing_token: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_resolving_at: Some(Utc::now()),
        started_at: None,
        finished_at: None,
        error: None,
    };

    let mut tx = pool.begin().await.unwrap();
    db::insert_full_workflow_run(
        &mut tx,
        &run,
        &parsed_workflow.dsl_version,
        serde_json::to_value(&parsed_workflow).unwrap(),
        Some(dsl),
        serde_json::json!({}),
        10,
        "1",
        "4Gi",
        "10Gi",
        "1h",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Start it
    handler::handle_workflow_start_pending(
        run_id,
        pool.clone(),
        nats_client.clone(),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    // Verify step instance was created with merged library values
    let step: StepInstance = sqlx::query_as(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1"#
    )
    .bind(run_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(step.step_name, "build");
    assert_eq!(step.step_type, "RunContainer"); // resolved from library

    // spec should contain image from library and command from step
    assert_eq!(step.spec["image"], "ubuntu:22.04");
    assert_eq!(step.spec["command"][1], "hello");

    // params should contain base_tag from library and cmd from step
    assert_eq!(step.params["base_tag"], "latest");
    assert_eq!(step.params["cmd"], "echo");
}
