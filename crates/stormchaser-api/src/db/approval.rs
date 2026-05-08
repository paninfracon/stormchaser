use serde_json::Value;
use sqlx::PgPool;

/// Inserts a record into the approval registry.
/// Insert approval registry.
pub async fn insert_approval_registry(
    pool: &PgPool,
    id: stormchaser_model::EventId,
    step_id: stormchaser_model::StepInstanceId,
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
