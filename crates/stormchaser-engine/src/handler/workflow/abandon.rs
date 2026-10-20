//! Runs whose starting event the engine gives up on.
//!
//! An event that keeps failing, transiently or by crashing its handler, is dropped in
//! the end. Its run would then stay queued, or with a direct run not exist at all,
//! though whoever started it was given its id. It is recorded as failed instead.

use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::RunId;
use tracing::{error, info};

use super::direct::Rejected;

/// Records the run that `payload`, an event on `subject`, was to start or move on as
/// failed with `reason`. A run that has already gone further is left alone. Errors are
/// logged: this is the last thing done for the event.
pub async fn abandon_run_event(subject: &str, payload: &Value, pool: &PgPool, reason: &str) {
    if let Err(e) = abandon(subject, payload, pool, reason).await {
        error!(
            "Could not record run {} as failed after giving up on it: {:#}",
            payload["run_id"].as_str().unwrap_or("unknown"),
            e
        );
    }
}

async fn abandon(subject: &str, payload: &Value, pool: &PgPool, reason: &str) -> Result<()> {
    let Some(run_id) = payload["run_id"]
        .as_str()
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .map(RunId::new)
    else {
        return Ok(());
    };
    let reason = format!("The engine gave up starting this run: {}", reason);
    match subject {
        "stormchaser.v1.run.queued" | "stormchaser.v1.run.start_pending" => {
            if fail_unstarted_run(pool, run_id, &reason).await? {
                info!("Run {} failed: {}", run_id, reason);
                crate::handler::archive_workflow(run_id, pool.clone()).await?;
            }
        }
        "stormchaser.v1.run.direct" => {
            if crate::db::workflow_run_exists(pool, run_id).await? {
                return Ok(());
            }
            let Some(source) = payload["dsl"].as_str() else {
                return Ok(());
            };
            info!("Direct run {} failed: {}", run_id, reason);
            Rejected::new(
                run_id,
                None,
                payload["initiating_user"].as_str().unwrap_or("system"),
                source,
                &payload["inputs"],
            )
            .record_failed(pool, &reason)
            .await?;
        }
        _ => {}
    }
    Ok(())
}

/// Fails a run that is still queued or start_pending. Whether it was.
async fn fail_unstarted_run(pool: &PgPool, run_id: RunId, reason: &str) -> Result<bool> {
    use crate::workflow_machine::{state, WorkflowMachine};
    use stormchaser_model::workflow::RunStatus;

    let mut tx = pool.begin().await?;
    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let run = crate::handler::fetch_run(run_id, &mut *tx).await?;
    match run.status {
        RunStatus::Queued => {
            WorkflowMachine::<state::Queued>::load(run)?
                .fail(reason.to_string(), &mut tx)
                .await?;
        }
        RunStatus::StartPending => {
            WorkflowMachine::<state::StartPending>::load(run)?
                .fail(reason.to_string(), &mut tx)
                .await?;
        }
        _ => return Ok(false),
    }
    tx.commit().await?;
    Ok(true)
}
