use crate::handler::{archive_workflow, fetch_run};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::Result;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::events::WorkflowAbortedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;
use tracing::info;

#[tracing::instrument(skip(pool, _nats_client, _tls_reloader), fields(run_id = %run_id))]
/// Handle workflow timeout.
pub async fn handle_workflow_timeout(
    run_id: RunId,
    pool: PgPool,
    _nats_client: async_nats::Client,
    _tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    info!("Workflow {} timed out, aborting", run_id);

    // The run and its steps end together, under the run's lock
    let mut tx = pool.begin().await?;
    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
        return Ok(());
    }
    let mut run = fetch_run(run_id, &mut *tx).await?;
    if matches!(
        run.status,
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted
    ) {
        return Ok(());
    }

    // 1. Mark Workflow as Aborted
    run.error = Some("Workflow timed out".to_string());
    match run.status {
        RunStatus::Queued => {
            WorkflowMachine::<state::Queued>::load(run)?
                .abort(&mut tx)
                .await?;
        }
        RunStatus::Resolving => {
            WorkflowMachine::<state::Resolving>::load(run)?
                .abort(&mut tx)
                .await?;
        }
        RunStatus::StartPending => {
            WorkflowMachine::<state::StartPending>::load(run)?
                .abort(&mut tx)
                .await?;
        }
        RunStatus::Running => {
            WorkflowMachine::<state::Running>::load(run)?
                .abort(&mut tx)
                .await?;
        }
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => unreachable!(),
    };

    // 2. Mark all non-terminal steps as failed
    let steps: Vec<StepInstance> =
        crate::db::get_step_instances_by_run_id(&mut *tx, run_id).await?;
    for step in steps {
        use crate::step_machine::{state as step_state, StepMachine};
        if step.status == StepStatus::Pending {
            StepMachine::<step_state::Pending>::load(step)?
                .fail("Workflow timed out".to_string(), None, &mut tx)
                .await?;
        } else if let Ok(machine) = StepMachine::<step_state::Unfinished>::load(step) {
            machine
                .fail("Workflow timed out".to_string(), None, &mut tx)
                .await?;
        }
    }
    // 3. Enqueue abort event
    let event = WorkflowAbortedEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::Aborted),
        timestamp: chrono::Utc::now(),
        status: stormchaser_model::workflow::RunStatus::Aborted,
    };
    use stormchaser_model::nats::NatsSubject;
    crate::db::outbox::insert_outbox_event(
        &mut *tx,
        NatsSubject::RunAborted(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Workflow(WorkflowEventType::Aborted),
        EventSource::System,
        serde_json::to_value(event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;

    tx.commit().await?;
    // 4. Archive
    archive_workflow(run_id, pool).await?;

    Ok(())
}
