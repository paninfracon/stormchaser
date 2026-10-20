//! Transitions made on many steps in one statement. Each moves only steps whose
//! status, checked in the statement, allows the move, so a step another engine has
//! moved meanwhile is left alone.

use super::{state, StepMachine, StepState};
use serde_json::Value;
use stormchaser_model::step::StepInstance;
use stormchaser_model::{RunId, StepInstanceId};

/// Claims up to `limit` of a run's pending steps for dispatch, oldest first. `FOR
/// UPDATE SKIP LOCKED` means two engines never claim the same step. Approval and Wait
/// steps are left to their events, and a step waiting for its next attempt until
/// `retry_at`.
pub async fn claim_for_run(
    pool: &sqlx::PgPool,
    run_id: RunId,
    limit: i64,
) -> Result<Vec<StepMachine<state::Initializing>>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let rows: Vec<StepInstance> = sqlx::query_as(
        r#"
        WITH claimed AS (
            SELECT id
            FROM step_instances
            WHERE run_id = $1
              AND status = 'pending'
              AND step_type NOT IN ('Approval', 'Wait')
              AND (retry_at IS NULL OR retry_at <= NOW())
            ORDER BY created_at ASC
            LIMIT $2
            FOR UPDATE SKIP LOCKED
        )
        UPDATE step_instances si
        SET status = 'initializing', claimed_at = NOW(), dispatched_at = NULL, engine_id = NULL,
            retry_at = NULL
        FROM claimed
        WHERE si.id = claimed.id
        RETURNING si.id, si.run_id, si.step_name, si.step_type, si.status as "status",
                  si.iteration_index, si.runner_id, si.affinity_context, si.started_at,
                  si.finished_at, si.exit_code, si.error, si.spec, si.params, si.created_at
        "#,
    )
    .bind(run_id)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(rows.into_iter().map(claimed).collect())
}

/// The machine of a step a statement here has just moved to `S`'s status.
fn claimed<S: StepState>(instance: StepInstance) -> StepMachine<S> {
    debug_assert!(S::STATUSES.contains(&instance.status));
    StepMachine {
        stored: instance.status.clone(),
        instance,
        _state: std::marker::PhantomData,
    }
}

/// A step claimed for dispatch and put back to pending, its dispatch lost.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LostDispatch {
    pub id: StepInstanceId,
    pub run_id: RunId,
    pub step_type: String,
    pub spec: Value,
}

/// Puts back to pending the steps of running runs that were claimed for dispatch more
/// than `older_than` ago and never handed over: the engine dispatching them died, or
/// could not put them back itself.
pub async fn requeue_lost_dispatches(
    executor: &mut sqlx::PgConnection,
    older_than: std::time::Duration,
) -> Result<Vec<LostDispatch>, sqlx::Error> {
    sqlx::query_as(
        r#"
        UPDATE step_instances si
        SET status = 'pending', claimed_at = NULL
        FROM workflow_runs wr
        WHERE si.run_id = wr.id
          AND wr.status = 'running'
          AND si.status = 'initializing'
          AND si.dispatched_at IS NULL
          AND si.claimed_at < NOW() - make_interval(secs => $1)
        RETURNING si.id, si.run_id, si.step_type, si.spec
        "#,
    )
    .bind(older_than.as_secs_f64())
    .fetch_all(executor)
    .await
}

/// Puts back to pending the built-in steps of `step_types`, which have no effect but
/// their output, whose engine has sent no heartbeat for `lost_after`. They run again.
pub async fn requeue_lost_engine_steps(
    executor: &mut sqlx::PgConnection,
    lost_after: std::time::Duration,
    step_types: &[&str],
) -> Result<Vec<(StepInstanceId, RunId)>, sqlx::Error> {
    sqlx::query_as(
        r#"
        UPDATE step_instances s
        SET status = 'pending', engine_id = NULL, runner_id = NULL, started_at = NULL,
            claimed_at = NULL, dispatched_at = NULL
        FROM engine_instances e, workflow_runs wr
        WHERE s.engine_id = e.id
          AND wr.id = s.run_id
          AND wr.status = 'running'
          AND s.status IN ('initializing', 'running')
          AND s.step_type = ANY($2)
          AND e.last_heartbeat_at < NOW() - make_interval(secs => $1)
        RETURNING s.id, s.run_id
        "#,
    )
    .bind(lost_after.as_secs_f64())
    .bind(step_types)
    .fetch_all(executor)
    .await
}
