#![allow(clippy::explicit_auto_deref)]
use serde_json::Value;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use stormchaser_model::StepInstance;
use stormchaser_model::StepInstanceId;
/// Module for integrations.
pub mod integrations;
/// Module for runner.
pub mod runner;
/// Module for step.
pub mod step;
/// Module for workflow.
pub mod workflow;

pub use integrations::*;
pub use runner::*;
pub use step::*;
pub use workflow::*;

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::workflow::{RunContext, RunQuotas, WorkflowRun};
use stormchaser_tls::TlsReloader;
use tracing::{debug, error, info, warn};

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
/// Fetch outputs.
pub async fn fetch_outputs(
    run_id: RunId,
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
) -> Result<Value> {
    let rows: Vec<(String, String, Value)> =
        crate::db::get_step_outputs_for_run(executor, run_id).await?;

    let mut steps_obj = serde_json::Map::new();
    for (step_name, key, value) in rows {
        let step_entry = steps_obj
            .entry(step_name)
            .or_insert(serde_json::json!({"outputs": {}}));
        if let Some(outputs) = step_entry
            .as_object_mut()
            .and_then(|o| o.get_mut("outputs"))
            .and_then(|o| o.as_object_mut())
        {
            outputs.insert(key, value);
        }
    }

    Ok(Value::Object(steps_obj))
}

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
/// Fetch run.
pub async fn fetch_run<'a, E>(run_id: RunId, executor: E) -> Result<WorkflowRun>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_workflow_run_by_id(executor, run_id)
        .await
        .map(|v: WorkflowRun| v)
        .with_context(|| format!("Failed to fetch workflow run for {}", run_id))
}

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
/// Fetch run context.
pub async fn fetch_run_context<'a, E>(run_id: RunId, executor: E) -> Result<RunContext>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_run_context_by_id(executor, run_id)
        .await
        .map(|v: RunContext| v)
        .with_context(|| format!("Failed to fetch run context for {}", run_id))
}

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
/// Fetch quotas.
pub async fn fetch_quotas<'a, E>(run_id: RunId, executor: E) -> Result<RunQuotas>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_run_quota_by_id(executor, run_id)
        .await
        .map(|v: RunQuotas| v)
        .with_context(|| format!("Failed to fetch run quotas for {}", run_id))
}

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
/// Fetch inputs.
pub async fn fetch_inputs<'a, E>(run_id: RunId, executor: E) -> Result<Value>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    let row: (Value,) = crate::db::get_run_inputs_by_id(executor, run_id)
        .await
        .with_context(|| format!("Failed to fetch inputs for {}", run_id))?;
    Ok(row.0)
}

#[tracing::instrument(skip(executor), fields(step_id = %step_id))]
/// Fetch step instance.
pub async fn fetch_step_instance<'a, E>(
    step_id: StepInstanceId,
    executor: E,
) -> Result<StepInstance>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_step_instance_by_id(executor, step_id)
        .await?
        .context("Step instance not found")
}

#[tracing::instrument(skip(pool, nats_client, tls_reloader), fields(run_id = %run_id))]
/// Dispatch pending steps.
pub async fn dispatch_pending_steps(
    run_id: RunId,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    let quotas = fetch_quotas(run_id, &pool).await?;
    let in_flight: i64 = crate::db::count_in_flight_steps_for_run(&pool, run_id).await?;

    debug!(
        "dispatch_pending_steps: {} steps in flight, quotas.max_concurrency = {}",
        in_flight, quotas.max_concurrency
    );

    // A shortcut; claim_for_run is what holds the run to its limit
    if in_flight >= quotas.max_concurrency as i64 {
        debug!("Run {}: Max concurrency {} reached", run_id, in_flight);
        return Ok(());
    }

    let max_cpu = crate::resource_utils::parse_cpu(&quotas.max_cpu).unwrap_or(0.0);
    let max_mem = crate::resource_utils::parse_memory(&quotas.max_memory).unwrap_or(0);

    // 2. Fetch and atomically claim Pending steps (sets status to Initializing,
    //    excluding Approval/Wait which are handled via events). Using FOR UPDATE SKIP LOCKED
    //    ensures that concurrent engine processes never double-dispatch the same step.
    let claimed_steps =
        crate::step_machine::bulk::claim_for_run(&pool, run_id, quotas.max_concurrency as i64)
            .await?;

    for machine in claimed_steps {
        let step = machine.instance().clone();
        info!("Run {}: Evaluating queued step {}", run_id, step.step_name);
        // We need to fetch the resolved spec and params for this step
        let inst_data: (Value, Value) = crate::db::get_step_spec_and_params(&pool, step.id).await?;

        // 3. Enforce CPU and Memory quotas before dispatching
        let req =
            crate::resource_utils::get_step_resource_requirements(&step.step_type, &inst_data.0);

        // A step that asks for more than its run may use in all can never be dispatched:
        // waiting for capacity would leave it pending until the run times out.
        if (max_cpu > 0.0 && req.cpu > max_cpu) || (max_mem > 0 && req.memory > max_mem) {
            let error = format!(
                "Step {} asks for {} CPU and {} bytes of memory, more than its run may use in all ({} CPU, {} memory). Raise the workflow's quotas or lower the step's request.",
                step.step_name, req.cpu, req.memory, quotas.max_cpu, quotas.max_memory
            );
            warn!("Run {}: {}", run_id, error);
            crate::handler::integrations::utils::report_step_failed(
                &pool, run_id, step.id, 0, error,
            )
            .await?;
            continue;
        }

        let can_dispatch = if max_cpu > 0.0 || max_mem > 0 {
            let mut conn = pool.acquire().await?;
            crate::db::claim_step_quota(
                &mut *conn, run_id, step.id, req.cpu, req.memory, max_cpu, max_mem,
            )
            .await?
        } else {
            true // If no quota configured, allow dispatch
        };

        if !can_dispatch {
            debug!(
                "Run {}: Insufficient CPU/Memory quota to dispatch step {}",
                run_id, step.step_name
            );
            // Put back to pending, to be dispatched once the run has room. Nothing has
            // been published, so it cannot be running anywhere.
            requeue_or_fail(machine, "the run had no room for it", &pool).await?;
            continue; // Try next step, maybe it's smaller and fits
        }

        info!("Run {}: Dispatching queued step {}", run_id, step.step_name);

        // Dispatch can fail, but we'll assume success for the quota claim.
        // If it fails, the step state handles it, and we might leak quota until the step completes/fails.
        // Ideally we'd release it on immediate dispatch failure, but we'll stick to the completion hooks for now.
        if let Err(e) = dispatch_step_instance(
            run_id,
            step.id,
            &step.step_name,
            &step.step_type,
            &inst_data.0,
            &inst_data.1,
            nats_client.clone(),
            pool.clone(),
            tls_reloader.clone(),
        )
        .await
        {
            error!("Failed to dispatch step {}: {:?}", step.id, e);
            if max_cpu > 0.0 || max_mem > 0 {
                let mut conn = pool.acquire().await?;
                let _ =
                    crate::db::release_step_quota(&mut *conn, run_id, step.id, req.cpu, req.memory)
                        .await;
            }
            requeue_or_fail(machine, &format!("dispatching it failed: {e:#}"), &pool).await?;
        }
    }

    Ok(())
}

/// Puts a claimed step back to pending if its task was never published. One whose task
/// may have been published is failed instead: putting it back could run it twice.
pub async fn requeue_or_fail(
    machine: crate::step_machine::StepMachine<crate::step_machine::state::Initializing>,
    why: &str,
    pool: &PgPool,
) -> Result<()> {
    use crate::step_machine::transitions::initializing::Requeue;
    let step_id = machine.instance().id;
    let run_id = machine.instance().run_id;
    let mut conn = pool.acquire().await?;
    // Built from the stored row: the dispatch attempt may have changed it
    let stored = fetch_step_instance(step_id, &mut *conn).await?;
    let Ok(machine) =
        crate::step_machine::StepMachine::<crate::step_machine::state::Initializing>::load(stored)
    else {
        // A runner has already reported on it; its own events carry it on
        return Ok(());
    };
    match machine.requeue(&mut conn).await? {
        Requeue::Requeued(_) => {
            debug!("Run {}: step {} is pending again: {}", run_id, step_id, why);
        }
        Requeue::HandedOver(_) => {
            warn!(
                "Run {}: step {} may have reached a runner and {}; failing it rather than risk running it twice",
                run_id, step_id, why
            );
            // Its task was handed over, so the token that would let the engine's report
            // through was cleared, and no runner is recorded to report it. The engine
            // takes it back, which also turns away any report from a runner that got it.
            crate::db::steps::take_step_back_from_runners(&mut *conn, step_id).await?;
            crate::handler::integrations::utils::report_step_failed(
                pool,
                run_id,
                step_id,
                0,
                format!("The step's dispatch may have reached a runner, and {why}"),
            )
            .await?;
        }
    }
    Ok(())
}

#[tracing::instrument(skip(pool), fields(run_id = %run_id))]
/// Archive workflow.
pub async fn archive_workflow(run_id: RunId, pool: PgPool) -> Result<()> {
    let mut tx = pool.begin().await?;

    // Lock the workflow run to serialize archiving
    let _ = crate::db::lock_workflow_run(&mut *tx, run_id).await?;

    // Check if it's already archived or if it's terminal
    let run: Option<(RunStatus,)> = crate::db::get_workflow_run_status(&mut *tx, run_id).await?;

    let run_status = match run {
        Some(r) => r.0,
        None => return Ok(()), // Already archived or doesn't exist
    };

    match run_status {
        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Aborted => {
            // Proceed with archiving
        }
        _ => return Ok(()), // Not terminal
    }

    info!("Archiving workflow run {}", run_id);
    crate::db::archive_and_delete_workflow_run(&mut *tx, run_id)
        .await
        .with_context(|| format!("Failed to archive and delete workflow run {}", run_id))?;

    tx.commit().await?;

    info!("Successfully archived workflow run {}", run_id);

    Ok(())
}
