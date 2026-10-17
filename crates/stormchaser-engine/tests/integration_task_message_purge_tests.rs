use async_nats::jetstream::stream::RawMessageErrorKind;
use serde_json::json;
use sqlx::PgPool;
use std::env::var;
use stormchaser_engine::db::task_messages::record_task_message;
use stormchaser_engine::workers::purge_finished_task_messages;
use stormchaser_model::events::{EventSource, EventType, StepEventType};
use stormchaser_model::nats::{publish_cloudevent_with_sequence, NatsSubject, STREAM_NAME};
use uuid::Uuid;

async fn pool() -> PgPool {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPool::connect(&db_url).await.unwrap()
}

/// Publishes a stand-in event on `subject` for `step_id` and returns its stream sequence.
async fn publish(js: &async_nats::jetstream::Context, subject: String, step_id: Uuid) -> u64 {
    publish_cloudevent_with_sequence(
        js,
        NatsSubject::Custom(subject),
        EventType::Step(StepEventType::Scheduled),
        EventSource::System,
        json!({ "step_id": step_id, "spec": { "env": [{ "name": "TOKEN", "value": "s3cret" }] } }),
        None,
        None,
    )
    .await
    .unwrap()
    .expect("the stream should confirm the message")
}

/// Runs the purge until it has nothing left to do (other tests share the table).
async fn purge_all(pool: &PgPool, js: &async_nats::jetstream::Context) {
    for _ in 0..50 {
        if purge_finished_task_messages(pool, js).await.unwrap() == 0 {
            return;
        }
    }
}

async fn recorded(pool: &PgPool, seq: u64) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM dispatched_task_messages WHERE stream_seq = $1)",
    )
    .bind(seq as i64)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn a_task_message_is_erased_from_the_stream_once_its_step_is_finished() {
    let pool = pool().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let js = stormchaser_engine::nats::init_jetstream(&nats)
        .await
        .unwrap();
    let stream = js.get_stream(STREAM_NAME).await.unwrap();

    let run_id = Uuid::new_v4();
    let step_id = Uuid::new_v4();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'purge-test', 'test-user', 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, status, step_type) VALUES ($1, $2, 'build', 'running', 'PurgeTestOnly')")
        .bind(step_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    // No runner listens for this step type
    let seq = publish(
        &js,
        "stormchaser.v1.0.step.scheduled.purgetestonly".to_string(),
        step_id,
    )
    .await;
    record_task_message(&pool, seq, step_id).await.unwrap();

    // While the step runs, a runner may still need the message redelivered
    purge_all(&pool, &js).await;
    assert!(stream.get_raw_message(seq).await.is_ok());
    assert!(recorded(&pool, seq).await);

    sqlx::query("UPDATE step_instances SET status = 'succeeded' WHERE id = $1")
        .bind(step_id)
        .execute(&pool)
        .await
        .unwrap();
    purge_all(&pool, &js).await;

    let gone = stream.get_raw_message(seq).await.unwrap_err();
    assert_eq!(gone.kind(), RawMessageErrorKind::NoMessageFound);
    assert!(!recorded(&pool, seq).await);
}

#[tokio::test]
async fn a_stale_record_never_erases_some_other_message() {
    let pool = pool().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let js = stormchaser_engine::nats::init_jetstream(&nats)
        .await
        .unwrap();
    let stream = js.get_stream(STREAM_NAME).await.unwrap();

    // A record for an archived step (no row) pointing at a message that is not its task
    let other_step = Uuid::new_v4();
    let seq = publish(
        &js,
        "stormchaser.v1.0.step.scheduled.purgetestonly".to_string(),
        other_step,
    )
    .await;
    record_task_message(&pool, seq, Uuid::new_v4())
        .await
        .unwrap();

    purge_all(&pool, &js).await;

    assert!(
        stream.get_raw_message(seq).await.is_ok(),
        "a message belonging to another step must survive"
    );
    assert!(!recorded(&pool, seq).await);
    stream.delete_message(seq).await.unwrap();
}

#[tokio::test]
async fn the_stream_has_a_maximum_age() {
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let js = stormchaser_engine::nats::init_jetstream(&nats)
        .await
        .unwrap();
    let mut stream = js.get_stream(STREAM_NAME).await.unwrap();
    assert!(!stream.info().await.unwrap().config.max_age.is_zero());
}
