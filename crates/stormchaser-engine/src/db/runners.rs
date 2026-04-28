#![allow(unused_imports)]
use crate::db::steps::StepDefinitionInput;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{Executor, Postgres};
use stormchaser_model::runner::RunnerStatus;
use stormchaser_model::step::StepStatus;
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use uuid::Uuid;

use stormchaser_model::runner;

#[allow(clippy::too_many_arguments)]
pub async fn mark_stale_runners_offline<'e, E>(
    executor: E,
    target_status: RunnerStatus,
    current_status: RunnerStatus,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        "UPDATE runners SET status = $1 WHERE status = $2 AND last_heartbeat_at < NOW() - INTERVAL '30 seconds'"
    )
    .bind(target_status)
    .bind(current_status)
    .execute(executor)
    .await
}

pub async fn get_runner(
    pool: &sqlx::PgPool,
    id: &str,
) -> Result<Option<runner::Runner>, sqlx::Error> {
    sqlx::query_as(r#"SELECT * FROM runners WHERE id = $1"#)
        .bind(id)
        .fetch_optional(pool)
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn register_runner_with_steps(
    conn: &mut sqlx::PgConnection,
    runner_id: &str,
    runner_type: &str,
    protocol_version: &str,
    capabilities: &[String],
    nats_subject: &str,
    step_types: Vec<StepDefinitionInput>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO runners (id, runner_type, status, protocol_version, capabilities, nats_subject, last_heartbeat_at, registered_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
        ON CONFLICT (id) DO UPDATE SET
            runner_type = EXCLUDED.runner_type,
            status = EXCLUDED.status,
            protocol_version = EXCLUDED.protocol_version,
            capabilities = EXCLUDED.capabilities,
            nats_subject = EXCLUDED.nats_subject,
            last_heartbeat_at = EXCLUDED.last_heartbeat_at
        "#
    )
    .bind(runner_id)
    .bind(runner_type)
    .bind(RunnerStatus::Online)
    .bind(protocol_version)
    .bind(capabilities)
    .bind(nats_subject)
    .execute(&mut *conn)
    .await?;

    for st in step_types {
        sqlx::query(
            r#"
                INSERT INTO step_definitions (step_type, schema, documentation, registered_at)
                VALUES ($1, $2, $3, NOW())
                ON CONFLICT (step_type) DO UPDATE SET
                    schema = EXCLUDED.schema,
                    documentation = EXCLUDED.documentation
                "#,
        )
        .bind(st.step_type.clone())
        .bind(&st.schema)
        .bind(st.documentation)
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            r#"
                INSERT INTO runner_step_types (runner_id, step_type)
                VALUES ($1, $2)
                ON CONFLICT (runner_id, step_type) DO NOTHING
                "#,
        )
        .bind(runner_id)
        .bind(st.step_type)
        .execute(&mut *conn)
        .await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_runner<'a, E>(
    executor: E,
    id: &str,
    runner_type: &str,
    status: RunnerStatus,
    protocol_version: &str,
    capabilities: &[String],
    nats_subject: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO runners (id, runner_type, status, protocol_version, capabilities, nats_subject, last_heartbeat_at, registered_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
        ON CONFLICT (id) DO UPDATE SET
            runner_type = EXCLUDED.runner_type,
            status = EXCLUDED.status,
            protocol_version = EXCLUDED.protocol_version,
            capabilities = EXCLUDED.capabilities,
            nats_subject = EXCLUDED.nats_subject,
            last_heartbeat_at = EXCLUDED.last_heartbeat_at
        "#
    )
    .bind(id)
    .bind(runner_type)
    .bind(status)
    .bind(protocol_version)
    .bind(capabilities)
    .bind(nats_subject)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn register_runner_step_type<'a, E>(
    executor: E,
    runner_id: &str,
    step_type: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
                INSERT INTO runner_step_types (runner_id, step_type)
                VALUES ($1, $2)
                ON CONFLICT (runner_id, step_type) DO NOTHING
                "#,
    )
    .bind(runner_id)
    .bind(step_type)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_runner_heartbeat<'a, E>(
    executor: E,
    status: RunnerStatus,
    id: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE runners SET last_heartbeat_at = NOW(), status = $1 WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_runner_status<'a, E>(
    executor: E,
    status: RunnerStatus,
    id: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE runners SET status = $1, last_heartbeat_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(executor)
        .await
}
