use sqlx::{Executor, Postgres};
use uuid::Uuid;

/// Remembers where a step's task message sits in the stream, so it can be erased later.
///
/// A sequence number is only reused if the stream was deleted and created again; the
/// newer step then takes over the row.
pub async fn record_task_message<'e, E>(
    executor: E,
    stream_seq: u64,
    step_instance_id: Uuid,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO dispatched_task_messages (stream_seq, step_instance_id)
        VALUES ($1, $2)
        ON CONFLICT (stream_seq)
        DO UPDATE SET step_instance_id = EXCLUDED.step_instance_id, created_at = NOW()
        "#,
    )
    .bind(stream_seq as i64)
    .bind(step_instance_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Locks a batch of task messages whose step has finished or has been archived.
pub async fn fetch_finished_task_messages<'e, E>(
    executor: E,
    limit: i64,
) -> Result<Vec<(i64, Uuid)>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        SELECT t.stream_seq, t.step_instance_id
        FROM dispatched_task_messages t
        WHERE NOT EXISTS (
            SELECT 1 FROM step_instances s
            WHERE s.id = t.step_instance_id
              AND s.status::text NOT IN
                  ('succeeded', 'failed', 'failed_ignored', 'skipped', 'aborted', 'lost_zombie')
        )
        ORDER BY t.stream_seq
        LIMIT $1
        FOR UPDATE OF t SKIP LOCKED
        "#,
    )
    .bind(limit)
    .fetch_all(executor)
    .await
}

/// Forgets a task message once it is gone from the stream.
pub async fn delete_task_message_record<'e, E>(
    executor: E,
    stream_seq: i64,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query("DELETE FROM dispatched_task_messages WHERE stream_seq = $1")
        .bind(stream_seq)
        .execute(executor)
        .await?;
    Ok(())
}
