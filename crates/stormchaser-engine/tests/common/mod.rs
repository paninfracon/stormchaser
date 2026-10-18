//! Helpers the engine's integration tests share.

// Each test binary uses only some of them
#![allow(dead_code)]

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::env::var;
use std::sync::Arc;
use stormchaser_model::RunId;
use stormchaser_tls::{TlsConfig, TlsReloader};
use tokio::task::JoinHandle;

// Each test binary that consumes events reserves shards of its own, which no engine in
// the tests consumes, so that binaries run in parallel do not take each other's events.
pub const ROUTER_RETRY_SHARDS: [u32; 2] = [997, 998];
pub const OUTBOX_RELAY_SHARD: u32 = 995;

pub async fn setup_db() -> PgPool {
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

pub async fn nats() -> async_nats::Client {
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    stormchaser_model::nats::connect(nats_url).await.unwrap()
}

pub async fn tls_reloader() -> Arc<TlsReloader> {
    Arc::new(TlsReloader::new(TlsConfig::default()).await.unwrap())
}

/// Deletes the run, live or archived, and the events queued for it. Most archived tables
/// go with their run; the step history, test summaries and test cases do not.
pub async fn delete_run(pool: &PgPool, run_id: RunId) {
    for statement in [
        "DELETE FROM archived_step_status_history WHERE step_instance_id IN (SELECT id FROM archived_step_instances WHERE run_id = $1)",
        "DELETE FROM archived_step_test_cases WHERE run_id = $1",
        "DELETE FROM archived_step_test_summaries WHERE run_id = $1",
        "DELETE FROM archived_workflow_runs WHERE id = $1",
        "DELETE FROM workflow_runs WHERE id = $1",
    ] {
        sqlx::query(statement)
            .bind(run_id)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query("DELETE FROM outbox_events WHERE payload LIKE '%' || $1 || '%'")
        .bind(run_id.to_string())
        .execute(pool)
        .await
        .unwrap();
}

/// The engine's outbox relay, stopped when dropped so that it does not outlive its test
pub struct Relay(JoinHandle<()>);

impl Drop for Relay {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub fn start_relay(pool: PgPool, nats: async_nats::Client) -> Relay {
    Relay(stormchaser_engine::workers::start_outbox_relay_worker(
        pool, nats,
    ))
}
