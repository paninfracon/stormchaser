#![allow(unused_imports)]
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Executor, Postgres};
use stormchaser_model::runner::RunnerStatus;
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn insert_event_correlation<'a, E>(
    executor: E,
    id: Uuid,
    step_instance_id: Uuid,
    run_id: Uuid,
    correlation_key: &str,
    correlation_value: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        "INSERT INTO event_correlations (id, step_instance_id, run_id, correlation_key, correlation_value) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(step_instance_id)
    .bind(run_id)
    .bind(correlation_key)
    .bind(correlation_value)
    .execute(executor)
    .await
}
