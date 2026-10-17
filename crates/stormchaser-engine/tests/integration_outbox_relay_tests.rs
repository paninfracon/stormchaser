//! The outbox relay publishes an event again when it dies between publishing it and
//! deleting its row. The stream keeps one copy.

use futures::StreamExt;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use std::time::Duration;
use stormchaser_model::events::{EventSource, EventType, WorkflowEventType};
use stormchaser_model::nats::NatsSubject;

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

#[derive(sqlx::FromRow)]
struct Row {
    id: uuid::Uuid,
    subject: String,
    payload: String,
    headers: serde_json::Value,
}

async fn outbox_row(pool: &sqlx::PgPool, subject: &str) -> Option<Row> {
    sqlx::query_as("SELECT id, subject, payload, headers FROM outbox_events WHERE subject = $1")
        .bind(subject)
        .fetch_optional(pool)
        .await
        .unwrap()
}

async fn until_published(pool: &sqlx::PgPool, subject: &str) {
    for _ in 0..50 {
        if outbox_row(pool, subject).await.is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the relay did not publish the event");
}

#[tokio::test]
async fn an_event_the_relay_publishes_twice_is_stored_once() {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = stormchaser_model::nats::connect(nats_url).await.unwrap();
    let js = stormchaser_engine::nats::init_jetstream(&nats_client)
        .await
        .unwrap();

    // A subject of this test's own
    let subject = format!("stormchaser.v1.995.test.outbox.{}", uuid::Uuid::new_v4());
    let consumer = js
        .get_stream("stormchaser")
        .await
        .unwrap()
        .create_consumer(async_nats::jetstream::consumer::pull::Config {
            filter_subject: subject.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    stormchaser_engine::db::outbox::insert_outbox_event(
        &pool,
        NatsSubject::Custom(subject.clone()),
        EventType::Workflow(WorkflowEventType::Running),
        EventSource::Engine,
        serde_json::json!({ "test": true }),
        None,
        None,
    )
    .await
    .unwrap();
    let row = outbox_row(&pool, &subject).await.unwrap();

    let relay =
        stormchaser_engine::workers::start_outbox_relay_worker(pool.clone(), nats_client.clone());
    until_published(&pool, &subject).await;

    // As if the relay had died after publishing, before its deletion was committed
    sqlx::query(
        "INSERT INTO outbox_events (id, subject, payload, headers) VALUES ($1, $2, $3, $4)",
    )
    .bind(row.id)
    .bind(&row.subject)
    .bind(&row.payload)
    .bind(&row.headers)
    .execute(&pool)
    .await
    .unwrap();
    until_published(&pool, &subject).await;
    relay.abort();

    let mut messages = consumer
        .fetch()
        .max_messages(10)
        .expires(Duration::from_secs(2))
        .messages()
        .await
        .unwrap();
    let mut stored = Vec::new();
    while let Some(message) = messages.next().await {
        stored.push(message.unwrap());
    }
    assert_eq!(stored.len(), 1, "the stream kept one copy");
    let msg_id = stored[0]
        .headers
        .as_ref()
        .and_then(|h| h.get("Nats-Msg-Id"))
        .map(|v| v.to_string());
    assert_eq!(msg_id, Some(row.id.to_string()));
}
