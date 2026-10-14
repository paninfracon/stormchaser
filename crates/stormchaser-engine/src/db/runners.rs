use crate::db::steps::StepDefinitionInput;
use sqlx::postgres::PgQueryResult;
use sqlx::{Executor, Postgres};
use stormchaser_model::runner::RunnerStatus;

use stormchaser_model::runner;

#[allow(clippy::too_many_arguments)]
/// Mark stale runners offline.
pub async fn mark_stale_runners_offline<'e, E>(
    executor: E,
    target_status: RunnerStatus,
    current_status: RunnerStatus,
) -> Result<PgQueryResult, sqlx::Error>
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

/// Get runner.
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
/// Register runner with steps.
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
/// Upsert runner.
pub async fn upsert_runner<'a, E>(
    executor: E,
    id: &str,
    runner_type: &str,
    status: RunnerStatus,
    protocol_version: &str,
    capabilities: &[String],
    nats_subject: &str,
) -> Result<PgQueryResult, sqlx::Error>
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
/// Register runner step type.
pub async fn register_runner_step_type<'a, E>(
    executor: E,
    runner_id: &str,
    step_type: &str,
) -> Result<PgQueryResult, sqlx::Error>
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
/// Update runner heartbeat.
pub async fn update_runner_heartbeat<'a, E>(
    executor: E,
    status: RunnerStatus,
    id: &str,
) -> Result<PgQueryResult, sqlx::Error>
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
/// Update runner status.
pub async fn update_runner_status<'a, E>(
    executor: E,
    status: RunnerStatus,
    id: &str,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("UPDATE runners SET status = $1, last_heartbeat_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(id)
        .execute(executor)
        .await
}

/// The key issued to `runner_id`, if it has one.
pub async fn get_runner_session_key<'a, E>(
    executor: E,
    runner_id: &str,
) -> Result<Option<Vec<u8>>, sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    sqlx::query_scalar("SELECT session_key FROM runner_sessions WHERE runner_id = $1")
        .bind(runner_id)
        .fetch_optional(executor)
        .await
}

/// Whether a runner registered under `runner_id`, whatever its key.
pub async fn runner_is_known<'a, E>(executor: E, runner_id: &str) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM runners WHERE id = $1)")
        .bind(runner_id)
        .fetch_one(executor)
        .await
}

/// Whether a process that holds a key for `runner_id` is still running: online, with a
/// heartbeat in the last 30 seconds (the same test that decides a runner is lost).
pub async fn runner_session_is_live<'a, E>(
    executor: E,
    runner_id: &str,
) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    sqlx::query_scalar(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM runner_sessions s JOIN runners r ON r.id = s.runner_id
            WHERE s.runner_id = $1
            AND r.status = 'online'
            AND r.last_heartbeat_at >= NOW() - INTERVAL '30 seconds'
        )
        "#,
    )
    .bind(runner_id)
    .fetch_one(executor)
    .await
}

/// Records `session_key` as the key of `runner_id`, unless a process may still hold the
/// id: it is live (see [`runner_session_is_live`]), or it was issued a key in the last 30
/// seconds and may not have sent its first heartbeat yet. The check and the write are one
/// statement, so two requests for the same id cannot both succeed. Returns whether the
/// key was stored.
pub async fn claim_runner_session<'a, E>(
    executor: E,
    runner_id: &str,
    session_key: &[u8],
) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    let claimed: Option<String> = sqlx::query_scalar(
        r#"
        INSERT INTO runner_sessions (runner_id, session_key, issued_at) VALUES ($1, $2, NOW())
        ON CONFLICT (runner_id) DO UPDATE SET session_key = EXCLUDED.session_key, issued_at = NOW()
        WHERE runner_sessions.issued_at < NOW() - INTERVAL '30 seconds'
        AND NOT EXISTS (
            SELECT 1 FROM runners r
            WHERE r.id = runner_sessions.runner_id
            AND r.status = 'online'
            AND r.last_heartbeat_at >= NOW() - INTERVAL '30 seconds'
        )
        RETURNING runner_id
        "#,
    )
    .bind(runner_id)
    .bind(session_key)
    .fetch_optional(executor)
    .await?;
    Ok(claimed.is_some())
}

/// Records the key issued to `runner_id`, replacing any earlier one.
pub async fn store_runner_session<'a, E>(
    executor: E,
    runner_id: &str,
    session_key: &[u8],
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'a, Database = sqlx::Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO runner_sessions (runner_id, session_key, issued_at) VALUES ($1, $2, NOW())
        ON CONFLICT (runner_id) DO UPDATE SET session_key = EXCLUDED.session_key, issued_at = NOW()
        "#,
    )
    .bind(runner_id)
    .bind(session_key)
    .execute(executor)
    .await?;
    Ok(())
}
