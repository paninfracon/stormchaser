use serde_json::Value;
use sqlx::{Executor, Postgres};
use stormchaser_model::events::{
    EventSource, EventType, SchemaId, SchemaVersion, StepCompletedEvent, StepEventType,
    StepFailedEvent, WorkflowEventType, WorkflowQueuedEvent,
};
use stormchaser_model::nats::compute_shard_id;
use stormchaser_model::nats::{build_cloudevent_and_headers, NatsSubject};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct OutboxEvent {
    pub id: Uuid,
    pub subject: String,
    pub payload: String,
    pub headers: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Insert a new outbox event
pub async fn insert_outbox_event<'e, E>(
    executor: E,
    subject: NatsSubject,
    event_type: EventType,
    source: EventSource,
    data: Value,
    schema_version: Option<SchemaVersion>,
    schema_id: Option<SchemaId>,
) -> Result<(), anyhow::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let subject_str = subject.as_str().into_owned();
    let (payload, headers_map) = build_cloudevent_and_headers(
        &subject_str,
        event_type,
        source,
        data,
        schema_version,
        schema_id,
    )?;

    // Convert headers_map to JSON
    let mut headers_json = serde_json::Map::new();
    for (key, values) in headers_map.iter() {
        let mut val_strings = Vec::new();
        for v in values {
            val_strings.push(v.as_str().to_string());
        }
        headers_json.insert(key.to_string(), Value::String(val_strings.join(",")));
    }

    sqlx::query(
        r#"
        INSERT INTO outbox_events (subject, payload, headers)
        VALUES ($1, $2, $3)
        "#,
    )
    .bind(subject_str)
    .bind(payload)
    .bind(serde_json::Value::Object(headers_json))
    .execute(executor)
    .await?;

    Ok(())
}

/// Queues the event that starts a new run, for the engine to resolve it. In the run's
/// own transaction, the run and the event that starts it are kept together or not at
/// all.
pub async fn insert_run_queued<'e, E>(
    executor: E,
    run_id: stormchaser_model::RunId,
    source: EventSource,
    sops_file: Option<String>,
    sops_role_arn: Option<String>,
) -> Result<(), anyhow::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    let event = WorkflowQueuedEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::Queued),
        timestamp: chrono::Utc::now(),
        status: stormchaser_model::workflow::RunStatus::Queued,
        dsl: None,
        inputs: None,
        initiating_user: None,
        sops_file,
        sops_role_arn,
    };
    insert_outbox_event(
        executor,
        NatsSubject::RunQueued(Some(compute_shard_id(&run_id))),
        EventType::Workflow(WorkflowEventType::Queued),
        source,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
}

/// Queues a step's completion for the step-completed handler.
pub async fn insert_step_completed<'e, E>(
    executor: E,
    event: &StepCompletedEvent,
) -> Result<(), anyhow::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    insert_outbox_event(
        executor,
        NatsSubject::StepCompleted(Some(compute_shard_id(&event.run_id))),
        EventType::Step(StepEventType::Completed),
        EventSource::System,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
}

/// Queues a step's failure for the step-failed handler.
pub async fn insert_step_failed<'e, E>(
    executor: E,
    event: &StepFailedEvent,
) -> Result<(), anyhow::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    insert_outbox_event(
        executor,
        NatsSubject::StepFailed(Some(compute_shard_id(&event.run_id))),
        EventType::Step(StepEventType::Failed),
        EventSource::System,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
}

/// Fetch a batch of outbox events, locking them for processing
pub async fn fetch_outbox_events_for_processing<'e, E>(
    executor: E,
    limit: i64,
) -> Result<Vec<OutboxEvent>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        r#"
        SELECT id, subject, payload, headers, created_at
        FROM outbox_events
        ORDER BY created_at ASC
        LIMIT $1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(limit)
    .fetch_all(executor)
    .await
}

/// Delete a processed outbox event
pub async fn delete_outbox_event<'e, E>(executor: E, id: Uuid) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"
        DELETE FROM outbox_events
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(executor)
    .await?;

    Ok(())
}

/// Move a failed outbox event to the back of oldest-first processing
pub async fn reschedule_outbox_event<'e, E>(executor: E, id: Uuid) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"
        UPDATE outbox_events
        SET created_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(executor)
    .await?;

    Ok(())
}
