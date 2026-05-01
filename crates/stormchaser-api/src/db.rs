use crate::{
    ListRunsQuery, TestReportSummary, TestSummaryResponse, UpdateStorageBackendRequest,
    WorkflowRunDetail,
};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use stormchaser_model::event_rules::{EventRule, WebhookConfig};
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

use stormchaser_model::cron;
use stormchaser_model::event;
use stormchaser_model::step;
use stormchaser_model::storage;
use stormchaser_model::TestCase;
/// Inserts a new workflow run into the database.
#[allow(clippy::too_many_arguments)]
/// Insert workflow run.
pub async fn insert_workflow_run(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
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
    run_id: Uuid,
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
    run_id: Uuid,
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
    run_id: Uuid,
) -> Result<Option<WorkflowRunDetail>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM combined_run_details WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await
}
/// Retrieves step instances for a specific run.
/// Get step instances.
pub async fn get_step_instances(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<step::StepInstance>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM combined_step_instances WHERE run_id = $1 ORDER BY created_at ASC",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}
/// Retrieves the outputs for a specific step instance.
/// Get step outputs.
pub async fn get_step_outputs(
    pool: &PgPool,
    step_instance_id: Uuid,
) -> Result<Vec<step::StepOutput>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM combined_step_outputs WHERE step_instance_id = $1")
        .bind(step_instance_id)
        .fetch_all(pool)
        .await
}
/// Retrieves the status history for a specific step instance.
/// Get step status history.
pub async fn get_step_status_history(
    pool: &PgPool,
    step_instance_id: Uuid,
) -> Result<Vec<step::StepStatusHistory>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM combined_step_status_history WHERE step_instance_id = $1 ORDER BY created_at ASC")
        .bind(step_instance_id)
        .fetch_all(pool)
        .await
}

/// Creates a new webhook configuration.
pub async fn create_webhook(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    description: &Option<String>,
    source_type: &str,
    secret_token: &Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO webhooks (id, name, description, source_type, secret_token) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(source_type)
    .bind(secret_token)
    .execute(pool)
    .await?;
    Ok(())
}

/// Retrieves all configured webhooks
pub async fn list_webhooks(pool: &PgPool) -> Result<Vec<WebhookConfig>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM webhooks ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}
/// Retrieves a specific webhook by ID.
/// Get webhook.
pub async fn get_webhook(pool: &PgPool, id: Uuid) -> Result<Option<WebhookConfig>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM webhooks WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}
/// Retrieves an active webhook by ID.
/// Get active webhook.
pub async fn get_active_webhook(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<WebhookConfig>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM webhooks WHERE id = $1 AND is_active = TRUE")
        .bind(id)
        .fetch_optional(pool)
        .await
}
/// Deletes a webhook from the database.
/// Delete webhook.
pub async fn delete_webhook(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM webhooks WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
/// Creates a new event rule.
#[allow(clippy::too_many_arguments)]
/// Create event rule.
pub async fn create_event_rule(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    description: &Option<String>,
    webhook_id: Option<Uuid>,
    event_type_pattern: &str,
    condition_expr: &Option<String>,
    workflow_name: &str,
    repo_url: &str,
    workflow_path: &str,
    git_ref: &str,
    input_mappings: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO event_rules (
            id, name, description, webhook_id, event_type_pattern, condition_expr,
            workflow_name, repo_url, workflow_path, git_ref, input_mappings
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(webhook_id)
    .bind(event_type_pattern)
    .bind(condition_expr)
    .bind(workflow_name)
    .bind(repo_url)
    .bind(workflow_path)
    .bind(git_ref)
    .bind(input_mappings)
    .execute(pool)
    .await?;
    Ok(())
}
/// Retrieves all event rules.
/// List event rules.
pub async fn list_event_rules(pool: &PgPool) -> Result<Vec<EventRule>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM event_rules ORDER BY created_at DESC")
        .fetch_all(pool)
        .await
}
/// Retrieves active event rules associated with a specific webhook.
/// Get active event rules by webhook.
pub async fn get_active_event_rules_by_webhook(
    pool: &PgPool,
    webhook_id: Uuid,
) -> Result<Vec<EventRule>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM event_rules WHERE webhook_id = $1 AND is_active = TRUE")
        .bind(webhook_id)
        .fetch_all(pool)
        .await
}
/// Deletes an event rule from the database.
/// Delete event rule.
pub async fn delete_event_rule(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM event_rules WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Creates a new cron workflow configuration.
#[allow(clippy::too_many_arguments)]
pub async fn create_cron_workflow(
    pool: &PgPool,
    id: Uuid,
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
    id: Uuid,
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
    id: Uuid,
) -> Result<Option<cron::CronWorkflow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM cron_workflows WHERE id = $1 AND is_active = TRUE")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Deletes a scheduled workflow configuration
pub async fn delete_cron_workflow(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM cron_workflows WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
/// Unsets the default Stormchaser File System.
/// Unset default sfs.
pub async fn unset_default_sfs(tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE storage_backends SET is_default_sfs = FALSE WHERE is_default_sfs = TRUE")
        .execute(&mut **tx)
        .await?;
    Ok(())
}
/// Creates a new storage backend.
/// Create storage backend.
pub async fn create_storage_backend(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    name: &str,
    description: &Option<String>,
    backend_type: &storage::BackendType,
    config: &Value,
    is_default_sfs: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO storage_backends (id, name, description, backend_type, config, is_default_sfs)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(backend_type)
    .bind(config)
    .bind(is_default_sfs)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
/// Retrieves all storage backends.
/// List storage backends.
pub async fn list_storage_backends(
    pool: &PgPool,
) -> Result<Vec<storage::StorageBackend>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM storage_backends ORDER BY name ASC")
        .fetch_all(pool)
        .await
}
/// Retrieves a storage backend by ID.
/// Get storage backend.
pub async fn get_storage_backend(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<storage::StorageBackend>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM storage_backends WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}
/// Updates an existing storage backend.
/// Update storage backend.
pub async fn update_storage_backend(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    payload: &UpdateStorageBackendRequest,
) -> Result<(), sqlx::Error> {
    let mut query = sqlx::QueryBuilder::new("UPDATE storage_backends SET ");
    let mut separated = query.separated(", ");

    if let Some(name) = &payload.name {
        separated.push("name = ").push_bind_unseparated(name);
    }
    if let Some(desc) = &payload.description {
        separated.push("description = ").push_bind_unseparated(desc);
    }
    if let Some(bt) = &payload.backend_type {
        separated.push("backend_type = ").push_bind_unseparated(bt);
    }
    if let Some(config) = &payload.config {
        separated.push("config = ").push_bind_unseparated(config);
    }
    if let Some(is_default) = &payload.is_default_sfs {
        separated
            .push("is_default_sfs = ")
            .push_bind_unseparated(is_default);
    }

    query.push(" WHERE id = ").push_bind(id);

    query.build().execute(&mut **tx).await?;
    Ok(())
}
/// Deletes a storage backend from the database.
/// Delete storage backend.
pub async fn delete_storage_backend(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM storage_backends WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Retrieves a list of artifacts associated with a given workflow run.
pub async fn list_run_artifacts(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<storage::ArtifactRegistry>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_artifacts AS (
                SELECT * FROM artifact_registry
                UNION ALL
                SELECT * FROM archived_artifact_registry
            )
            SELECT * FROM combined_artifacts WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}
/// Retrieves test reports for a specific workflow run.
/// List run test reports.
pub async fn list_run_test_reports(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<TestReportSummary>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_reports AS (
                SELECT id, run_id, report_name, file_name, format, checksum, created_at, backend_id, remote_path FROM step_test_reports
                UNION ALL
                SELECT id, run_id, report_name, file_name, format, checksum, created_at, backend_id, remote_path FROM archived_step_test_reports
            )
            SELECT id, report_name, file_name, format, checksum, created_at, backend_id, remote_path FROM combined_reports WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}
/// Retrieves test summaries for a specific workflow run.
/// List run test summaries.
pub async fn list_run_test_summaries(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<TestSummaryResponse>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_summaries AS (
                SELECT id, run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms, created_at FROM step_test_summaries
                UNION ALL
                SELECT id, run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms, created_at FROM archived_step_test_summaries
            )
            SELECT id, run_id, step_instance_id, report_name, total_tests, passed, failed, skipped, errors, duration_ms, created_at FROM combined_summaries WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}
/// Retrieves individual test cases for a workflow run.
/// List run test cases.
pub async fn list_run_test_cases(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<TestCase>, sqlx::Error> {
    sqlx::query_as(
        r#"
            WITH combined_cases AS (
                SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::test_case_status as "status", duration_ms, message, created_at FROM step_test_cases
                UNION ALL
                SELECT id, run_id, step_instance_id, report_name, test_suite, test_case, status::test_case_status as "status", duration_ms, message, created_at FROM archived_step_test_cases
            )
            SELECT * FROM combined_cases WHERE run_id = $1 ORDER BY created_at ASC
            "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}
/// Retrieves a test report by ID.
/// Get test report.
pub async fn get_test_report(
    pool: &PgPool,
    report_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
            WITH combined_reports AS (
                SELECT id, content FROM step_test_reports
                UNION ALL
                SELECT id, content FROM archived_step_test_reports
            )
            SELECT content FROM combined_reports WHERE id = $1
            "#,
    )
    .bind(report_id)
    .fetch_optional(pool)
    .await
}
/// Retrieves a step ID by its name and run ID.
/// Get step id by name.
pub async fn get_step_id_by_name(
    pool: &PgPool,
    run_id: Uuid,
    step_name: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM step_instances WHERE run_id = $1 AND step_name = $2 LIMIT 1")
        .bind(run_id)
        .bind(step_name)
        .fetch_optional(pool)
        .await
}
/// Retrieves the status of a workflow run.
/// Get workflow run status.
pub async fn get_workflow_run_status(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT status::text FROM combined_workflow_runs WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await
}
/// Retrieves step names and their IDs for a workflow run.
/// Get step names.
pub async fn get_step_names(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, step_name FROM combined_step_instances WHERE run_id = $1")
        .bind(run_id)
        .fetch_all(pool)
        .await
}
/// Retrieves the combined status for a workflow run.
/// Get combined run status.
pub async fn get_combined_run_status(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT status::text FROM combined_workflow_runs WHERE id = $1")
        .bind(run_id)
        .fetch_optional(pool)
        .await
}
/// Retrieves combined step statuses for a workflow run.
/// Get combined step statuses.
pub async fn get_combined_step_statuses(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Vec<(Uuid, String, String)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, step_name, status::text FROM combined_step_instances WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}
/// Retrieves a step instance for human approval.
/// Get step instance for approval.
pub async fn get_step_instance_for_approval(
    pool: &PgPool,
    step_id: Uuid,
    run_id: Uuid,
) -> Result<Option<step::StepInstance>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1 AND run_id = $2"#,
    )
    .bind(step_id)
    .bind(run_id)
    .fetch_optional(pool)
    .await
}
/// Inserts a record into the approval registry.
/// Insert approval registry.
pub async fn insert_approval_registry(
    pool: &PgPool,
    id: Uuid,
    step_id: Uuid,
    user_id: &str,
    status: &str,
    payload: &Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO approval_registry (id, step_instance_id, user_id, status, payload) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(step_id)
    .bind(user_id)
    .bind(status)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}
/// Retrieves an event correlation by key and value.
/// Get event correlation.
pub async fn get_event_correlation(
    pool: &PgPool,
    key: &str,
    value: &str,
) -> Result<Option<event::EventCorrelation>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, step_instance_id, run_id, correlation_key, correlation_value, created_at FROM event_correlations WHERE correlation_key = $1 AND correlation_value = $2"
    )
    .bind(key)
    .bind(value)
    .fetch_optional(pool)
    .await
}

/// Deletes a workflow run completely from the system (active and archived).
pub async fn delete_workflow_run(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
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

/// Inserts a new webhook.
pub async fn insert_webhook(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    description: &Option<String>,
    source_type: &str,
    secret_token: &Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO webhooks (id, name, description, source_type, secret_token) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(source_type)
    .bind(secret_token)
    .execute(pool)
    .await?;
    Ok(())
}

/// Inserts a new cron workflow.
pub async fn insert_cron_workflow(
    pool: &PgPool,
    id: Uuid,
    payload: &crate::routes::CreateCronWorkflowRequest,
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
    .bind(&payload.repo_url)
    .bind(&payload.workflow_path)
    .bind(&payload.git_ref)
    .bind(&payload.inputs)
    .bind(secret_token)
    .bind(external_job_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Retrieves run outputs for OPA evaluation.
pub async fn get_run_outputs_for_opa(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<serde_json::Map<String, serde_json::Value>, sqlx::Error> {
    use sqlx::Row;
    let outputs_rows = sqlx::query(
        r#"
        SELECT i.step_name, o.output_key, o.output_value
        FROM combined_step_instances i
        JOIN combined_step_outputs o ON i.id = o.step_instance_id
        WHERE i.run_id = $1
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    let mut run_outputs_map = serde_json::Map::new();
    for row in outputs_rows {
        let step_name: String = row.get("step_name");
        let output_key: String = row.get("output_key");
        let output_value: serde_json::Value = row.get("output_value");

        if !run_outputs_map.contains_key(&step_name) {
            run_outputs_map.insert(step_name.clone(), serde_json::json!({"outputs": {}}));
        }
        if let Some(step_obj) = run_outputs_map
            .get_mut(&step_name)
            .and_then(|v| v.as_object_mut())
        {
            if let Some(outputs_obj) =
                step_obj.get_mut("outputs").and_then(|v| v.as_object_mut())
            {
                outputs_obj.insert(output_key, output_value);
            }
        }
    }
    Ok(run_outputs_map)
}

/// Data returned for workflow OPA context
pub struct WorkflowOpaContextData {
    pub initiating_user: String,
    pub workflow_definition: serde_json::Value,
    pub run_inputs: serde_json::Value,
}

/// Retrieves workflow context for OPA evaluation.
pub async fn get_workflow_context_for_opa(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Option<WorkflowOpaContextData>, sqlx::Error> {
    let context_row = sqlx::query(
        r#"
        SELECT
            wr.initiating_user,
            rc.workflow_definition,
            rc.inputs as run_inputs
        FROM workflow_runs wr
        JOIN run_contexts rc ON wr.id = rc.run_id
        WHERE wr.id = $1
        "#,
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = context_row {
        use sqlx::Row;
        let initiating_user: String = row.get("initiating_user");
        let workflow_definition: serde_json::Value = row.get("workflow_definition");
        let run_inputs: serde_json::Value = row.get("run_inputs");
        Ok(Some(WorkflowOpaContextData {
            initiating_user,
            workflow_definition,
            run_inputs,
        }))
    } else {
        Ok(None)
    }
}

/// Deletes an event correlation record
pub async fn delete_event_correlation(
    pool: &sqlx::PgPool,
    id: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM event_correlations WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
