use serde_json::Value;
use sqlx::postgres::PgQueryResult;
use sqlx::{Executor, Postgres};
use stormchaser_model::{RunId, StepInstanceId};

#[allow(clippy::too_many_arguments)]
/// Upsert step output.
pub async fn upsert_step_output<'a, E>(
    executor: E,
    step_instance_id: StepInstanceId,
    key: &str,
    value: &Value,
) -> Result<PgQueryResult, sqlx::Error>
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
/// Upsert step output with sensitivity.
pub async fn upsert_step_output_with_sensitivity<'a, E>(
    executor: E,
    step_instance_id: StepInstanceId,
    key: &str,
    value: &Value,
    is_sensitive: bool,
) -> Result<PgQueryResult, sqlx::Error>
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
/// Get step outputs for run.
pub async fn get_step_outputs_for_run<'a, E, O>(
    executor: E,
    run_id: RunId,
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
