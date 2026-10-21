//! A step claimed for dispatch and never handed over, its engine dead, is put back and
//! dispatched again; so is a step left pending with no event coming to dispatch it.

mod common;

use common::{delete_run, nats, setup_db, tls_reloader};
use serde_json::json;
use std::time::Duration;
use stormchaser_engine::handler;
use stormchaser_engine::step_machine::{state, StepMachine};
use stormchaser_engine::workers::recover_dispatches;
use stormchaser_model::events::StepFailedEvent;
use stormchaser_model::{RunId, StepInstanceId};
use uuid::Uuid;

/// A sweep covers every run in the database, so one test's would take another's steps
/// mid-way. In an engine only the leader sweeps.
static SWEEP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let nats_client = nats().await;
    let tls_reloader = tls_reloader().await;
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

/// Sets the run's CPU in use, and whether the step holds one CPU of it
async fn use_cpu(pool: &sqlx::PgPool, run_id: RunId, step_id: Uuid, in_use: f64, held: bool) {
    sqlx::query("UPDATE run_quotas SET current_cpu_usage = $2 WHERE run_id = $1")
        .bind(run_id)
        .bind(in_use)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        r#"UPDATE step_instances SET spec = '{"image": "alpine", "command": ["true"], "cpu": "1"}', quota_held = $2 WHERE id = $1"#,
    )
    .bind(step_id)
    .bind(held)
    .execute(pool)
    .await
    .unwrap();
}

async fn cpu_in_use(pool: &sqlx::PgPool, run_id: RunId) -> f64 {
    sqlx::query_scalar("SELECT current_cpu_usage FROM run_quotas WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_lost_dispatch_gives_back_only_the_quota_it_holds() {
    let _turn = SWEEP.lock().await;
    let pool = setup_db().await;

    // Lost after it claimed a CPU: given back, then claimed again on dispatch
    let (held_run, held_step) = run_with_step(&pool, "initializing", Some(600.0), false).await;
    use_cpu(&pool, held_run, held_step, 1.0, true).await;
    // Lost before it claimed any (its engine died first, or its failed dispatch gave it
    // back already): the CPU in use is another step's
    let (other_run, other_step) = run_with_step(&pool, "initializing", Some(600.0), false).await;
    use_cpu(&pool, other_run, other_step, 1.0, false).await;

    recover(&pool).await;

    assert_eq!(step_state(&pool, held_step).await.0, "initializing");
    assert_eq!(cpu_in_use(&pool, held_run).await, 1.0);
    assert_eq!(step_state(&pool, other_step).await.0, "initializing");
    assert_eq!(
        cpu_in_use(&pool, other_run).await,
        2.0,
        "the other step's CPU was given back for it"
    );

    delete_run(&pool, held_run).await;
    delete_run(&pool, other_run).await;
}

#[tokio::test]
async fn a_handed_over_step_whose_dispatch_failed_is_failed() {
    let pool = setup_db().await;
    let nats_client = nats().await;
    let (run_id, step_id) = run_with_step(&pool, "initializing", Some(10.0), true).await;
    let step_id = StepInstanceId::new(step_id);

    // Handed over: its completion token was cleared for the runner
    sqlx::query("UPDATE step_instances SET completion_token = NULL WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();
    let instance = handler::fetch_step_instance(step_id, &pool).await.unwrap();
    handler::requeue_or_fail(
        StepMachine::<state::Initializing>::load(instance).unwrap(),
        "dispatching it failed",
        &pool,
    )
    .await
    .unwrap();

    let payload: String = sqlx::query_scalar(
        "SELECT payload FROM outbox_events WHERE subject LIKE '%.step.failed' AND payload LIKE '%' || $1 || '%'",
    )
    .bind(step_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("its failure is queued");
    let cloud_event: serde_json::Value = serde_json::from_str(&payload).unwrap();
    let failure: StepFailedEvent = serde_json::from_value(cloud_event["data"].clone()).unwrap();

    // A runner that got the task cannot report on it any more
    let mut from_runner = StepFailedEvent::new(run_id, step_id, 0, "exit status 1");
    from_runner.runner_id = Some("test-dispatch-recovery-runner".to_string());
    for event in [from_runner, failure] {
        handler::handle_step_failed(
            event,
            pool.clone(),
            nats_client.clone(),
            tls_reloader().await,
        )
        .await
        .unwrap();
    }

    let status: String =
        sqlx::query_scalar("SELECT status::text FROM step_instances WHERE id = $1 UNION ALL SELECT status::text FROM archived_step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    let error: Option<String> =
        sqlx::query_scalar("SELECT error FROM step_instances WHERE id = $1 UNION ALL SELECT error FROM archived_step_instances WHERE id = $1")
            .bind(step_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        error
            .as_deref()
            .unwrap_or_default()
            .contains("may have reached a runner"),
        "failed by the engine, not the runner: {error:?}"
    );

    delete_run(&pool, run_id).await;
}
