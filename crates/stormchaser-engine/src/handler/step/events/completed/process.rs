use crate::handler::fetch_outputs;
use crate::handler::step::dispatch::dispatch_step_instance;
use crate::handler::step::scheduling::schedule_step;
use crate::handler::StepInstance;
use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_dsl::ast;
use stormchaser_model::step::StepStatus;
use stormchaser_model::RunId;
use stormchaser_tls::TlsReloader;

#[allow(clippy::too_many_arguments)]
pub async fn process_step_completion(
    dsl_step: &ast::Step,
    all_steps: &[StepInstance],
    run_id: RunId,
    tx: &mut sqlx::PgConnection,
    nats_client: async_nats::Client,
    pool: PgPool,
    tls_reloader: Arc<TlsReloader>,
    inputs: Value,
    workflow: &ast::Workflow,
) -> Result<bool> {
    tracing::info!(
        "process_step_completion called for step {} run {}",
        dsl_step.name,
        run_id
    );
    if !schedule_iterated_batches(
        dsl_step,
        all_steps,
        run_id,
        tx,
        nats_client.clone(),
        pool.clone(),
        tls_reloader.clone(),
    )
    .await?
    {
        return Ok(false);
    }

    evaluate_successors(
        dsl_step,
        all_steps,
        run_id,
        tx,
        nats_client,
        pool,
        workflow,
        inputs,
    )
    .await?;

    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn schedule_iterated_batches(
    dsl_step: &ast::Step,
    all_steps: &[StepInstance],
    run_id: RunId,
    tx: &mut sqlx::PgConnection,
    nats_client: async_nats::Client,
    pool: PgPool,
    tls_reloader: Arc<TlsReloader>,
) -> Result<bool> {
    let all_instances_of_this_step: Vec<&StepInstance> = all_steps
        .iter()
        .filter(|s| s.step_name == dsl_step.name)
        .collect();

    let finished_instances = all_instances_of_this_step
        .iter()
        .filter(|s| s.status == StepStatus::Succeeded || s.status == StepStatus::Skipped)
        .count();

    let total_instances = all_instances_of_this_step.len();

    if finished_instances < total_instances {
        // Some instances are still running or waiting
        let waiting_instances: Vec<&&StepInstance> = all_instances_of_this_step
            .iter()
            .filter(|s| s.status == StepStatus::WaitingForEvent)
            .collect();

        if !waiting_instances.is_empty() {
            let running_or_pending = all_instances_of_this_step
                .iter()
                .filter(|s| s.status == StepStatus::Running || s.status == StepStatus::Pending)
                .count();

            let max_parallel = dsl_step
                .strategy
                .as_ref()
                .and_then(|s| s.max_parallel)
                .unwrap_or(u32::MAX);

            if (running_or_pending as u32) < max_parallel {
                let to_schedule = max_parallel - (running_or_pending as u32);
                for next_instance in waiting_instances.iter().take(to_schedule as usize) {
                    let machine = crate::step_machine::StepMachine::<
                        crate::step_machine::state::WaitingForEvent,
                    >::from_instance((**next_instance).clone());
                    let _ = machine.reschedule(&mut *tx).await?;

                    let inst_data: (Value, Value) =
                        crate::db::get_step_spec_and_params(&mut *tx, next_instance.id).await?;

                    dispatch_step_instance(
                        run_id,
                        next_instance.id,
                        &dsl_step.name,
                        &dsl_step.r#type,
                        &inst_data.0,
                        &inst_data.1,
                        nats_client.clone(),
                        pool.clone(),
                        tls_reloader.clone(),
                    )
                    .await?;
                }
            }
        }
        return Ok(false);
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
async fn evaluate_successors(
    dsl_step: &ast::Step,
    all_steps: &[StepInstance],
    run_id: RunId,
    tx: &mut sqlx::PgConnection,
    nats_client: async_nats::Client,
    pool: PgPool,
    workflow: &ast::Workflow,
    inputs: Value,
) -> Result<()> {
    if dsl_step.next.is_empty() {
        return Ok(());
    }

    let hcl_ctx = crate::hcl_eval::create_context(
        inputs,
        run_id,
        fetch_outputs(run_id, &mut *tx).await?,
        Some(workflow),
        Some(dsl_step),
    );

    for next_step_name in &dsl_step.next {
        let predecessors: Vec<&ast::Step> = workflow
            .steps
            .iter()
            .filter(|s| s.next.contains(next_step_name))
            .collect();

        let all_predecessors_done = predecessors.iter().all(|pred_dsl| {
            let pred_instances: Vec<&StepInstance> = all_steps
                .iter()
                .filter(|s| s.step_name == pred_dsl.name)
                .collect();

            !pred_instances.is_empty()
                && pred_instances
                    .iter()
                    .all(|s| s.status == StepStatus::Succeeded || s.status == StepStatus::Skipped)
        });

        if all_predecessors_done {
            if let Some(next_dsl) = workflow.steps.iter().find(|s| s.name == *next_step_name) {
                #[allow(clippy::explicit_auto_deref)]
                schedule_step(
                    run_id,
                    next_dsl,
                    &mut *tx,
                    nats_client.clone(),
                    &hcl_ctx,
                    pool.clone(),
                    workflow,
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_process_step_completion_compiles() {
        let _f = process_step_completion;
    }
}
