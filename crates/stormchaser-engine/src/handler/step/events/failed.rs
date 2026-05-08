use crate::handler::{archive_workflow, dispatch_pending_steps, fetch_run, fetch_step_instance};
use crate::workflow_machine::{state, WorkflowMachine};
use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::events::WorkflowFailedEvent;
use stormchaser_model::step::StepStatus;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;
use stormchaser_tls::TlsReloader;
use tracing::{error, info};

use crate::handler::step::quota::release_step_quota_for_instance;

use super::helpers::persist_step_test_reports;

#[tracing::instrument(skip(payload, pool, nats_client, tls_reloader), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step failed.
pub async fn handle_step_failed(
    payload: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let run_id_str = payload["run_id"].as_str().context("Missing run_id")?;
    let run_id = uuid::Uuid::parse_str(run_id_str).map(RunId::new)?;
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = uuid::Uuid::parse_str(step_id_str).map(StepInstanceId::new)?;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));
    let error_msg = payload["error"].as_str().unwrap_or("Unknown error");
    let exit_code = payload["exit_code"].as_i64().map(|c| c as i32);

    info!("Step {} (Run {}) failed: {}", step_id, run_id, error_msg);

    let mut tx = pool.begin().await?;

    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
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
    let _ = machine
        .fail(error_msg.to_string(), exit_code, &mut *tx)
        .await?;

    let attributes = [
        opentelemetry::KeyValue::new("step_name", instance.step_name),
        opentelemetry::KeyValue::new("step_type", instance.step_type),
        opentelemetry::KeyValue::new("runner_id", instance.runner_id.unwrap_or_default()),
        opentelemetry::KeyValue::new("error", error_msg.to_string()),
    ];

    crate::STEPS_FAILED.add(1, &attributes);

    if let Some(started_at) = instance.started_at {
        let duration = (Utc::now() - started_at).to_std().unwrap_or_default();
        crate::STEP_DURATION.record(duration.as_secs_f64(), &attributes);
    }

    if let Some(outputs) = payload["outputs"].as_object() {
        for (key, value) in outputs {
            crate::db::upsert_step_output(&mut *tx, step_id, key, value).await?;
        }
    }

    // Persist test reports even on failure
    persist_step_test_reports(&payload, &mut tx, run_id, step_id, &pool).await?;

    let run = fetch_run(run_id, &mut *tx).await?;
    let run_machine = WorkflowMachine::<state::Running>::new_from_run(run.clone());
    let _ = run_machine
        .fail(format!("Step {} failed: {}", step_id, error_msg), &mut *tx)
        .await?;

    let js = async_nats::jetstream::new(nats_client.clone());
    if let Err(e) = stormchaser_model::nats::publish_cloudevent(
        &js,
        "stormchaser.v1.run.failed",
        "workflow_failed",
        "stormchaser-engine",
        serde_json::to_value(WorkflowFailedEvent {
            run_id,
            event_type: "workflow_failed".to_string(),
            timestamp: chrono::Utc::now(),
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
            opentelemetry::KeyValue::new("workflow_name", run.workflow_name),
            opentelemetry::KeyValue::new("initiating_user", run.initiating_user),
            opentelemetry::KeyValue::new("error", format!("Step {} failed", step_id)),
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
