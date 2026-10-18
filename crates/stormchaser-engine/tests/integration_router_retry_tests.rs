//! An event whose handler failed is delivered again only when another try could
//! succeed: the database was unreachable, say, not the run missing.

mod common;

use common::{nats, setup_db, tls_reloader, ROUTER_RETRY_SHARDS};
use futures::StreamExt;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::git_cache::GitCache;
use stormchaser_engine::router::handle_message;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;

/// How long the consumer waits for an acknowledgement before delivering an event again
const ACK_WAIT: Duration = Duration::from_secs(2);

struct Harness {
    nats_client: async_nats::Client,
    consumer:
        async_nats::jetstream::consumer::Consumer<async_nats::jetstream::consumer::pull::Config>,
    subject: String,
    _git_dir: tempfile::TempDir,
    git_cache: Arc<GitCache>,
    tls_reloader: Arc<TlsReloader>,
}

impl Harness {
    /// On a shard of its own, which no engine in the tests consumes
    async fn new(shard: u32) -> Self {
        let nats_client = nats().await;
        let js = stormchaser_engine::nats::init_jetstream(&nats_client)
            .await
            .unwrap();
        let subject = NatsSubject::RunQueued(Some(shard)).as_str().to_string();
        let consumer = js
            .get_stream("stormchaser")
            .await
            .unwrap()
            .create_consumer(async_nats::jetstream::consumer::pull::Config {
                filter_subject: subject.clone(),
                deliver_policy: async_nats::jetstream::consumer::DeliverPolicy::New,
                ack_wait: ACK_WAIT,
                ..Default::default()
            })
            .await
            .unwrap();
        let git_dir = tempfile::tempdir().unwrap();
        Self {
            nats_client,
            consumer,
            subject,
            git_cache: Arc::new(GitCache::new(git_dir.path())),
            _git_dir: git_dir,
            tls_reloader: tls_reloader().await,
        }
    }

    async fn publish(&self, run_id: RunId) {
        let js = async_nats::jetstream::new(self.nats_client.clone());
        js.publish(
            self.subject.clone(),
            json!({ "run_id": run_id }).to_string().into(),
        )
        .await
        .unwrap()
        .await
        .unwrap();
    }

    async fn next(&self, within: Duration) -> Option<async_nats::jetstream::Message> {
        let mut batch = self
            .consumer
            .fetch()
            .max_messages(1)
            .expires(within)
            .messages()
            .await
            .unwrap();
        batch.next().await.map(|m| m.unwrap())
    }

    async fn handle(&self, message: async_nats::jetstream::Message, pool: sqlx::PgPool) {
        let payload: serde_json::Value = serde_json::from_slice(&message.payload).unwrap();
        handle_message(
            &self.subject,
            payload,
            message,
            pool,
            self.git_cache.clone(),
            Arc::new(OpaClient::new(None, None)),
            self.nats_client.clone(),
            self.tls_reloader.clone(),
            Arc::new(None),
        )
        .await;
    }
}

fn run_id_of(message: &async_nats::jetstream::Message) -> String {
    serde_json::from_slice::<serde_json::Value>(&message.payload).unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn an_event_that_failed_on_an_unreachable_database_is_delivered_again() {
    let harness = Harness::new(ROUTER_RETRY_SHARDS[0]).await;
    let run_id = RunId::new_v4();
    harness.publish(run_id).await;

    let first = harness
        .next(Duration::from_secs(5))
        .await
        .expect("delivered");
    assert_eq!(run_id_of(&first), run_id.to_string());

    let pool = setup_db().await;
    pool.close().await;
    harness.handle(first, pool).await;

    let again = harness
        .next(Duration::from_secs(5))
        .await
        .expect("delivered again after the database error");
    assert_eq!(run_id_of(&again), run_id.to_string());
    assert_eq!(again.info().unwrap().delivered, 2);
    again.double_ack().await.unwrap();
}

#[tokio::test]
async fn an_event_for_a_missing_run_is_not_delivered_again() {
    let harness = Harness::new(ROUTER_RETRY_SHARDS[1]).await;
    let run_id = RunId::new_v4();
    harness.publish(run_id).await;

    let first = harness
        .next(Duration::from_secs(5))
        .await
        .expect("delivered");
    harness.handle(first, setup_db().await).await;

    // Twice the ack wait: an event left unacknowledged would be back by then
    assert!(harness.next(ACK_WAIT * 2).await.is_none());
}
