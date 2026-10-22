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
            attempt: None,
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
        CANCEL_SUBJECT,
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

/// Connects as one of the users in `deploy/nats/nats-server.conf`, which the test stack
/// runs, with the password `scripts/test-env.sh` gives it. Each user may read only its
/// own inbox prefix, as stormchaser_model::nats sets it.
async fn connect_as(user: &str, password_var: &str) -> async_nats::Client {
    let url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let password = var(password_var).unwrap_or_else(|_| panic!("{password_var} must be set"));
    let options = async_nats::ConnectOptions::new()
        .user_and_password(user.to_string(), password)
        .custom_inbox_prefix(format!("_INBOX_{user}"));
    async_nats::connect_with_options(url, options)
        .await
        .unwrap_or_else(|e| panic!("cannot connect to NATS as {user}: {e}"))
}

/// Checks the server's permissions, which are what make a cancellation trustworthy:
/// a runner can read the cancel subject, and neither it nor the API can publish there.
#[tokio::test]
async fn only_the_engine_can_cancel_a_run() {
    let engine = connect_as("stormchaser", "STORMCHASER_NATS_PASSWORD").await;
    let api = connect_as("stormchaser-api", "STORMCHASER_NATS_API_PASSWORD").await;
    let runner = connect_as("stormchaser-runner", "STORMCHASER_NATS_RUNNER_PASSWORD").await;
    stormchaser_engine::nats::init_jetstream(&engine)
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

    // The forgeries were published first, so had the server accepted either the runner
    // would see it before the engine's. Cancellations other tests published within the
    // replay window are passed over.
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let message = cancellations
                .next()
                .await
                .expect("the cancellation stream ended")
                .unwrap();
            let run_id = run_cancel::parse(&message).unwrap().run_id;
            assert_ne!(run_id, forged_by_runner, "a runner cancelled a run");
            assert_ne!(run_id, forged_by_api, "the API cancelled a run");
            if run_id == real {
                break;
            }
        }
    })
    .await
    .expect("the runner did not see the engine's cancellation");
}

/// Checks that each user reads only its own inbox prefix: a runner cannot read the
/// engine's deliveries or the API's answers, while its own inbox works.
///
#[tokio::test]
async fn a_user_reads_only_its_own_inbox() {
    let engine = connect_as("stormchaser", "STORMCHASER_NATS_PASSWORD").await;
    let runner = connect_as("stormchaser-runner", "STORMCHASER_NATS_RUNNER_PASSWORD").await;

    let mut own = runner
        .subscribe("_INBOX_stormchaser-runner.probe")
        .await
        .unwrap();
    let mut engines = runner.subscribe("_INBOX_stormchaser.>").await.unwrap();
    let mut apis = runner.subscribe("_INBOX_stormchaser-api.>").await.unwrap();
    let mut shared = runner.subscribe("_INBOX.>").await.unwrap();
    runner.flush().await.unwrap();

    for subject in [
        "_INBOX_stormchaser.probe",
        "_INBOX_stormchaser-api.probe",
        "_INBOX.probe",
        "_INBOX_stormchaser-runner.probe",
    ] {
        engine.publish(subject, "x".into()).await.unwrap();
    }
    engine.flush().await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), own.next())
        .await
        .expect("the runner did not get a message on its own inbox");
    for (name, sub) in [
        ("the engine's", &mut engines),
        ("the API's", &mut apis),
        ("the shared", &mut shared),
    ] {
        assert!(
            tokio::time::timeout(Duration::from_millis(500), sub.next())
                .await
                .is_err(),
            "the runner read {name} inbox"
        );
    }
}

/// Checks that a runner can look the stream up but neither create it nor create (and so
/// reconfigure) one of the engine's consumers. A refused publish gets no answer, while an
/// allowed JetStream request is always answered, even with an error.
///
#[tokio::test]
async fn a_runner_cannot_create_the_stream_or_the_engines_consumers() {
    let engine = connect_as("stormchaser", "STORMCHASER_NATS_PASSWORD").await;
    let runner = connect_as("stormchaser-runner", "STORMCHASER_NATS_RUNNER_PASSWORD").await;
    stormchaser_engine::nats::init_jetstream(&engine)
        .await
        .unwrap();

    let answered = |subject: &'static str, body: serde_json::Value| {
        let runner = runner.clone();
        async move {
            matches!(
                tokio::time::timeout(
                    Duration::from_secs(3),
                    runner.request(subject, body.to_string().into()),
                )
                .await,
                Ok(Ok(_))
            )
        }
    };

    assert!(
        answered("$JS.API.STREAM.INFO.stormchaser", json!({})).await,
        "the runner cannot look the stream up"
    );
    assert!(
        !answered(
            "$JS.API.STREAM.CREATE.stormchaser",
            json!({"name": "stormchaser", "subjects": ["stormchaser.v1.>"], "deny_delete": true})
        )
        .await,
        "the runner may create the stream"
    );
    assert!(
        !answered(
            "$JS.API.CONSUMER.CREATE.stormchaser.orchestration-engine-global.stormchaser.v1.runner.>",
            json!({
                "stream_name": "stormchaser",
                "config": {"name": "orchestration-engine-global", "filter_subject": "stormchaser.v1.runner.>"}
            })
        )
        .await,
        "the runner may redefine the engine's consumer"
    );
    // ...while its own consumers still work
    let stream =
        stormchaser_model::nats::wait_for_stream(&async_nats::jetstream::new(runner.clone())).await;
    stream
        .create_consumer(async_nats::jetstream::consumer::pull::Config {
            name: Some("runner-docker-permission-probe".to_string()),
            filter_subject: "stormchaser.v1.runner.docker.permission-probe".to_string(),
            ..Default::default()
        })
        .await
        .expect("the runner cannot create its own consumer");
}
