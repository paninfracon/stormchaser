use crate::handler::{archive_workflow, dispatch_pending_steps, fetch_run, fetch_step_instance};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::Result;
use chrono::Utc;
use opentelemetry::KeyValue;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::events::WorkflowFailedEvent;
use stormchaser_model::events::{EventSource, EventType, WorkflowEventType};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::step::StepStatus;
use stormchaser_tls::TlsReloader;
use tracing::{error, info};

use crate::handler::step::quota::release_step_quota_for_instance;

use super::helpers::persist_step_test_reports;

#[tracing::instrument(skip(event, pool, nats_client, tls_reloader), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step failed.
pub async fn handle_step_failed(
    event: stormchaser_model::events::StepFailedEvent,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let run_id = event.run_id;
    let step_id = event.step_id;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let error_msg = &event.error;
    let exit_code = event.exit_code;

    info!("Step {} (Run {}) failed: {}", step_id, run_id, error_msg);

    let mut tx = pool.begin().await?;

    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
        return Ok(());
    }

    let workflow_run = fetch_run(run_id, &mut *tx).await?;
    if event.fencing_token == 0 {
        tracing::debug!(
            "Failure event for run {} step {} arrived without fencing token; allowing for compatibility",
            run_id, step_id
        );
    }
    if event.fencing_token > 0 && event.fencing_token < workflow_run.fencing_token {
        tracing::warn!(
            "Rejecting stale failure event for run {} step {} due to fencing token mismatch (event: {}, run: {})",
            run_id, step_id, event.fencing_token, workflow_run.fencing_token
        );
        return Ok(());
    }

    let instance = fetch_step_instance(step_id, &mut *tx).await?;

    // Ensure we don't process duplicate completion events
    if instance.status == StepStatus::Succeeded || instance.status == StepStatus::Failed {
        return Ok(());
    }

    let _ = release_step_quota_for_instance(&mut *tx, run_id, step_id).await;

    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance.clone(),
        );

    if error_msg == "lost_zombie" {
        let _ = machine.zombify(&mut *tx).await?;
    } else {
        let _ = machine
            .fail(error_msg.to_string(), exit_code, &mut *tx)
            .await?;
    }

    let attributes = [
        KeyValue::new("step_name", instance.step_name),
        KeyValue::new("step_type", instance.step_type),
        KeyValue::new("runner_id", instance.runner_id.unwrap_or_default()),
        KeyValue::new("error", error_msg.to_string()),
    ];

    crate::STEPS_FAILED.add(1, &attributes);

    if let Some(started_at) = instance.started_at {
        let duration = (Utc::now() - started_at).to_std().unwrap_or_default();
        crate::STEP_DURATION.record(duration.as_secs_f64(), &attributes);
    }

    if let Some(outputs) = event
        .outputs
        .as_ref()
        .map(|m| {
            m.clone()
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>()
        })
        .as_ref()
    {
        for (key, value) in outputs {
            crate::db::upsert_step_output(&mut *tx, step_id, key, value).await?;
        }
    }

    // Persist test reports even on failure
    persist_step_test_reports(event.test_reports.as_ref(), &mut tx, run_id, step_id, &pool).await?;

    let run = fetch_run(run_id, &mut *tx).await?;
    let run_machine = WorkflowMachine::<state::Running>::new_from_run(run.clone());
    let _ = run_machine
        .fail(format!("Step {} failed: {}", step_id, error_msg), &mut *tx)
        .await?;

    let js = async_nats::jetstream::new(nats_client.clone());
    use stormchaser_model::nats::NatsSubject;
    if let Err(e) = publish_cloudevent(
        &js,
        NatsSubject::RunFailed(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Workflow(WorkflowEventType::Failed),
        EventSource::Engine,
        serde_json::to_value(WorkflowFailedEvent {
            run_id,
            event_type: EventType::Workflow(WorkflowEventType::Failed),
            timestamp: chrono::Utc::now(),
            status: stormchaser_model::workflow::RunStatus::Failed,
        })
        .unwrap(),
        None,
        None,
    )
    .await
    {
        error!(
            "Failed to publish workflow failed event for {}: {:?}",
            run_id, e
        );
    }

    crate::RUNS_FAILED.add(
        1,
        &[
            KeyValue::new("workflow_name", run.workflow_name),
            KeyValue::new("initiating_user", run.initiating_user),
            KeyValue::new("error", format!("Step {} failed", step_id)),
        ],
    );

    tx.commit().await?;
    info!("Workflow run {} failed, archiving...", run_id);
    archive_workflow(run_id, pool.clone()).await?;

    if let Err(e) = dispatch_pending_steps(run_id, pool, nats_client, tls_reloader).await {
        error!(
            "Failed to dispatch pending steps after failure for run {}: {:?}",
            run_id, e
        );
    }

    Ok(())
}
