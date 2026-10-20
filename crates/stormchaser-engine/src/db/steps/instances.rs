use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgQueryResult;
use sqlx::postgres::PgRow;
use sqlx::{Executor, Postgres};
pub use stormchaser_core::db::steps::{read_step_completion_token, step_completion_token};
use stormchaser_model::step::StepStatus;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ZombieStep {
    pub id: StepInstanceId,
    pub run_id: RunId,
    pub runner_id: Option<String>,
    pub fencing_token: i64,
}

#[allow(clippy::too_many_arguments)]
/// Get step instances by run id.
pub async fn get_step_instances_by_run_id<'a, E, O>(
    executor: E,
    run_id: RunId,
) -> Result<Vec<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1"#,
    )
    .bind(run_id)
    .fetch_all(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get step spec and params.
pub async fn get_step_spec_and_params<'a, E, O>(
    executor: E,
    id: StepInstanceId,
) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>("SELECT spec, params FROM step_instances WHERE id = $1")
        .bind(id)
        .fetch_one(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
/// Insert step instance.
pub async fn insert_step_instance<'a, E>(
    executor: E,
    id: StepInstanceId,
    run_id: RunId,
    step_name: &str,
    step_type: &str,
    status: StepStatus,
    created_at: DateTime<Utc>,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6)
                    ON CONFLICT DO NOTHING
                "#,
    )
    .bind(id)
    .bind(run_id)
    .bind(step_name)
    .bind(step_type)
    .bind(status)
    .bind(created_at)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Count running steps for run.
pub async fn count_running_steps_for_run<'a, E, O>(
    executor: E,
    run_id: RunId,
) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin,
    (O,): for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_scalar::<_, O>(
        r#"SELECT COUNT(*) FROM step_instances WHERE run_id = $1 AND status = 'running'"#,
    )
    .bind(run_id)
    .fetch_one(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Insert step instance with spec.
pub async fn insert_step_instance_with_spec<'a, E>(
    executor: E,
    id: StepInstanceId,
    run_id: RunId,
    step_name: &str,
    step_type: &str,
    status: StepStatus,
    iteration_index: Option<i32>,
    affinity_context: Option<String>,
    spec: Value,
    params: Value,
    created_at: DateTime<Utc>,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                INSERT INTO step_instances (id, run_id, step_name, step_type, status, iteration_index, affinity_context, spec, params, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
    )
    .bind(id)
    .bind(run_id)
    .bind(step_name)
    .bind(step_type)
    .bind(status)
    .bind(iteration_index)
    .bind(affinity_context)
    .bind(spec)
    .bind(params)
    .bind(created_at)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Insert step instance with spec on conflict do nothing.
pub async fn insert_step_instance_with_spec_on_conflict_do_nothing<'a, E>(
    executor: E,
    id: StepInstanceId,
    run_id: RunId,
    step_name: &str,
    step_type: &str,
    status: StepStatus,
    iteration_index: Option<i32>,
    affinity_context: Option<String>,
    spec: Value,
    params: Value,
    created_at: DateTime<Utc>,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
            INSERT INTO step_instances (id, run_id, step_name, step_type, status, iteration_index, affinity_context, spec, params, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                ON CONFLICT DO NOTHING
            "#,
    )
    .bind(id)
    .bind(run_id)
    .bind(step_name)
    .bind(step_type)
    .bind(status)
    .bind(iteration_index)
    .bind(affinity_context)
    .bind(spec)
    .bind(params)
    .bind(created_at)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get pending step instances for run.
pub async fn get_pending_step_instances_for_run<'a, E, O>(
    executor: E,
    run_id: RunId,
    limit: i64,
) -> Result<Vec<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"
        SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at
        FROM step_instances
        WHERE run_id = $1 AND status = 'pending' AND step_type NOT IN ('Approval', 'Wait')
        ORDER BY created_at ASC
        LIMIT $2
        "#
    )
    .bind(run_id)
    .bind(limit)
    .fetch_all(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get step instance by id.
pub async fn get_step_instance_by_id<'a, E, O>(
    executor: E,
    id: StepInstanceId,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1"#
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

/// Get step type and spec.
pub async fn get_step_type_and_spec<'a, E, O>(
    executor: E,
    id: StepInstanceId,
) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>("SELECT step_type, spec FROM step_instances WHERE id = $1")
        .bind(id)
        .fetch_one(executor)
        .await
}

/// Clears a step's completion token because it is being handed to a runner, which
/// reports the result itself and never holds the token.
pub async fn clear_step_completion_token<'a, E>(
    executor: E,
    id: StepInstanceId,
) -> Result<(), sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE step_instances SET completion_token = NULL WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Fetch steps that are currently running on offline runners.
pub async fn fetch_zombie_steps<'a, E>(executor: E) -> Result<Vec<ZombieStep>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        SELECT s.id, s.run_id, s.runner_id, wr.fencing_token
        FROM step_instances s
        JOIN workflow_runs wr ON s.run_id = wr.id
        JOIN runners r ON s.runner_id = r.id
        WHERE s.status IN ('running', 'unpacking_sfs', 'packing_sfs')
        AND (r.status = 'offline' OR r.last_heartbeat_at < NOW() - INTERVAL '30 seconds')
        FOR UPDATE OF s SKIP LOCKED
        "#,
    )
    .fetch_all(executor)
    .await
}

/// Whether a runner has gone quiet: it is unknown, offline, or has missed a heartbeat.
///
/// Runners send a heartbeat every 10 seconds. One that has sent none for
/// `RUNNER_QUIET_SECS` has missed at least one, which is the earliest sign that the
/// steps it holds may need another runner. It is a lower bar than the 30 seconds after
/// which the engine gives a runner's steps up for lost: a task is redelivered to another
/// runner before then, and that runner must be able to take the step over.
pub async fn runner_has_gone_quiet<'a, E>(executor: E, runner_id: &str) -> Result<bool, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    let alive: bool = sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM runners
            WHERE id = $1
              AND status <> 'offline'
              AND last_heartbeat_at >= NOW() - make_interval(secs => $2)
        )
        "#,
    )
    .bind(runner_id)
    .bind(RUNNER_QUIET_SECS)
    .fetch_one(executor)
    .await?;
    Ok(!alive)
}

/// Seconds without a heartbeat after which another runner may take over a runner's steps.
pub const RUNNER_QUIET_SECS: f64 = 12.0;

/// Whether a step is still in flight on a runner that is offline or has stopped
/// heartbeating: the same condition [`fetch_zombie_steps`] selects on.
pub async fn step_runner_is_lost<'a, E>(
    executor: E,
    id: StepInstanceId,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1
            FROM step_instances s
            JOIN runners r ON s.runner_id = r.id
            WHERE s.id = $1
            AND s.status IN ('running', 'unpacking_sfs', 'packing_sfs')
            AND (r.status = 'offline' OR r.last_heartbeat_at < NOW() - INTERVAL '30 seconds')
        )
        "#,
    )
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Get runner id for a given affinity context in a run.
pub async fn get_affinity_runner_id<'a, E>(
    executor: E,
    run_id: RunId,
    affinity_context: &str,
) -> Result<Option<String>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    let record: Option<(Option<String>,)> = sqlx::query_as(
        r#"
        SELECT runner_id
        FROM step_instances
        WHERE run_id = $1 AND affinity_context = $2 AND runner_id IS NOT NULL
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(affinity_context)
    .fetch_optional(executor)
    .await?;

    Ok(record.and_then(|r| r.0))
}

/// Get the id of the earliest-dispatched in-flight step for a given affinity context.
///
/// This is used to elect an affinity leader so that concurrent dispatches do not
/// route to different runners before any runner has been established for the group.
pub async fn get_affinity_leader_step_id<'a, E>(
    executor: E,
    run_id: RunId,
    affinity_context: &str,
) -> Result<Option<StepInstanceId>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    let record: Option<(StepInstanceId,)> = sqlx::query_as(
        r#"
        SELECT id
        FROM step_instances
        WHERE run_id = $1 AND affinity_context = $2
          AND status IN ('pending', 'initializing', 'unpacking_sfs', 'running', 'packing_sfs')
        ORDER BY created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .bind(run_id)
    .bind(affinity_context)
    .fetch_optional(executor)
    .await?;

    Ok(record.map(|r| r.0))
}

/// Moves a step that is with a runner to another runner, returning true if it moved.
///
/// Used when a different runner reports that it is now executing the step: it adopted the
/// step's container or job after a restart, or it received the redelivered task.
pub async fn reassign_step_runner<'a, E>(
    executor: E,
    id: StepInstanceId,
    runner_id: &str,
) -> Result<bool, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    let result = sqlx::query(
        r#"
        UPDATE step_instances
        SET runner_id = $2
        WHERE id = $1
          AND status IN ('initializing', 'running', 'unpacking_sfs', 'packing_sfs')
          AND runner_id IS DISTINCT FROM $2
        "#,
    )
    .bind(id)
    .bind(runner_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Records that a claimed step's task message is about to be published to a runner.
pub async fn mark_step_published<'a, E>(
    executor: E,
    id: StepInstanceId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE step_instances SET dispatched_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await
}

/// Records that the engine process `engine_id` is starting a claimed step itself. A
/// step marked and then lost is failed; one started and not marked would never end.
pub async fn set_step_run_by_engine<'a, E>(
    executor: E,
    id: StepInstanceId,
    engine_id: uuid::Uuid,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE step_instances SET dispatched_at = NOW(), engine_id = $2 WHERE id = $1")
        .bind(id)
        .bind(engine_id)
        .execute(executor)
        .await
}

/// A built-in step whose engine stopped sending heartbeats before it finished.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LostEngineStep {
    pub id: StepInstanceId,
    pub run_id: RunId,
    pub fencing_token: i64,
    pub completion_token: Option<String>,
}

/// Takes the built-in steps whose engine has sent no heartbeat for `lost_after`,
/// and forgets which engine ran them, so that each is taken once. They stay locked
/// until the transaction ends.
///
/// Steps of the `rerunnable` types are left for [`reset_lost_engine_steps`]. Each
/// statement sees what other transactions committed before it began, so a step can
/// become lost after that query ran in the same transaction (a stalled engine that
/// starts one more step, say); without this filter it would be failed here instead of
/// run again.
pub async fn take_lost_engine_steps(
    tx: &mut sqlx::PgConnection,
    lost_after: std::time::Duration,
    rerunnable: &[&str],
) -> Result<Vec<LostEngineStep>, sqlx::Error> {
    sqlx::query_as(
        r#"
        WITH lost AS (
            SELECT s.id
            FROM step_instances s
            JOIN engine_instances e ON s.engine_id = e.id
            WHERE s.status IN ('initializing', 'running')
              AND NOT s.step_type = ANY($2)
              AND e.last_heartbeat_at < NOW() - make_interval(secs => $1)
            FOR UPDATE OF s SKIP LOCKED
        )
        UPDATE step_instances s
        SET engine_id = NULL
        FROM lost, workflow_runs wr
        WHERE s.id = lost.id AND wr.id = s.run_id
        RETURNING s.id, s.run_id, wr.fencing_token, s.completion_token
        "#,
    )
    .bind(lost_after.as_secs_f64())
    .bind(rerunnable)
    .fetch_all(tx)
    .await
}

/// Forgets engines that have sent no heartbeat for `gone_after` and are running
/// no step.
pub async fn delete_gone_engines<'a, E>(
    executor: E,
    gone_after: std::time::Duration,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        DELETE FROM engine_instances e
        WHERE e.last_heartbeat_at < NOW() - make_interval(secs => $1)
          AND NOT EXISTS (
              SELECT 1 FROM step_instances s
              WHERE s.engine_id = e.id AND s.status IN ('initializing', 'running')
          )
        "#,
    )
    .bind(gone_after.as_secs_f64())
    .execute(executor)
    .await
}

/// Running runs with a step created more than `older_than` ago and still pending.
/// Dispatch happens when a run's events arrive; a run whose steps were all put back to
/// pending has none coming. A run whose steps wait for its concurrency limit is listed
/// too, and dispatching it again finds no room, which costs a query.
pub async fn runs_with_waiting_steps<'a, E>(
    executor: E,
    older_than: std::time::Duration,
) -> Result<Vec<RunId>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_scalar(
        r#"
        SELECT DISTINCT si.run_id
        FROM step_instances si
        JOIN workflow_runs wr ON wr.id = si.run_id
        WHERE wr.status = 'running'
          AND si.status = 'pending'
          AND si.step_type NOT IN ('Approval', 'Wait')
          AND si.created_at < NOW() - make_interval(secs => $1)
          AND (si.retry_at IS NULL OR si.retry_at <= NOW())
        "#,
    )
    .bind(older_than.as_secs_f64())
    .fetch_all(executor)
    .await
}

/// How many times a step has been run, counting the run in progress.
pub async fn step_attempt<'a, E>(executor: E, id: StepInstanceId) -> Result<i32, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_scalar("SELECT attempt FROM step_instances WHERE id = $1")
        .bind(id)
        .fetch_one(executor)
        .await
}

/// Running runs with a step whose wait before its next attempt is over.
pub async fn runs_with_due_retries<'a, E>(executor: E) -> Result<Vec<RunId>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_scalar(
        r#"
        SELECT DISTINCT si.run_id
        FROM step_instances si
        JOIN workflow_runs wr ON wr.id = si.run_id
        WHERE si.retry_at <= NOW()
          AND si.status = 'pending'
          AND wr.status = 'running'
        "#,
    )
    .fetch_all(executor)
    .await
}
