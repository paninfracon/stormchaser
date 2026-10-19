use sqlx::{Executor, Postgres};
use stormchaser_model::StepInstanceId;

/// Returns the secret that must accompany the event finishing a step the engine handles
/// itself, or `None` once the step has been handed to a runner.
pub async fn read_step_completion_token<'a, E>(
    executor: E,
    id: StepInstanceId,
) -> Result<Option<String>, sqlx::Error>
where
    E: Executor<'a, Database = Postgres>,
{
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT completion_token FROM step_instances WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
    .map(Option::flatten)
}

/// [`read_step_completion_token`], for checking an event's token: a lookup failure is
/// reported as `None`, and the event is then rejected, which is the safe outcome there.
/// The engine writing a result of its own reads the token with
/// [`read_step_completion_token`] instead, since an event without it would be rejected
/// and its step never end.
pub async fn step_completion_token<'a, E>(executor: E, id: StepInstanceId) -> Option<String>
where
    E: Executor<'a, Database = Postgres>,
{
    match read_step_completion_token(executor, id).await {
        Ok(token) => token,
        Err(e) => {
            tracing::error!("Failed to read completion token for step {}: {:?}", id, e);
            None
        }
    }
}
