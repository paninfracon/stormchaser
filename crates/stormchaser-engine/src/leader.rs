//! One engine replica at a time runs the background workers: liveness, timeouts,
//! resolver crash recovery, the outbox relay, the task message purge and dispatch
//! recovery.
//!
//! The replicas compete for a Postgres advisory lock. The one that takes it holds it
//! on a connection of its own, outside the pool, and runs the workers for as long as
//! that connection answers. When it stops answering, or the replica dies, Postgres
//! releases the lock and another replica takes over.

use anyhow::Result;
use sqlx::pool::PoolConnection;
use sqlx::{PgConnection, PgPool, Postgres};
use std::sync::Arc;
use std::time::Duration;
use stormchaser_tls::TlsReloader;
use tokio::task::JoinHandle;

/// The advisory lock the replicas compete for ("stormchr" in ASCII)
pub const LEADER_LOCK: i64 = 0x73746f726d636872;

/// How often a replica that is not leading tries to, and the leader checks it still is
const CHECK_INTERVAL: Duration = Duration::from_secs(5);

/// How long the leader's connection may take to answer before the leader steps down
const HOLD_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Takes the advisory lock `key`, if no other session holds it, on a connection that
/// leaves the pool with it. Dropping the connection closes it, which releases the lock,
/// and so does the server once the holder has gone quiet.
pub async fn try_lock(pool: &PgPool, key: i64) -> Result<Option<PgConnection>> {
    let mut conn: PoolConnection<Postgres> = pool.acquire().await?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(key)
        .fetch_one(&mut *conn)
        .await?;
    if !locked {
        // A connection that took no lock goes back to the pool
        return Ok(None);
    }
    let mut conn = conn.detach();
    end_session_when_holder_vanishes(&mut conn).await;
    Ok(Some(conn))
}

/// Has Postgres end the lock's session, and so release the lock, soon after its holder
/// stops speaking to it. A replica that crashes has its socket closed for it, but one
/// whose host dies or is cut off from the network does not: without this, the server
/// would hold the lock until its TCP keepalive gave up, two hours by default, and no
/// replica would run the background workers meanwhile.
///
/// The leader checks its connection every [`CHECK_INTERVAL`], so a session idle for 30
/// seconds belongs to a leader that has gone. `idle_session_timeout` needs PostgreSQL
/// 14; the keepalive settings cover older servers, and a server that refuses either
/// keeps its defaults.
async fn end_session_when_holder_vanishes(conn: &mut PgConnection) {
    for setting in [
        "SET idle_session_timeout = '30s'",
        "SET tcp_keepalives_idle = 10",
        "SET tcp_keepalives_interval = 5",
        "SET tcp_keepalives_count = 3",
    ] {
        if let Err(e) = sqlx::query(setting).execute(&mut *conn).await {
            tracing::warn!(
                "Could not configure the leader lock's session ({}): {}",
                setting,
                e
            );
        }
    }
}

/// Whether the connection holding the lock still answers in time
async fn still_holds(conn: &mut PgConnection) -> bool {
    let check = sqlx::query("SELECT 1").execute(&mut *conn);
    matches!(
        tokio::time::timeout(HOLD_CHECK_TIMEOUT, check).await,
        Ok(Ok(_))
    )
}

fn start_workers(
    pool: &PgPool,
    nats_client: &async_nats::Client,
    tls_reloader: &Arc<TlsReloader>,
) -> Vec<JoinHandle<()>> {
    use crate::workers::*;
    vec![
        start_liveness_worker(pool.clone(), nats_client.clone()),
        start_timeout_worker(pool.clone(), nats_client.clone(), tls_reloader.clone()),
        start_resolver_crash_recovery_worker(pool.clone(), nats_client.clone()),
        start_outbox_relay_worker(pool.clone(), nats_client.clone()),
        start_task_message_purge_worker(pool.clone(), nats_client.clone()),
        start_dispatch_recovery_worker(pool.clone(), nats_client.clone(), tls_reloader.clone()),
        start_retry_worker(pool.clone(), nats_client.clone(), tls_reloader.clone()),
    ]
    .into_iter()
    .chain(start_retention_worker(pool.clone()))
    .collect()
}

/// Competes for leadership for as long as the engine runs, running the background
/// workers while this replica leads.
pub fn start_leader_election(
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            match try_lock(&pool, LEADER_LOCK).await {
                Ok(Some(mut conn)) => {
                    tracing::info!("This engine now runs the background workers");
                    let workers = start_workers(&pool, &nats_client, &tls_reloader);
                    loop {
                        tokio::time::sleep(CHECK_INTERVAL).await;
                        if !still_holds(&mut conn).await {
                            break;
                        }
                    }
                    tracing::warn!(
                        "Lost the connection holding the leader lock; stopping the background workers"
                    );
                    for worker in workers {
                        worker.abort();
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::warn!("Could not compete for leadership: {:#}", e),
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    })
}
