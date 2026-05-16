use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::postgres::PgQueryResult;
use sqlx::postgres::PgRow;
use sqlx::{Executor, Postgres};
use stormchaser_model::workflow::{RunStatus, WorkflowRun};
use stormchaser_model::RunId;

#[allow(clippy::too_many_arguments)]
/// Get active workflow runs with quotas.
pub async fn get_active_workflow_runs_with_quotas<'e, E, O>(
    executor: E,
) -> Result<Vec<O>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"
                SELECT wr.id, wr.status, wr.created_at, wr.started_at, rq.timeout
                FROM workflow_runs wr
                JOIN run_quotas rq ON wr.id = rq.run_id
                WHERE wr.status IN ('queued', 'resolving', 'start_pending', 'running')
                "#,
    )
    .fetch_all(executor)
    .await
}

/// Get runs stuck in the 'resolving' state.
pub async fn get_stalled_resolving_runs<'e, E>(
    executor: E,
    timeout_minutes: i64,
) -> Result<Vec<RunId>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    #[derive(sqlx::FromRow)]
    struct IdRow {
        id: uuid::Uuid,
    }

    let rows: Vec<IdRow> = sqlx::query_as(
        r#"
        SELECT id
        FROM workflow_runs
        WHERE status = 'resolving'
          AND started_resolving_at < NOW() - INTERVAL '1 minute' * $1
        "#,
    )
    .bind(timeout_minutes)
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(|r| RunId::new(r.id)).collect())
}

#[allow(clippy::too_many_arguments)]
/// Insert full workflow run.
pub async fn insert_full_workflow_run(
    conn: &mut sqlx::PgConnection,
    run: &WorkflowRun,
    dsl_version: &str,
    workflow_definition: Value,
    source_code: Option<&str>,
    inputs: Value,
    max_concurrency: i32,
    max_cpu: &str,
    max_memory: &str,
    max_storage: &str,
    timeout: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token, created_at, updated_at, started_resolving_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7::run_status, $8, $9, $10, $11)
        "#
    )
    .bind(run.id)
    .bind(&run.workflow_name)
    .bind(Some(&run.initiating_user))
    .bind(Some(&run.repo_url))
    .bind(Some(&run.workflow_path))
    .bind(Some(&run.git_ref))
    .bind(run.status.clone())
    .bind(Some(run.fencing_token))
    .bind(run.created_at)
    .bind(run.updated_at)
    .bind(run.started_resolving_at)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(run.id)
    .bind(dsl_version)
    .bind(workflow_definition)
    .bind(source_code)
    .bind(inputs)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO run_quotas (run_id, max_concurrency, max_cpu, max_memory, max_storage, timeout)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(run.id)
    .bind(max_concurrency)
    .bind(max_cpu)
    .bind(max_memory)
    .bind(max_storage)
    .bind(timeout)
    .execute(&mut *conn)
    .await?;

    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// Insert workflow run.
pub async fn insert_workflow_run<'a, E>(
    executor: E,
    id: RunId,
    workflow_name: &str,
    initiating_user: Option<&str>,
    repo_url: Option<&str>,
    workflow_path: Option<&str>,
    git_ref: Option<&str>,
    status: RunStatus,
    fencing_token: Option<i64>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    started_resolving_at: Option<DateTime<Utc>>,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token, created_at, updated_at, started_resolving_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7::run_status, $8, $9, $10, $11)
        "#
    )
    .bind(id)
    .bind(workflow_name)
    .bind(initiating_user)
    .bind(repo_url)
    .bind(workflow_path)
    .bind(git_ref)
    .bind(status)
    .bind(fencing_token)
    .bind(created_at)
    .bind(updated_at)
    .bind(started_resolving_at)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Insert run context.
pub async fn insert_run_context<'a, E>(
    executor: E,
    run_id: RunId,
    dsl_version: &str,
    workflow_definition: Value,
    source_code: Option<&str>,
    inputs: Value,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO run_contexts (run_id, dsl_version, workflow_definition, source_code, inputs)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(run_id)
    .bind(dsl_version)
    .bind(workflow_definition)
    .bind(source_code)
    .bind(inputs)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Update run context.
pub async fn update_run_context<'a, E>(
    executor: E,
    workflow_definition: Value,
    source_code: Option<&str>,
    dsl_version: &str,
    inputs: Value,
    secrets: Value,
    sensitive_values: Vec<String>,
    run_id: RunId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"UPDATE run_contexts SET workflow_definition = $1, source_code = $2, dsl_version = $3, inputs = $4, secrets = $5, sensitive_values = $6 WHERE run_id = $7"#
    )
    .bind(workflow_definition)
    .bind(source_code)
    .bind(dsl_version)
    .bind(inputs)
    .bind(secrets)
    .bind(sensitive_values)
    .bind(run_id)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Lock workflow run.
pub async fn lock_workflow_run<'a, E>(executor: E, id: RunId) -> Result<Option<PgRow>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query("SELECT id FROM workflow_runs WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
/// Update workflow run status.
pub async fn update_workflow_run_status<'a, E>(
    executor: E,
    status: RunStatus,
    id: RunId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
            UPDATE workflow_runs
            SET status = $1, finished_at = NOW(), updated_at = NOW(), version = version + 1
            WHERE id IN (SELECT id FROM workflow_runs WHERE id = $2 AND status != $1 FOR UPDATE)
            "#,
    )
    .bind(status)
    .bind(id)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Fail workflow run.
pub async fn fail_workflow_run<'a, E>(
    executor: E,
    status: RunStatus,
    error: &str,
    id: RunId,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        UPDATE workflow_runs
        SET status = $1, finished_at = NOW(), updated_at = NOW(), version = version + 1, error = $2
        WHERE id = $3 AND status != $1
        "#,
    )
    .bind(status)
    .bind(error)
    .bind(id)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get workflow run by id.
pub async fn get_workflow_run_by_id<'a, E, O>(executor: E, id: RunId) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"SELECT id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status as "status", version, fencing_token, created_at, updated_at, started_resolving_at, started_at, finished_at, error FROM workflow_runs WHERE id = $1"#
    )
    .bind(id)
    .fetch_one(executor)
    .await
}

/// Get workflow run fencing token by id.
pub async fn get_workflow_run_fencing_token_by_id<'a, E>(
    executor: E,
    id: RunId,
) -> Result<i64, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_scalar("SELECT fencing_token FROM workflow_runs WHERE id = $1")
        .bind(id)
        .fetch_one(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
/// Get run context by id.
pub async fn get_run_context_by_id<'a, E, O>(executor: E, run_id: RunId) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>(
        r#"SELECT run_id, dsl_version, workflow_definition, source_code, inputs, secrets, sensitive_values FROM run_contexts WHERE run_id = $1"#
    )
    .bind(run_id)
    .fetch_one(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get run inputs by id.
pub async fn get_run_inputs_by_id<'a, E, O>(executor: E, run_id: RunId) -> Result<O, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>("SELECT inputs FROM run_contexts WHERE run_id = $1")
        .bind(run_id)
        .fetch_one(executor)
        .await
}

#[allow(clippy::too_many_arguments)]
/// Update workflow run status full.
pub async fn update_workflow_run_status_full<'a, E>(
    executor: E,
    status: &RunStatus,
    updated_at: DateTime<Utc>,
    started_resolving_at: Option<DateTime<Utc>>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    error: Option<&str>,
    id: RunId,
    version: i32,
) -> Result<PgQueryResult, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query(
        r#"
        UPDATE workflow_runs
        SET status = $1, updated_at = $2, started_resolving_at = $3, started_at = $4, finished_at = $5, error = $6, version = version + 1
        WHERE id = $7 AND version = $8
        "#
    )
    .bind(status)
    .bind(updated_at)
    .bind(started_resolving_at)
    .bind(started_at)
    .bind(finished_at)
    .bind(error)
    .bind(id)
    .bind(version)
    .execute(executor)
    .await
}

#[allow(clippy::too_many_arguments)]
/// Get workflow run status.
pub async fn get_workflow_run_status<'a, E, O>(
    executor: E,
    id: RunId,
) -> Result<Option<O>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
    O: Send + Unpin + for<'r> sqlx::FromRow<'r, PgRow>,
{
    sqlx::query_as::<_, O>(r#"SELECT status FROM workflow_runs WHERE id = $1"#)
        .bind(id)
        .fetch_optional(executor)
        .await
}
