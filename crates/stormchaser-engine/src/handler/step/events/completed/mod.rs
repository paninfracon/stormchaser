#![allow(clippy::explicit_auto_deref)]
use crate::handler::StepInstance;
use crate::handler::{archive_workflow, fetch_run_context, fetch_step_instance};
use anyhow::{Context, Result};
use chrono::Utc;
use opentelemetry::KeyValue;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_dsl::ast::{Step, Workflow};
use stormchaser_model::workflow::RunContext;
use stormchaser_model::{LogBackend, RunId};
use stormchaser_tls::TlsReloader;
use tracing::{debug, info};

use crate::handler::step::dispatch::find_step;
use crate::handler::step::quota::release_step_quota_for_instance;

use super::helpers::{
    completion_authorized, is_terminal_step_status, persist_step_test_reports,
    reporter_is_assigned_runner, step_belongs_to_run,
};

mod artifacts;
mod logs;
mod outputs;
mod process;
mod storage;
mod workflow;

#[tracing::instrument(skip(event, pool, nats_client, log_backend, tls_reloader), fields(run_id = tracing::field::Empty, step_id = tracing::field::Empty))]
/// Handle step completed.
pub async fn handle_step_completed(
    event: stormchaser_model::events::StepCompletedEvent,
    pool: PgPool,
    nats_client: async_nats::Client,
    log_backend: Arc<Option<LogBackend>>,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let run_id = event.run_id;
    let step_id = event.step_id;

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("step_id", tracing::field::display(step_id));

    info!("Step {} (Run {}) completed successfully", step_id, run_id);

    let mut tx = pool.begin().await?;

    // Lock the workflow run to serialize state evaluations and prevent parallel join race conditions
    if crate::db::lock_workflow_run(&mut *tx, run_id)
        .await?
        .is_none()
    {
        debug!(
            "Workflow run {} already archived or missing, skipping handler",
            run_id
        );
        return Ok(());
    }

    let workflow_run = crate::handler::fetch_run(run_id, &mut *tx).await?;
    if event.fencing_token == 0 {
        debug!(
            "Completion event for run {} step {} arrived without fencing token; allowing for compatibility",
            run_id, step_id
        );
    }
    if event.fencing_token > 0 && event.fencing_token < workflow_run.fencing_token {
        tracing::warn!(
            "Rejecting stale completion event for run {} step {} due to fencing token mismatch (event: {}, run: {})",
            run_id, step_id, event.fencing_token, workflow_run.fencing_token
        );
        return Ok(());
    }

    // 1. Update StepInstance status
    let instance = fetch_step_instance(step_id, &mut *tx).await?;
    if !step_belongs_to_run(&instance, run_id) {
        return Ok(());
    }

    // A duplicate, or this event delivered again because the archive that followed it
    // failed: the run may still need archiving
    if is_terminal_step_status(&instance.status) {
        drop(tx);
        return archive_workflow(run_id, pool.clone()).await;
    }

    if !completion_authorized(&mut tx, &instance, event.completion_token.as_deref()).await {
        return Ok(());
    }
    if !reporter_is_assigned_runner(&mut tx, &instance, event.runner_id.as_deref()).await {
        return Ok(());
    }

    let _ = release_step_quota_for_instance(&mut *tx, run_id, step_id).await;

    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
            instance.clone(),
        );
    let _ = machine.succeed(&mut *tx).await?;

    let attributes = [
        KeyValue::new("step_name", instance.step_name),
        KeyValue::new("step_type", instance.step_type),
        KeyValue::new("runner_id", instance.runner_id.unwrap_or_default()),
    ];

    crate::STEPS_COMPLETED.add(1, &attributes);

    if let Some(started_at) = instance.started_at {
        let duration = (Utc::now() - started_at).to_std().unwrap_or_default();
        crate::STEP_DURATION.record(duration.as_secs_f64(), &attributes);
    }

    // 2. Fetch ALL steps again to ensure we have the latest status for everyone
    let all_steps: Vec<StepInstance> =
        crate::db::get_step_instances_by_run_id(&mut *tx, run_id).await?;

    let context = fetch_run_context(run_id, &mut *tx).await?;
    let workflow_ast: Workflow = serde_json::from_value(context.workflow_definition.clone())
        .context("Failed to parse workflow definition from DB")?;

    let current_step_instance = all_steps
        .iter()
        .find(|s| s.id.into_inner() == step_id.into_inner())
        .context("Completed step not found in DB")?;

    tracing::info!(
        "Looking for step_name: '{}'",
        current_step_instance.step_name
    );
    for step in &workflow_ast.steps {
        tracing::info!("Available step in ast: '{}'", step.name);
    }

    let mut dsl_step = find_step(&workflow_ast.steps, &current_step_instance.step_name).cloned();
    tracing::info!("Found dsl_step? {}", dsl_step.is_some());

    let outputs_map = event.outputs.as_ref().map(|m| {
        m.clone()
            .into_iter()
            .collect::<serde_json::Map<String, serde_json::Value>>()
    });
    outputs::persist_step_outputs(&mut *tx, step_id, outputs_map.as_ref(), dsl_step.as_ref())
        .await?;

    let storage_hashes_map = event.storage_hashes.as_ref().map(|m| {
        m.clone()
            .into_iter()
            .collect::<serde_json::Map<String, serde_json::Value>>()
    });
    storage::persist_storage_hashes(
        &mut *tx,
        run_id.into_inner(),
        storage_hashes_map.as_ref(),
        &workflow_ast,
    )
    .await?;

    let artifacts_map = event.artifacts.as_ref().map(|m| {
        m.clone()
            .into_iter()
            .collect::<serde_json::Map<String, serde_json::Value>>()
    });
    artifacts::persist_artifact_metadata(
        &mut *tx,
        run_id,
        step_id,
        artifacts_map.as_ref(),
        &workflow_ast,
        &crate::handler::step::dispatch::published_artifacts(
            &workflow_ast,
            &current_step_instance.step_name,
        ),
    )
    .await?;

    let declared_reports = dsl_step
        .as_ref()
        .map(|step| step.reports.as_slice())
        .unwrap_or_default();
    persist_step_test_reports(
        event.test_reports.as_ref(),
        declared_reports,
        &mut tx,
        run_id,
        step_id,
        &pool,
    )
    .await?;

    if let Some(step) = &mut dsl_step {
        outputs::setup_terraform_outputs(step);
    }

    if let (Some(step), Some(backend)) = (&dsl_step, &*log_backend) {
        logs::scrape_outputs_from_logs(
            &mut *tx,
            step_id,
            step,
            backend,
            current_step_instance.started_at,
            current_step_instance.finished_at,
        )
        .await?;
    }

    let finished = advance_run(
        &mut *tx,
        run_id,
        dsl_step.as_ref(),
        &all_steps,
        &context,
        &workflow_ast,
        nats_client.clone(),
        pool.clone(),
        tls_reloader.clone(),
    )
    .await?;

    tx.commit().await?;

    after_advance(finished, run_id, pool, nats_client, tls_reloader).await
}

/// Moves a run on after one of its steps finished in a way that lets it continue:
/// schedules what waited on the step and, once every step is done, completes the run.
/// Returns whether the run finished, for [`after_advance`] once `tx` commits.
#[allow(clippy::too_many_arguments)]
pub(super) async fn advance_run(
    tx: &mut sqlx::PgConnection,
    run_id: RunId,
    dsl_step: Option<&Step>,
    all_steps: &[StepInstance],
    context: &RunContext,
    workflow_ast: &Workflow,
    nats_client: async_nats::Client,
    pool: PgPool,
    tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    if let Some(step) = dsl_step {
        let step_done = process::process_step_completion(
            step,
            all_steps,
            run_id,
            &mut *tx,
            nats_client.clone(),
            pool,
            tls_reloader,
            context.inputs.clone(),
            context.secrets.clone(),
            workflow_ast,
        )
        .await?;
        if !step_done {
            return Ok(false);
        }
    }
    workflow::check_workflow_completion(&mut *tx, run_id, workflow_ast, nats_client).await
}

/// Archives a run that [`advance_run`] finished, or dispatches the steps it scheduled
pub(super) async fn after_advance(
    finished: bool,
    run_id: RunId,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    if finished {
        return archive_workflow(run_id, pool).await;
    }
    if let Err(e) =
        crate::handler::dispatch_pending_steps(run_id, pool, nats_client, tls_reloader).await
    {
        tracing::error!(
            "Failed to dispatch pending steps for run {}: {:?}",
            run_id,
            e
        );
    }
    Ok(())
}
