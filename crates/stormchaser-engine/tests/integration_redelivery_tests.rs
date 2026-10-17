//! JetStream delivers an event again when it is not acknowledged in time, or when its
//! handler failed. Handling a run's events a second time must not run it a second time.

use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;
use stormchaser_engine::git_cache::GitCache;
use stormchaser_engine::handler;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use stormchaser_tls::{TlsConfig, TlsReloader};

async fn setup_db() -> sqlx::PgPool {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

async fn nats() -> async_nats::Client {
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    stormchaser_model::nats::connect(nats_url).await.unwrap()
}

async fn status_and_version(pool: &sqlx::PgPool, run_id: RunId) -> (RunStatus, i32) {
    sqlx::query_as("SELECT status, version FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn step_count(pool: &sqlx::PgPool, run_id: RunId) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM step_instances WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn delete_run(pool: &sqlx::PgPool, run_id: RunId) {
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
}

fn direct_payload(run_id: RunId) -> serde_json::Value {
    json!({
        "run_id": run_id,
        "dsl": format!(r#"
            stormchaser_dsl_version = "1.0"
            workflow "test-redelivery-{run_id}" {{
                steps {{
                    step "hello" "RunContainer" {{
                        image = "alpine"
                        command = ["echo", "hello"]
                    }}
                }}
            }}
        "#),
        "inputs": {},
        "initiating_user": "test-user"
    })
}

#[tokio::test]
async fn a_redelivered_direct_run_is_not_created_again() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let opa_client = Arc::new(OpaClient::new(None, None));
    let run_id = RunId::new_v4();

    for _ in 0..2 {
        handler::handle_workflow_direct(
            direct_payload(run_id),
            pool.clone(),
            opa_client.clone(),
            nats_client.clone(),
        )
        .await
        .expect("a redelivered direct run is not an error");
    }

    let quotas: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_quotas WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(quotas, 1);
    assert_eq!(
        status_and_version(&pool, run_id).await.0,
        RunStatus::StartPending
    );

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_redelivered_start_pending_does_not_start_the_run_again() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let opa_client = Arc::new(OpaClient::new(None, None));
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
    let run_id = RunId::new_v4();

    handler::handle_workflow_direct(
        direct_payload(run_id),
        pool.clone(),
        opa_client,
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
    let after_first = status_and_version(&pool, run_id).await;
    assert_eq!(after_first.0, RunStatus::Running);
    assert_eq!(step_count(&pool, run_id).await, 1);

    handler::handle_workflow_start_pending(run_id, pool.clone(), nats_client, tls_reloader)
        .await
        .expect("a redelivered start_pending is not an error");

    assert_eq!(status_and_version(&pool, run_id).await, after_first);
    assert_eq!(step_count(&pool, run_id).await, 1);

    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_redelivered_queued_event_leaves_a_started_run_alone() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
    let git_dir = tempfile::tempdir().unwrap();
    let run_id = RunId::new_v4();

    // Its repository does not exist: were the run resolved again, it would fail
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'test-redelivery', 'test', 'running', $2, 'https://invalid.example/repo.git', 'test.storm', 'main')")
        .bind(run_id)
        .bind(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0))
        .execute(&pool)
        .await
        .unwrap();
    let before = status_and_version(&pool, run_id).await;

    handler::handle_workflow_queued(
        run_id,
        json!({}),
        pool.clone(),
        Arc::new(GitCache::new(git_dir.path())),
        Arc::new(OpaClient::new(None, None)),
        nats_client,
        tls_reloader,
    )
    .await
    .expect("a redelivered queued event is not an error");

    assert_eq!(status_and_version(&pool, run_id).await, before);

    delete_run(&pool, run_id).await;
}

/// A finished run left unarchived, its archive having failed after its last step's
/// result was saved
async fn finished_unarchived_run(pool: &sqlx::PgPool) -> (RunId, uuid::Uuid) {
    let run_id = RunId::new_v4();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'test-redelivery-archive', 'test', 'succeeded', 1, 'http://example.com', 'test.storm', 'main')")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs) VALUES ($1, '1.0', '{}', '', '{}')")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    let step_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params) VALUES ($1, $2, 'only', 'RunContainer', 'succeeded', '{}', '{}')")
        .bind(step_id)
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    (run_id, step_id)
}

async fn archived(pool: &sqlx::PgPool, run_id: RunId) -> bool {
    let live: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM workflow_runs WHERE id = $1)")
            .bind(run_id)
            .fetch_one(pool)
            .await
            .unwrap();
    let kept: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM archived_workflow_runs WHERE id = $1)")
            .bind(run_id)
            .fetch_one(pool)
            .await
            .unwrap();
    !live && kept
}

async fn delete_archived_run(pool: &sqlx::PgPool, run_id: RunId) {
    sqlx::query("DELETE FROM archived_workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_redelivered_step_completion_archives_its_finished_run() {
    let pool = setup_db().await;
    let (run_id, step_id) = finished_unarchived_run(&pool).await;

    handler::handle_step_completed(
        serde_json::from_value(json!({
            "run_id": run_id, "step_id": step_id, "event_type": "StepCompletedEvent",
            "fencing_token": 1, "timestamp": chrono::Utc::now(), "exit_code": 0
        }))
        .unwrap(),
        pool.clone(),
        nats().await,
        Arc::new(None),
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    assert!(archived(&pool, run_id).await);
    delete_archived_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_redelivered_step_failure_archives_its_finished_run() {
    let pool = setup_db().await;
    let (run_id, step_id) = finished_unarchived_run(&pool).await;

    handler::handle_step_failed(
        serde_json::from_value(json!({
            "run_id": run_id, "step_id": step_id, "event_type": "StepFailedEvent",
            "fencing_token": 1, "timestamp": chrono::Utc::now(), "error": "boom",
            "exit_code": 1
        }))
        .unwrap(),
        pool.clone(),
        nats().await,
        Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap()),
    )
    .await
    .unwrap();

    assert!(archived(&pool, run_id).await);
    delete_archived_run(&pool, run_id).await;
}
