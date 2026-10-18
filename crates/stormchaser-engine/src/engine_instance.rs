//! Each engine process sends a heartbeat for as long as it runs. The built-in steps it
//! runs itself record its id, so that when its heartbeats stop, the leader can fail
//! the steps it was running instead of leaving them to the run's timeout.

use anyhow::Result;
use sqlx::PgPool;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// This engine process
pub static ENGINE_ID: LazyLock<Uuid> = LazyLock::new(Uuid::new_v4);

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// How long an engine may go without a heartbeat before the steps it runs are lost
pub const LOST_AFTER_SECS: f64 = 60.0;

pub async fn heartbeat(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query(
        "INSERT INTO engine_instances (id, last_heartbeat_at) VALUES ($1, NOW())
         ON CONFLICT (id) DO UPDATE SET last_heartbeat_at = NOW()",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Records this engine's first heartbeat, before it can take any step, and sends the
/// rest from then on.
pub async fn start_heartbeat(pool: PgPool) -> Result<JoinHandle<()>> {
    heartbeat(&pool, *ENGINE_ID).await?;
    Ok(tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = heartbeat(&pool, *ENGINE_ID).await {
                tracing::warn!("Failed to record the engine's heartbeat: {:?}", e);
            }
        }
    }))
}
