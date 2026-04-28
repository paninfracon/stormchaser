#![allow(clippy::explicit_auto_deref)]
use serde_json::Value;
pub mod integrations;
pub mod runner;
pub mod step;
pub mod workflow;

pub use integrations::*;
pub use runner::*;
pub use step::*;
pub use workflow::*;

use anyhow::{Context, Result};
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::step::StepInstance;
use stormchaser_model::workflow::{RunContext, RunQuotas, WorkflowRun};
use stormchaser_tls::TlsReloader;
use tracing::{debug, error, info};
use uuid::Uuid;

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
pub async fn fetch_outputs(
    run_id: Uuid,
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
pub async fn fetch_run<'a, E>(run_id: Uuid, executor: E) -> Result<WorkflowRun>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_workflow_run_by_id(executor, run_id)
        .await
        .map(|v: WorkflowRun| v)
        .with_context(|| format!("Failed to fetch workflow run for {}", run_id))
}

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
pub async fn fetch_run_context<'a, E>(run_id: Uuid, executor: E) -> Result<RunContext>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_run_context_by_id(executor, run_id)
        .await
        .map(|v: RunContext| v)
        .with_context(|| format!("Failed to fetch run context for {}", run_id))
}

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
pub async fn fetch_quotas<'a, E>(run_id: Uuid, executor: E) -> Result<RunQuotas>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_run_quota_by_id(executor, run_id)
        .await
        .map(|v: RunQuotas| v)
        .with_context(|| format!("Failed to fetch run quotas for {}", run_id))
}

#[tracing::instrument(skip(executor), fields(run_id = %run_id))]
pub async fn fetch_inputs<'a, E>(run_id: Uuid, executor: E) -> Result<Value>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    let row: (Value,) = crate::db::get_run_inputs_by_id(executor, run_id)
        .await
        .with_context(|| format!("Failed to fetch inputs for {}", run_id))?;
    Ok(row.0)
}

#[tracing::instrument(skip(executor), fields(step_id = %step_id))]
pub async fn fetch_step_instance<'a, E>(step_id: Uuid, executor: E) -> Result<StepInstance>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    crate::db::get_step_instance_by_id(executor, step_id)
        .await?
        .context("Step instance not found")
}

#[tracing::instrument(skip(pool, nats_client, tls_reloader), fields(run_id = %run_id))]
pub async fn dispatch_pending_steps(
    run_id: Uuid,
    pool: PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) -> Result<()> {
    // 1. Fetch Quotas and Current Running Count
    let quotas = fetch_quotas(run_id, &pool).await?;
    let running_count: i64 = crate::db::count_running_steps_for_run(&pool, run_id).await?;

    if running_count >= quotas.max_concurrency as i64 {
        debug!(
            "Run {}: Max concurrency ({}) reached, not dispatching more steps",
            run_id, quotas.max_concurrency
        );
        return Ok(());
    }

    let available_slots = (quotas.max_concurrency as i64) - running_count;
    debug!(
        "Run {}: {} slots available for concurrent steps",
        run_id, available_slots
    );

    let max_cpu = crate::resource_utils::parse_cpu(&quotas.max_cpu).unwrap_or(0.0);
    let max_mem = crate::resource_utils::parse_memory(&quotas.max_memory).unwrap_or(0);

    // 2. Fetch Pending steps (excluding Approval/Wait which are handled via events)
    let pending_steps: Vec<StepInstance> =
        crate::db::get_pending_step_instances_for_run(&pool, run_id, available_slots).await?;

    for step in pending_steps {
        info!("Run {}: Evaluating queued step {}", run_id, step.step_name);
        // We need to fetch the resolved spec and params for this step
        let inst_data: (Value, Value) = crate::db::get_step_spec_and_params(&pool, step.id).await?;

        // 3. Enforce CPU and Memory quotas before dispatching
        let (cpu_req, mem_req) =
            crate::resource_utils::get_step_resource_requirements(&step.step_type, &inst_data.0);

        let can_dispatch = if max_cpu > 0.0 || max_mem > 0 {
            let mut conn = pool.acquire().await?;
            crate::db::claim_step_quota(&mut *conn, run_id, cpu_req, mem_req, max_cpu, max_mem)
                .await?
        } else {
            true // If no quota configured, allow dispatch
        };

        if !can_dispatch {
            debug!(
                "Run {}: Insufficient CPU/Memory quota to dispatch step {}",
                run_id, step.step_name
            );
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
                let _ = crate::db::release_step_quota(&mut *conn, run_id, cpu_req, mem_req).await;
            }
        }
    }

    Ok(())
}

#[tracing::instrument(skip(pool), fields(run_id = %run_id))]
pub async fn archive_workflow(run_id: Uuid, pool: PgPool) -> Result<()> {
    use stormchaser_model::workflow::RunStatus;
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
