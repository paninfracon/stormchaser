use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgQueryResult;
use sqlx::postgres::PgRow;
use sqlx::{Executor, Postgres};
use stormchaser_model::step::StepStatus;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

#[allow(clippy::too_many_arguments)]
/// Complete step instance.
pub async fn complete_step_instance<'a, E>(
    executor: E,
    status: &StepStatus,
    exit_code: Option<i32>,
    runner_id: Option<&str>,
    id: StepInstanceId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        UPDATE step_instances
        SET status = $1, finished_at = NOW(), exit_code = $2, runner_id = COALESCE($3, runner_id)
        WHERE id = $4
        "#,
    )
    .bind(status)
    .bind(exit_code)
    .bind(runner_id)
    .bind(id)
    .execute(executor)
    .await
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
/// Update step instance status.
pub async fn update_step_instance_status<'a, E>(
    executor: E,
    status: &StepStatus,
    id: StepInstanceId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE step_instances SET status = $1 WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(executor)
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
/// Fail step instance with error.
pub async fn fail_step_instance_with_error<'a, E>(
    executor: E,
    status: StepStatus,
    error: &str,
    exit_code: Option<i32>,
    id: StepInstanceId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        UPDATE step_instances
        SET status = $1, finished_at = NOW(), error = $2, exit_code = $3
        WHERE id = $4
        "#,
    )
    .bind(status)
    .bind(error)
    .bind(exit_code)
    .bind(id)
    .execute(executor)
    .await
}

/// Record step status history.
pub async fn record_step_status_history<'a, E>(
    executor: E,
    step_instance_id: StepInstanceId,
    status: &StepStatus,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("INSERT INTO step_status_history (step_instance_id, status) VALUES ($1, $2)")
        .bind(step_instance_id)
        .bind(status)
        .execute(executor)
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
                WITH inserted AS (
                    INSERT INTO step_instances (id, run_id, step_name, step_type, status, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6)
                    ON CONFLICT DO NOTHING
                    RETURNING id
                )
                INSERT INTO step_status_history (step_instance_id, status)
                SELECT id, $5 FROM inserted
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
    spec: Value,
    params: Value,
    created_at: DateTime<Utc>,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                WITH inserted AS (
                    INSERT INTO step_instances (id, run_id, step_name, step_type, status, iteration_index, spec, params, created_at)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                    RETURNING id
                )
                INSERT INTO step_status_history (step_instance_id, status)
                SELECT id, $5 FROM inserted
                "#,
    )
    .bind(id)
    .bind(run_id)
    .bind(step_name)
    .bind(step_type)
    .bind(status)
    .bind(iteration_index)
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
    spec: Value,
    params: Value,
    created_at: DateTime<Utc>,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
            WITH inserted AS (
                INSERT INTO step_instances (id, run_id, step_name, step_type, status, iteration_index, spec, params, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT DO NOTHING
                RETURNING id
            )
            INSERT INTO step_status_history (step_instance_id, status)
            SELECT id, $5 FROM inserted
            "#,
    )
    .bind(id)
    .bind(run_id)
    .bind(step_name)
    .bind(step_type)
    .bind(status)
    .bind(iteration_index)
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

#[allow(clippy::too_many_arguments)]
/// Fail pending steps for run on timeout.
pub async fn fail_pending_steps_for_run_on_timeout<'a, E>(
    executor: E,
    run_id: RunId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        UPDATE step_instances
        SET status = 'failed', error = 'Workflow timed out', finished_at = NOW()
        WHERE run_id = $1 AND status IN ('pending', 'running', 'waiting_for_event')
        "#,
    )
    .bind(run_id)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Update step instance running.
pub async fn update_step_instance_running<'a, E>(
    executor: E,
    status: &StepStatus,
    runner_id: Option<&str>,
    id: StepInstanceId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        "UPDATE step_instances SET status = $1, started_at = COALESCE(started_at, NOW()), runner_id = COALESCE($2, runner_id) WHERE id = $3"
    )
    .bind(status)
    .bind(runner_id)
    .bind(id)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Update step instance terminal.
pub async fn update_step_instance_terminal<'a, E>(
    executor: E,
    status: &StepStatus,
    id: StepInstanceId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE step_instances SET status = $1, finished_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(executor)
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
