use anyhow::Result;
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::workflow::WorkflowRun;

/// Persist run.
pub async fn persist_run(run: &mut WorkflowRun, executor: &mut sqlx::PgConnection) -> Result<()> {
    let result = crate::db::update_workflow_run_status_full(
        executor,
        &run.status,
        run.updated_at,
        run.started_resolving_at,
        run.started_at,
        run.finished_at,
        run.error.as_deref(),
        run.id,
        run.version,
    )
    .await;

    let res = match result {
        Ok(r) => r,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to persist workflow run state for {}: {:?}",
                run.id,
                e
            ))
        }
    };

    if res.rows_affected() == 0 {
        anyhow::bail!(
            "Optimistic concurrency control failure for run {}: version mismatch (expected {})",
            run.id,
            run.version
        );
    }

    run.version += 1;

    Ok(())
}

/// Persists a `StepInstance` state change to the database, along with logs and outputs.
pub async fn persist_step_instance(
    instance: &StepInstance,
    executor: &mut sqlx::PgConnection,
) -> Result<()> {
    match instance.status {
        StepStatus::Running | StepStatus::UnpackingSfs => {
            crate::db::update_step_instance_running(
                &mut *executor,
                &instance.status,
                instance.runner_id.as_deref(),
                instance.id,
            )
            .await?;
        }
        StepStatus::Succeeded | StepStatus::Failed => {
            crate::db::complete_step_instance(
                &mut *executor,
                &instance.status,
                instance.exit_code,
                instance.runner_id.as_deref(),
                instance.id,
            )
            .await?;
        }
        StepStatus::Skipped | StepStatus::Aborted => {
            crate::db::update_step_instance_terminal(&mut *executor, &instance.status, instance.id)
                .await?;
        }
        _ => {
            crate::db::update_step_instance_status(&mut *executor, &instance.status, instance.id)
                .await?;
        }
    }

    // Record status history
    crate::db::record_step_status_history(executor, instance.id, &instance.status).await?;

    Ok(())
}
