use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::event_rules::EventRule;
use uuid::Uuid;

use stormchaser_model::event;

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
