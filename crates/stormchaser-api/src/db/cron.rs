use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::CronWorkflowId;

use stormchaser_model::cron;

/// Creates a new cron workflow configuration.
#[allow(clippy::too_many_arguments)]
pub async fn create_cron_workflow(
    pool: &PgPool,
    id: CronWorkflowId,
    name: &str,
    description: &Option<String>,
    cronspec: &str,
    workflow_name: &str,
    repo_url: &str,
    workflow_path: &str,
    git_ref: &str,
    inputs: &Value,
    secret_token: &str,
    external_job_id: &Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO cron_workflows (id, name, description, cronspec, workflow_name, repo_url, workflow_path, git_ref, inputs, secret_token, external_job_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(cronspec)
    .bind(workflow_name)
    .bind(repo_url)
    .bind(workflow_path)
    .bind(git_ref)
    .bind(inputs)
    .bind(secret_token)
    .bind(external_job_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Retrieves all cron workflows.
/// List cron workflows.
pub async fn list_cron_workflows(pool: &PgPool) -> Result<Vec<cron::CronWorkflow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM cron_workflows ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}

/// Retrieves a cron workflow by ID.
/// Get cron workflow.
pub async fn get_cron_workflow(
    pool: &PgPool,
    id: CronWorkflowId,
) -> Result<Option<cron::CronWorkflow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM cron_workflows WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Retrieves an active cron workflow by ID.
/// Get active cron workflow.
pub async fn get_active_cron_workflow(
    pool: &PgPool,
    id: CronWorkflowId,
) -> Result<Option<cron::CronWorkflow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM cron_workflows WHERE id = $1 AND is_active = TRUE")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Deletes a scheduled workflow configuration
pub async fn delete_cron_workflow(pool: &PgPool, id: CronWorkflowId) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM cron_workflows WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Inserts a new cron workflow.
pub async fn insert_cron_workflow(
    pool: &PgPool,
    id: CronWorkflowId,
    payload: &crate::routes::CreateCronWorkflowRequest,
    repo_url: &str,
    secret_token: &str,
    external_job_id: Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO cron_workflows (id, name, description, cronspec, workflow_name, repo_url, workflow_path, git_ref, inputs, secret_token, external_job_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(id)
    .bind(&payload.name)
    .bind(&payload.description)
    .bind(&payload.cronspec)
    .bind(&payload.workflow_name)
    .bind(repo_url)
    .bind(&payload.workflow_path)
    .bind(&payload.git_ref)
    .bind(&payload.inputs)
    .bind(secret_token)
    .bind(external_job_id)
    .execute(pool)
    .await?;
    Ok(())
}
