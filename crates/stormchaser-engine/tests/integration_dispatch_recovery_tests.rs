//! A step claimed for dispatch and never handed over, its engine dead, is put back and
//! dispatched again; so is a step left pending with no event coming to dispatch it.

use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::workers::recover_dispatches;
use stormchaser_model::RunId;
use stormchaser_tls::{TlsConfig, TlsReloader};
use uuid::Uuid;

/// A sweep covers every run in the database, so one test's would take another's steps
/// mid-way. In an engine only the leader sweeps.
static SWEEP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

/// A running run with one container step in `status`, claimed `claimed_secs_ago`
async fn run_with_step(
    pool: &sqlx::PgPool,
    status: &str,
    claimed_secs_ago: Option<f64>,
    dispatched: bool,
) -> (RunId, Uuid) {
    let run_id = RunId::new_v4();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, 'test', 'running', $3, 'http://example.com', 'test.storm', 'main')")
        .bind(run_id)
        .bind(format!("test-dispatch-recovery-{}", run_id))
        .bind(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO run_quotas (run_id, max_concurrency, max_cpu, max_memory, max_storage, timeout) VALUES ($1, 10, '4', '8Gi', '10Gi', '1h')")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
    let workflow_def = json!({
        "dsl_version": "1.0", "name": "test-dispatch-recovery", "steps": [],
        "libraries": [], "storage": [], "event_rules": [], "inputs": [], "outputs": [],
        "handlers": []
    });
    sqlx::query("INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs) VALUES ($1, '1.0', $2, '', '{}')")
        .bind(run_id)
        .bind(workflow_def)
        .execute(pool)
        .await
        .unwrap();

    let step_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params, created_at, claimed_at, dispatched_at)
        VALUES ($1, $2, 'test-step', 'RunContainer', $3::step_status,
                '{"image": "alpine", "command": ["true"]}', '{}',
                NOW() - INTERVAL '1 hour',
                NOW() - make_interval(secs => $4),
                CASE WHEN $5 THEN NOW() END)
        "#,
    )
    .bind(step_id)
    .bind(run_id)
    .bind(status)
    .bind(claimed_secs_ago)
    .bind(dispatched)
    .execute(pool)
    .await
    .unwrap();
    (run_id, step_id)
}

async fn step_state(pool: &sqlx::PgPool, step_id: Uuid) -> (String, bool, i64) {
    let (status, dispatched): (String, bool) = sqlx::query_as(
        "SELECT status::text, dispatched_at IS NOT NULL FROM step_instances WHERE id = $1",
    )
    .bind(step_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let task_messages: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM dispatched_task_messages WHERE step_instance_id = $1",
    )
    .bind(step_id)
    .fetch_one(pool)
    .await
    .unwrap();
    (status, dispatched, task_messages)
}

async fn recover(pool: &sqlx::PgPool) {
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let tls_reloader = Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap());
    recover_dispatches(
        pool,
        &nats_client,
        &tls_reloader,
        Duration::from_secs(300),
        Duration::from_secs(60),
    )
    .await
    .unwrap();
}

async fn delete_run(pool: &sqlx::PgPool, run_id: RunId) {
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_step_whose_dispatch_was_lost_is_dispatched_again() {
    let _turn = SWEEP.lock().await;
    let pool = setup_db().await;
    let (run_id, step_id) = run_with_step(&pool, "initializing", Some(600.0), false).await;

    recover(&pool).await;

    assert_eq!(
        step_state(&pool, step_id).await,
        ("initializing".to_string(), true, 1),
        "claimed again, handed over and its task message recorded"
    );
    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_step_that_was_handed_over_is_left_alone() {
    let _turn = SWEEP.lock().await;
    let pool = setup_db().await;
    let (run_id, step_id) = run_with_step(&pool, "initializing", Some(600.0), true).await;

    recover(&pool).await;

    assert_eq!(
        step_state(&pool, step_id).await,
        ("initializing".to_string(), true, 0)
    );
    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_step_being_dispatched_now_is_left_alone() {
    let _turn = SWEEP.lock().await;
    let pool = setup_db().await;
    let (run_id, step_id) = run_with_step(&pool, "initializing", Some(10.0), false).await;

    recover(&pool).await;

    assert_eq!(
        step_state(&pool, step_id).await,
        ("initializing".to_string(), false, 0)
    );
    delete_run(&pool, run_id).await;
}

#[tokio::test]
async fn a_pending_step_with_nothing_to_dispatch_it_is_dispatched() {
    let _turn = SWEEP.lock().await;
    let pool = setup_db().await;
    let (run_id, step_id) = run_with_step(&pool, "pending", None, false).await;

    recover(&pool).await;

    assert_eq!(
        step_state(&pool, step_id).await,
        ("initializing".to_string(), true, 1)
    );
    delete_run(&pool, run_id).await;
}
