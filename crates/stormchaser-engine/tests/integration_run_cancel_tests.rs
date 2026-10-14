//! A run that ends while its steps are still on runners tells the runners to stop them.

use chrono::Utc;
use futures::StreamExt;
use serde_json::json;
use sqlx::PgPool;
use std::env::var;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::handler;
use stormchaser_model::events::{
    EventSource, EventType, RunCancelEvent, RunnerEventType, StepEventType, StepFailedEvent,
};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::runner::CANCEL_SUBJECT;
use stormchaser_model::{run_cancel, RunId, StepInstanceId};
use stormchaser_tls::TlsReloader;

async fn pool() -> PgPool {
    PgPool::connect(&var("DATABASE_URL").expect("DATABASE_URL must be set"))
        .await
        .unwrap()
}

async fn tls() -> Arc<TlsReloader> {
    Arc::new(
        TlsReloader::new(stormchaser_tls::TlsConfig::default())
            .await
            .unwrap(),
    )
}

async fn nats() -> async_nats::Client {
    let url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    stormchaser_model::nats::connect(url).await.unwrap()
}

/// Inserts a runner whose last heartbeat was `seconds_ago`.
async fn insert_runner(pool: &PgPool, id: &str, seconds_ago: f64) {
    sqlx::query(
        "INSERT INTO runners (id, runner_type, protocol_version, nats_subject, last_heartbeat_at) VALUES ($1, 'docker', 'v1', $2, NOW() - make_interval(secs => $3))",
    )
    .bind(id)
    .bind(format!("stormchaser.v1.runner.docker.{id}"))
    .bind(seconds_ago)
    .execute(pool)
    .await
    .unwrap();
}

/// A running step that has been handed to `runner` (so it has no completion token).
async fn insert_step_on(pool: &PgPool, runner: &str) -> (RunId, StepInstanceId) {
    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();
    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_name, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'run-cancel-test', 'running', 1, 'http://git.local', 'workflow.storm', 'main')"
    )
    .bind(run_id)
    .execute(pool)
    .await
    .unwrap();
    let workflow = stormchaser_dsl::StormchaserParser::new()
        .parse(
            r#"
        stormchaser_dsl_version = "v1"
        workflow "run-cancel-test" {
            steps {
                step "build" "RunContainer" {
                    image = "alpine"
                }
            }
        }
    "#,
        )
        .unwrap();
    stormchaser_engine::db::runs::insert_run_context(
        pool,
        run_id,
        "v1",
        serde_json::to_value(&workflow).unwrap(),
        Some("code"),
        json!({}),
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type, runner_id, started_at, spec, completion_token) VALUES ($1, $2, 'build', 'running', 'RunContainer', $3, NOW(), '{}', NULL)")
        .bind(step_id)
        .bind(run_id)
        .bind(runner)
        .execute(pool)
        .await
        .unwrap();
    (run_id, step_id)
}

/// The run ids of the cancellations waiting in the outbox.
async fn queued_cancellations(pool: &PgPool, run_id: RunId) -> usize {
    let payloads: Vec<(String,)> =
        sqlx::query_as("SELECT payload FROM outbox_events WHERE subject = $1")
            .bind(CANCEL_SUBJECT)
            .fetch_all(pool)
            .await
            .unwrap();
    payloads
        .iter()
        .filter(|(payload,)| payload.contains(&run_id.to_string()))
        .count()
}

#[tokio::test]
async fn a_run_that_fails_tells_runners_to_stop_its_other_steps() {
    let pool = pool().await;
    let runner = format!("cancel-{}", uuid::Uuid::new_v4().simple());
    insert_runner(&pool, &runner, 0.0).await;
    let (run_id, step_id) = insert_step_on(&pool, &runner).await;
    assert_eq!(queued_cancellations(&pool, run_id).await, 0);

    handler::step::events::handle_step_failed(
        StepFailedEvent {
            run_id,
            step_id,
            fencing_token: 1,
            completion_token: None,
            event_type: EventType::Step(StepEventType::Failed),
            error: "exit 1".to_string(),
            runner_id: Some(runner.clone()),
            exit_code: Some(1),
            storage_hashes: None,
            artifacts: None,
            test_reports: None,
            outputs: None,
            timestamp: Utc::now(),
        },
        pool.clone(),
        nats().await,
        tls().await,
    )
    .await
    .unwrap();

    assert_eq!(queued_cancellations(&pool, run_id).await, 1);
}

#[tokio::test]
async fn a_run_that_times_out_tells_runners_to_stop_its_steps() {
    let pool = pool().await;
    let runner = format!("cancel-{}", uuid::Uuid::new_v4().simple());
    insert_runner(&pool, &runner, 0.0).await;
    let (run_id, _) = insert_step_on(&pool, &runner).await;

    handler::handle_workflow_timeout(run_id, pool.clone(), nats().await, tls().await)
        .await
        .unwrap();

    assert_eq!(queued_cancellations(&pool, run_id).await, 1);
}

/// What the engine publishes is what a runner reads: the cancellation arrives on the
/// runner's own consumer and names the run, and other messages do not count as one.
#[tokio::test]
async fn a_runner_reads_the_cancellation_the_engine_publishes() {
    let js = async_nats::jetstream::new(nats().await);
    let stream = js
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: "stormchaser".to_string(),
            subjects: vec!["stormchaser.>".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();
    let mut cancellations = run_cancel::subscribe(&stream).await.unwrap();

    let run_id = RunId::new_v4();
    let event_type = EventType::Runner(RunnerEventType::Cancel);
    publish_cloudevent(
        &js,
        NatsSubject::RunnerCancel,
        event_type.clone(),
        EventSource::Engine,
        serde_json::to_value(RunCancelEvent {
            run_id,
            event_type,
            reason: "the run failed".to_string(),
            timestamp: Utc::now(),
        })
        .unwrap(),
        None,
        None,
    )
    .await
    .unwrap();

    // Other tests and earlier runs publish cancellations too; find this one
    let found = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(Ok(message)) = cancellations.next().await {
            if run_cancel::parse(&message).is_some_and(|event| event.run_id == run_id) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(found, "the runner never saw the cancellation");
}

fn cancel_payload(run_id: RunId) -> Vec<u8> {
    let event_type = EventType::Runner(RunnerEventType::Cancel);
    let (payload, _) = stormchaser_model::nats::build_cloudevent_and_headers(
        event_type.clone(),
        EventSource::Engine,
        serde_json::to_value(RunCancelEvent {
            run_id,
            event_type,
            reason: "forged".to_string(),
            timestamp: Utc::now(),
        })
        .unwrap(),
        None,
        None,
    )
    .unwrap();
    payload.into_bytes()
}

/// Checks the server's permissions, which are what make a cancellation trustworthy:
/// a runner can read the cancel subject, and neither it nor the API can publish there.
///
/// Ignored by default. Needs a NATS server running `deploy/nats/nats-server.conf`:
///
/// ```text
/// STORMCHASER_PERM_NATS_URL=nats://127.0.0.1:4299 NATS_CONTROL_PASSWORD=... \
///   NATS_API_PASSWORD=... NATS_RUNNER_PASSWORD=... \
///   cargo test -p stormchaser-engine --test integration_run_cancel_tests -- --ignored
/// ```
#[tokio::test]
#[ignore = "needs a NATS server with the per-role users"]
async fn only_the_engine_can_cancel_a_run() {
    let url = var("STORMCHASER_PERM_NATS_URL").expect("STORMCHASER_PERM_NATS_URL must be set");
    let connect = |user: &str, password_var: &str| {
        let options = async_nats::ConnectOptions::new()
            .user_and_password(user.to_string(), var(password_var).expect(password_var));
        async_nats::connect_with_options(url.clone(), options)
    };
    let engine = connect("stormchaser", "NATS_CONTROL_PASSWORD")
        .await
        .unwrap();
    let api = connect("stormchaser-api", "NATS_API_PASSWORD")
        .await
        .unwrap();
    let runner = connect("stormchaser-runner", "NATS_RUNNER_PASSWORD")
        .await
        .unwrap();

    async_nats::jetstream::new(engine.clone())
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: "stormchaser".to_string(),
            subjects: vec!["stormchaser.>".to_string()],
            ..Default::default()
        })
        .await
        .unwrap();

    // A runner can open its reader with its own, restricted, account
    let runner_stream = async_nats::jetstream::new(runner.clone())
        .get_stream("stormchaser")
        .await
        .unwrap();
    let mut cancellations = run_cancel::subscribe(&runner_stream).await.unwrap();

    // Forgeries from the runner and the API, then the real thing from the engine
    let forged_by_runner = RunId::new_v4();
    let forged_by_api = RunId::new_v4();
    let real = RunId::new_v4();
    runner
        .publish(CANCEL_SUBJECT, cancel_payload(forged_by_runner).into())
        .await
        .unwrap();
    runner.flush().await.unwrap();
    api.publish(CANCEL_SUBJECT, cancel_payload(forged_by_api).into())
        .await
        .unwrap();
    api.flush().await.unwrap();
    engine
        .publish(CANCEL_SUBJECT, cancel_payload(real).into())
        .await
        .unwrap();
    engine.flush().await.unwrap();

    // The first cancellation the runner sees is the engine's: the forgeries were
    // published earlier and would have arrived first had the server accepted them
    let first = tokio::time::timeout(Duration::from_secs(20), cancellations.next())
        .await
        .expect("the runner saw no cancellation")
        .unwrap()
        .unwrap();
    assert_eq!(run_cancel::parse(&first).unwrap().run_id, real);
}
