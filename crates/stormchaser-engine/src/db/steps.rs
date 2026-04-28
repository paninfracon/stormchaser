#![allow(unused_imports)]
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Executor, Postgres};
use stormchaser_model::runner::RunnerStatus;
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use uuid::Uuid;

use stormchaser_model::test_report;

pub struct StepDefinitionInput {
    pub step_type: String,
    pub schema: Value,
    pub documentation: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_step_definition<'a, E>(
    executor: E,
    step_type: &str,
    schema: &Value,
    documentation: Option<&str>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                INSERT INTO step_definitions (step_type, schema, documentation, registered_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (step_type) DO UPDATE SET
                    schema = EXCLUDED.schema,
                    documentation = EXCLUDED.documentation
                "#,
    )
    .bind(step_type)
    .bind(schema)
    .bind(documentation)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_step_definition_with_wasm<'a, E>(
    executor: E,
    step_type: &str,
    schema: &Value,
    documentation: Option<&str>,
    wasm_module: &str,
    wasm_function: &str,
    wasm_config: &Value,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO step_definitions (step_type, schema, documentation, registered_at, wasm_module, wasm_function, wasm_config)
        VALUES ($1, $2, $3, NOW(), $4, $5, $6)
        ON CONFLICT (step_type) DO UPDATE SET
            schema = EXCLUDED.schema,
            documentation = EXCLUDED.documentation,
            wasm_module = EXCLUDED.wasm_module,
            wasm_function = EXCLUDED.wasm_function,
            wasm_config = EXCLUDED.wasm_config
        "#,
    )
    .bind(step_type)
    .bind(schema)
    .bind(documentation)
    .bind(wasm_module)
    .bind(wasm_function)
    .bind(wasm_config)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn complete_step_instance<'a, E>(
    executor: E,
    status: &StepStatus,
    exit_code: Option<i32>,
    runner_id: Option<&str>,
    id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn get_step_instances_by_run_id<'a, E, O>(
    executor: E,
    run_id: Uuid,
) -> Result<Vec<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE run_id = $1"#,
    )
    .bind(run_id)
    .fetch_all(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_step_output<'a, E>(
    executor: E,
    step_instance_id: Uuid,
    key: &str,
    value: &Value,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                INSERT INTO step_outputs (step_instance_id, key, value)
                VALUES ($1, $2, $3)
                ON CONFLICT (step_instance_id, key) DO UPDATE SET value = EXCLUDED.value
                "#,
    )
    .bind(step_instance_id)
    .bind(key)
    .bind(value)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_step_output_with_sensitivity<'a, E>(
    executor: E,
    step_instance_id: Uuid,
    key: &str,
    value: &Value,
    is_sensitive: bool,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                                            INSERT INTO step_outputs (step_instance_id, key, value, is_sensitive)
                                            VALUES ($1, $2, $3, $4)
                                            ON CONFLICT (step_instance_id, key) DO UPDATE SET value = EXCLUDED.value
                                            "#,
    )
    .bind(step_instance_id)
    .bind(key)
    .bind(value)
    .bind(is_sensitive)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_step_instance_status<'a, E>(
    executor: E,
    status: &StepStatus,
    id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn get_step_spec_and_params<'a, E, O>(executor: E, id: Uuid) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>("SELECT spec, params FROM step_instances WHERE id = $1")
        .bind(id)
        .fetch_one(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn fail_step_instance_with_error<'a, E>(
    executor: E,
    status: StepStatus,
    error: &str,
    exit_code: Option<i32>,
    id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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

#[allow(clippy::too_many_arguments)]
pub async fn get_step_outputs_for_run<'a, E, O>(
    executor: E,
    run_id: Uuid,
) -> Result<Vec<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"
        SELECT s.step_name, o.key, o.value
        FROM step_outputs o
        JOIN step_instances s ON o.step_instance_id = s.id
        WHERE s.run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_all(executor)
    .await
}

pub async fn record_step_status_history<'a, E>(
    executor: E,
    step_instance_id: Uuid,
    status: &StepStatus,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn insert_step_instance<'a, E>(
    executor: E,
    id: Uuid,
    run_id: Uuid,
    step_name: &str,
    step_type: &str,
    status: StepStatus,
    created_at: DateTime<Utc>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn count_running_steps_for_run<'a, E, O>(
    executor: E,
    run_id: Uuid,
) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin,
    (O,): for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_scalar::<_, O>(
        r#"SELECT COUNT(*) FROM step_instances WHERE run_id = $1 AND status = 'running'"#,
    )
    .bind(run_id)
    .fetch_one(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_step_instance_with_spec<'a, E>(
    executor: E,
    id: Uuid,
    run_id: Uuid,
    step_name: &str,
    step_type: &str,
    status: StepStatus,
    iteration_index: Option<i32>,
    spec: Value,
    params: Value,
    created_at: DateTime<Utc>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn insert_step_instance_with_spec_on_conflict_do_nothing<'a, E>(
    executor: E,
    id: Uuid,
    run_id: Uuid,
    step_name: &str,
    step_type: &str,
    status: StepStatus,
    iteration_index: Option<i32>,
    spec: Value,
    params: Value,
    created_at: DateTime<Utc>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn get_wasm_step_definition<'a, E, O>(
    executor: E,
    step_type: &str,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>(
        "SELECT wasm_module, wasm_function, wasm_config FROM step_definitions WHERE step_type = $1 AND wasm_module IS NOT NULL"
    )
    .bind(step_type)
    .fetch_optional(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn get_pending_step_instances_for_run<'a, E, O>(
    executor: E,
    run_id: Uuid,
    limit: i64,
) -> Result<Vec<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
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
pub async fn get_step_instance_by_id<'a, E, O>(
    executor: E,
    id: Uuid,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1"#
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn fail_pending_steps_for_run_on_timeout<'a, E>(
    executor: E,
    run_id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn update_step_instance_running<'a, E>(
    executor: E,
    status: &StepStatus,
    runner_id: Option<&str>,
    id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
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
pub async fn update_step_instance_terminal<'a, E>(
    executor: E,
    status: &StepStatus,
    id: Uuid,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE step_instances SET status = $1, finished_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(executor)
        .await
}

pub async fn get_test_summaries_for_run<'a, E>(
    executor: E,
    run_id: Uuid,
) -> Result<Vec<test_report::TestSummary>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        WITH combined AS (
            SELECT * FROM step_test_summaries
            UNION ALL
            SELECT * FROM archived_step_test_summaries
        )
        SELECT * FROM combined WHERE run_id = $1 ORDER BY created_at ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(executor)
    .await
}

pub async fn get_test_cases_for_report<'a, E>(
    executor: E,
    run_id: Uuid,
    report_name: &str,
) -> Result<Vec<test_report::TestCase>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        WITH combined AS (
            SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::text as status, duration_ms, message, created_at FROM step_test_cases
            UNION ALL
            SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::text as status, duration_ms, message, created_at FROM archived_step_test_cases
        )
        SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::test_case_status as status, duration_ms, message, created_at
        FROM combined WHERE run_id = $1 AND report_name = $2 ORDER BY created_at ASC
        "#,
    )
    .bind(run_id)
    .bind(report_name)
    .fetch_all(executor)
    .await
}

pub async fn get_step_type_and_spec<'a, E, O>(executor: E, id: Uuid) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
{
    sqlx::query_as::<_, O>("SELECT step_type, spec FROM step_instances WHERE id = $1")
        .bind(id)
        .fetch_one(executor)
        .await
}
