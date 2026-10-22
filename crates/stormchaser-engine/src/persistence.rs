//! Where the state machines save. Each save is a compare-and-set on the status the
//! machine loaded, so a machine never overwrites a change it has not seen.

use anyhow::{Context, Result};
use stormchaser_model::step::{StepInstance, StepStatus};
use stormchaser_model::workflow::WorkflowRun;

/// Saves a run's new status and times, if the row is still at the version and status
/// the machine loaded.
pub async fn persist_run(
    run: &mut WorkflowRun,
    expected: &stormchaser_model::workflow::RunStatus,
    executor: &mut sqlx::PgConnection,
) -> Result<()> {
    let result = sqlx::query(
        r#"
        UPDATE workflow_runs
        SET status = $1, updated_at = $2, started_resolving_at = $3, started_at = $4,
            finished_at = $5, error = $6, version = version + 1
        WHERE id = $7 AND version = $8 AND status = $9
        "#,
    )
    .bind(&run.status)
    .bind(run.updated_at)
    .bind(run.started_resolving_at)
    .bind(run.started_at)
    .bind(run.finished_at)
    .bind(run.error.as_deref())
    .bind(run.id)
    .bind(run.version)
    .bind(expected)
    .execute(executor)
    .await;

    // The database error stays in the chain, so a caller can tell a lost connection
    // from a bad update
    let res =
        result.with_context(|| format!("Failed to persist workflow run state for {}", run.id))?;

    if res.rows_affected() == 0 {
        anyhow::bail!(
            "Optimistic concurrency control failure for run {}: it is no longer {:?} at version {}",
            run.id,
            expected,
            run.version
        );
    }

    run.version += 1;

    Ok(())
}

/// Saves a step's new status and what a transition sets with it, if its stored status
/// is still `expected`. The database records the change in the step's history.
pub async fn persist_step_instance(
    instance: &StepInstance,
    expected: &StepStatus,
    executor: &mut sqlx::PgConnection,
) -> Result<()> {
    let saved = sqlx::query(
        r#"
        UPDATE step_instances
        SET status = $2, runner_id = $3, started_at = $4, finished_at = $5,
            exit_code = $6, error = $7
        WHERE id = $1 AND status = $8
        "#,
    )
    .bind(instance.id)
    .bind(&instance.status)
    .bind(instance.runner_id.as_deref())
    .bind(instance.started_at)
    .bind(instance.finished_at)
    .bind(instance.exit_code)
    .bind(instance.error.as_deref())
    .bind(expected)
    .execute(executor)
    .await
    .with_context(|| format!("Failed to save step {}", instance.id))?;

    if saved.rows_affected() == 0 {
        anyhow::bail!(
            "step {} is no longer {:?}; not moving it to {:?}",
            instance.id,
            expected,
            instance.status
        );
    }
    Ok(())
}
