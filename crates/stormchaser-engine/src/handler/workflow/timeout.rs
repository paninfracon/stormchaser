use crate::handler::{archive_workflow, fetch_run};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::workflow::RunStatus;
use stormchaser_tls::TlsReloader;
use tracing::info;
use uuid::Uuid;

#[tracing::instrument(skip(pool, nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handle workflow timeout.
pub async fn handle_workflow_timeout(
    run_id: Uuid,
    pool: PgPool,
    nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    info!("Workflow {} timed out, aborting", run_id);

    let run = fetch_run(run_id, &pool).await?;
    if matches!(
        run.status,
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted
    ) {
        return Ok(());
    }

    // 1. Mark Workflow as Aborted
    let mut machine_run = run.clone();
    machine_run.error = Some("Workflow timed out".to_string());

    match run.status {
        RunStatus::Queued => {
            WorkflowMachine::<state::Queued>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        RunStatus::Resolving => {
            WorkflowMachine::<state::Resolving>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        RunStatus::StartPending => {
            WorkflowMachine::<state::StartPending>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        RunStatus::Running => {
            WorkflowMachine::<state::Running>::new_from_run(machine_run)
                .abort(&mut *pool.acquire().await?)
                .await?;
        }
        _ => {} // Should not happen due to check above
    };

    // 2. Mark all non-terminal steps as failed
    let steps: Vec<stormchaser_model::step::StepInstance> =
        crate::db::get_step_instances_by_run_id(&pool, run_id).await?;

    let mut tx = pool.begin().await?;
    for step in steps {
        match step.status {
            stormchaser_model::step::StepStatus::Pending => {
                crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            stormchaser_model::step::StepStatus::UnpackingSfs => {
                crate::step_machine::StepMachine::<crate::step_machine::state::UnpackingSfs>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            stormchaser_model::step::StepStatus::Running => {
                crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            stormchaser_model::step::StepStatus::PackingSfs => {
                crate::step_machine::StepMachine::<crate::step_machine::state::PackingSfs>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            stormchaser_model::step::StepStatus::WaitingForEvent => {
                crate::step_machine::StepMachine::<crate::step_machine::state::WaitingForEvent>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            _ => {}
        }
    }
    tx.commit().await?;

    // 3. Publish abort event
    let event = serde_json::json!({
        "run_id": run_id,
        "event_type": "workflow_aborted",
        "reason": "timeout",
        "timestamp": Utc::now(),
    });
    let js = async_nats::jetstream::new(nats_client);
    js.publish("stormchaser.run.aborted", event.to_string().into())
        .await?;

    // 4. Archive
    archive_workflow(run_id, pool).await?;

    Ok(())
}
