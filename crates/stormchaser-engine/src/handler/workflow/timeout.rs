use crate::handler::{archive_workflow, fetch_run};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::events::WorkflowAbortedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;
use tracing::info;

#[tracing::instrument(skip(pool, nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handle workflow timeout.
pub async fn handle_workflow_timeout(
    run_id: RunId,
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
    let steps: Vec<StepInstance> = crate::db::get_step_instances_by_run_id(&pool, run_id).await?;

    let mut tx = pool.begin().await?;
    for step in steps {
        match step.status {
            StepStatus::Pending => {
                crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            StepStatus::UnpackingSfs => {
                crate::step_machine::StepMachine::<crate::step_machine::state::UnpackingSfs>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            StepStatus::Running => {
                crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            StepStatus::PackingSfs => {
                crate::step_machine::StepMachine::<crate::step_machine::state::PackingSfs>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            StepStatus::WaitingForEvent => {
                crate::step_machine::StepMachine::<crate::step_machine::state::WaitingForEvent>::from_instance(step)
                    .fail("Workflow timed out".to_string(), None, &mut *tx)
                    .await?;
            }
            _ => {}
        }
    }
    tx.commit().await?;

    // 3. Publish abort event
    let event = WorkflowAbortedEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::Aborted),
        timestamp: Utc::now(),
    };
    let js = async_nats::jetstream::new(nats_client);
    use stormchaser_model::nats::NatsSubject;
    stormchaser_model::nats::publish_cloudevent(
        &js,
        NatsSubject::RunAborted(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Workflow(WorkflowEventType::Aborted),
        EventSource::System,
        serde_json::to_value(event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;

    // 4. Archive
    archive_workflow(run_id, pool).await?;

    Ok(())
}
