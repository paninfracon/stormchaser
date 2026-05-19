use crate::handler::fetch_run;
use crate::handler::StepInstance;
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::Result;
use opentelemetry::KeyValue;
use stormchaser_dsl::ast::Workflow;
use stormchaser_model::events::{
    EventSource, EventType, WorkflowCompletedEvent, WorkflowEventType,
};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::step::StepStatus;
use stormchaser_model::RunId;
use tracing::{error, info};

pub async fn check_workflow_completion(
    tx: &mut sqlx::PgConnection,
    run_id: RunId,
    workflow: &Workflow,
    nats_client: async_nats::Client,
) -> Result<bool> {
    let all_steps_final: Vec<StepInstance> =
        crate::db::get_step_instances_by_run_id(&mut *tx, run_id).await?;

    let all_dsl_steps_done = workflow.steps.iter().all(|dsl_step| {
        let step_instances: Vec<&StepInstance> = all_steps_final
            .iter()
            .filter(|s| s.step_name == dsl_step.name)
            .collect();

        let is_done = !step_instances.is_empty()
            && step_instances
                .iter()
                .all(|s| s.status == StepStatus::Succeeded || s.status == StepStatus::Skipped);

        tracing::info!(
            "Step {} done? {} (instances: {})",
            dsl_step.name,
            is_done,
            step_instances.len()
        );
        is_done
    });

    tracing::info!("All steps done? {}", all_dsl_steps_done);

    if all_dsl_steps_done {
        let run = fetch_run(run_id, &mut *tx).await?;
        let machine = WorkflowMachine::<state::Running>::new_from_run(run.clone());
        let _ = machine.succeed(&mut *tx).await?;

        let js = async_nats::jetstream::new(nats_client.clone());
        use stormchaser_model::nats::NatsSubject;
        if let Err(e) = publish_cloudevent(
            &js,
            NatsSubject::RunCompleted(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
            EventType::Workflow(WorkflowEventType::Completed),
            EventSource::Engine,
            serde_json::to_value(WorkflowCompletedEvent {
                run_id,
                event_type: EventType::Workflow(WorkflowEventType::Completed),
                timestamp: chrono::Utc::now(),
                status: stormchaser_model::workflow::RunStatus::Succeeded,
            })
            .unwrap(),
            None,
            None,
        )
        .await
        {
            error!(
                "Failed to publish workflow completed event for {}: {:?}",
                run_id, e
            );
        }

        crate::RUNS_COMPLETED.add(
            1,
            &[
                KeyValue::new("workflow_name", run.workflow_name),
                KeyValue::new("initiating_user", run.initiating_user),
            ],
        );

        info!(
            "Workflow run {} completed successfully, archiving...",
            run_id
        );
        return Ok(true); // Signal to caller to archive after committing tx
    }

    Ok(false)
}
