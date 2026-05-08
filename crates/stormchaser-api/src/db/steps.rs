use sqlx::PgPool;
use stormchaser_model::step::StepStatus;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

use stormchaser_model::step;

/// Retrieves step instances for a specific run.
/// Get step instances.
pub async fn get_step_instances(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Vec<step::StepInstance>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM combined_step_instances WHERE run_id = $1 ORDER BY created_at ASC",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

/// Retrieves a single step instance by its UUID and run ID.
pub async fn get_step_instance_by_id(
    pool: &PgPool,
    run_id: RunId,
    step_id: StepInstanceId,
) -> Result<Option<step::StepInstance>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM combined_step_instances WHERE run_id = $1 AND id = $2")
        .bind(run_id)
        .bind(step_id)
        .fetch_optional(pool)
        .await
}

/// Retrieves the outputs for a specific step instance.
/// Get step outputs.
pub async fn get_step_outputs(
    pool: &PgPool,
    step_instance_id: StepInstanceId,
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
    step_instance_id: StepInstanceId,
) -> Result<Vec<step::StepStatusHistory>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM combined_step_status_history WHERE step_instance_id = $1 ORDER BY created_at ASC")
        .bind(step_instance_id)
        .fetch_all(pool)
        .await
}

/// Retrieves a step ID by its name and run ID.
/// Get step id by name.
pub async fn get_step_id_by_name(
    pool: &PgPool,
    run_id: RunId,
    step_name: &str,
) -> Result<Option<stormchaser_model::StepInstanceId>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM step_instances WHERE run_id = $1 AND step_name = $2 LIMIT 1")
        .bind(run_id)
        .bind(step_name)
        .fetch_optional(pool)
        .await
}

/// Retrieves step names and their IDs for a workflow run.
/// Get step names.
pub async fn get_step_names(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Vec<(stormchaser_model::StepInstanceId, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, step_name FROM combined_step_instances WHERE run_id = $1")
        .bind(run_id)
        .fetch_all(pool)
        .await
}

/// Retrieves combined step statuses for a workflow run.
/// Get combined step statuses.
pub async fn get_combined_step_statuses(
    pool: &PgPool,
    run_id: RunId,
) -> Result<Vec<(stormchaser_model::StepInstanceId, String, StepStatus)>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT id, step_name, status as "status: StepStatus" FROM combined_step_instances WHERE run_id = $1"#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
}

/// Retrieves a step instance for human approval.
/// Get step instance for approval.
pub async fn get_step_instance_for_approval(
    pool: &PgPool,
    step_id: StepInstanceId,
    run_id: RunId,
) -> Result<Option<step::StepInstance>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT id, run_id, step_name, step_type, status as "status", iteration_index, runner_id, affinity_context, started_at, finished_at, exit_code, error, spec, params, created_at FROM step_instances WHERE id = $1 AND run_id = $2"#,
    )
    .bind(step_id)
    .bind(run_id)
    .fetch_optional(pool)
    .await
}
