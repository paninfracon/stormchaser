use crate::{ListRunsQuery, WorkflowRunDetail};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;

/// Inserts a new workflow run into the database.
#[allow(clippy::too_many_arguments)]
/// Insert workflow run.
pub async fn insert_workflow_run(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    workflow_name: &str,
    initiating_user: &str,
    repo_url: &str,
    workflow_path: &str,
    git_ref: &str,
    status: RunStatus,
    fencing_token: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO workflow_runs (id, workflow_name, initiating_user, repo_url, workflow_path, git_ref, status, fencing_token)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(run_id)
    .bind(workflow_name)
    .bind(initiating_user)
    .bind(repo_url)
    .bind(workflow_path)
    .bind(git_ref)
    .bind(status)
    .bind(fencing_token)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Inserts the context details of a workflow run.
/// Insert run context.
pub async fn insert_run_context(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    dsl_version: &str,
    workflow_definition: Value,
    source_code: &str,
    inputs: &Value,
) -> Result<(), sqlx::Error> {
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
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Inserts the resource quotas for a workflow run.
/// Insert run quotas.
pub async fn insert_run_quotas(
    tx: &mut Transaction<'_, Postgres>,
    run_id: RunId,
    max_concurrency: i32,
    max_cpu: &str,
    max_memory: &str,
    max_storage: &str,
    timeout: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO run_quotas (run_id, max_concurrency, max_cpu, max_memory, max_storage, timeout)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(run_id)
    .bind(max_concurrency)
    .bind(max_cpu)
    .bind(max_memory)
    .bind(max_storage)
    .bind(timeout)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Retrieves a list of workflow runs, optionally filtered by the query parameters.
pub async fn list_workflow_runs(
    pool: &PgPool,
    params: &ListRunsQuery,
    limit: i64,
    offset: i64,
) -> Result<Vec<WorkflowRunDetail>, sqlx::Error> {
    let mut query = sqlx::QueryBuilder::new(
        r#"
        WITH combined_runs AS (
            SELECT
                wr.id, wr.workflow_name, wr.initiating_user, wr.repo_url, wr.workflow_path, wr.git_ref,
                wr.status::run_status as "status", wr.version, wr.created_at, wr.updated_at, wr.started_resolving_at, wr.started_at, wr.finished_at, wr.error,
                rc.inputs, rc.secrets, rc.source_code, rc.dsl_version
            FROM workflow_runs wr
            JOIN run_contexts rc ON wr.id = rc.run_id
            UNION ALL
            SELECT
                wr.id, wr.workflow_name, wr.initiating_user, wr.repo_url, wr.workflow_path, wr.git_ref,
                wr.status::run_status as "status", wr.version, wr.created_at, wr.updated_at, wr.started_resolving_at, wr.started_at, wr.finished_at, wr.error,
                rc.inputs, rc.secrets, rc.source_code, rc.dsl_version
            FROM archived_workflow_runs wr
            JOIN archived_run_contexts rc ON wr.id = rc.run_id
        )
        SELECT * FROM combined_runs wr WHERE 1=1
        "#,
    );

    if let Some(name) = &params.workflow_name {
        query.push(" AND wr.workflow_name LIKE ");
        query.push_bind(format!("%{}%", name));
    }

    if let Some(status) = &params.status {
        query.push(" AND wr.status = ");
        query.push_bind(status);
    }

    if let Some(user) = &params.initiating_user {
        query.push(" AND wr.initiating_user = ");
        query.push_bind(user);
    }

    if let Some(repo) = &params.repo_url {
        query.push(" AND wr.repo_url = ");
        query.push_bind(repo);
    }

    if let Some(path) = &params.workflow_path {
        query.push(" AND wr.workflow_path = ");
        query.push_bind(path);
    }

    if let Some(after) = &params.created_after {
        query.push(" AND wr.created_at >= ");
        query.push_bind(after);
    }

    if let Some(before) = &params.created_before {
        query.push(" AND wr.created_at <= ");
        query.push_bind(before);
    }

    query.push(" ORDER BY wr.created_at DESC LIMIT ");
    query.push_bind(limit);
    query.push(" OFFSET ");
    query.push_bind(offset);

    query.build_query_as().fetch_all(pool).await
}

/// Retrieves full details for a workflow run.
/// Get workflow run detail.
pub async fn get_workflow_run_detail(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Option<WorkflowRunDetail>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM combined_run_details WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await
}

/// Retrieves the status of a workflow run.
/// Get workflow run status.
pub async fn get_workflow_run_status(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Option<RunStatus>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT status as "status: RunStatus" FROM combined_workflow_runs WHERE id = $1"#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

/// Retrieves the combined status for a workflow run.
/// Get combined run status.
pub async fn get_combined_run_status(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Option<RunStatus>, sqlx::Error> {
    sqlx::query_scalar(
        r#"SELECT status as "status: RunStatus" FROM combined_workflow_runs WHERE id = $1"#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

/// Deletes a workflow run completely from the system (active and archived).
pub async fn delete_workflow_run(pool: &PgPool, id: RunId) -> Result<(), sqlx::Error> {
    // Delete from active table
    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;

    // Delete from archived table
    sqlx::query("DELETE FROM archived_workflow_runs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}
