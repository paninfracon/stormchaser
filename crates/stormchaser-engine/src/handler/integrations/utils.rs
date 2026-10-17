use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepCompletedEvent, StepEventType, StepFailedEvent,
};
use stormchaser_model::nats::{compute_shard_id, NatsSubject};
use stormchaser_model::{Connection, RunId, StepInstanceId};

/// Fetches the URL from a configured HttpApi connection by name.
pub async fn fetch_http_api_url_from_connection(
    pool: &PgPool,
    run_id: RunId,
    connection_name: &str,
) -> Result<String> {
    let mut conn = pool.acquire().await?;
    if let Some(connection) = crate::db::connections::get_storage_backend_by_name::<
        &mut sqlx::PgConnection,
        Connection,
    >(&mut *conn, connection_name)
    .await?
    {
        drop(conn);
        crate::authz::ensure_run_may_use(pool, run_id, &connection).await?;
        if connection.connection_type == stormchaser_model::ConnectionType::HttpApi {
            if let Some(url) = connection
                .config
                .get("base_url")
                .and_then(|u| u.as_str())
                // Keep legacy `url` support for existing HttpApi connections.
                .or_else(|| connection.config.get("url").and_then(|u| u.as_str()))
            {
                Ok(url.to_string())
            } else {
                anyhow::bail!(
                    "Connection {} is missing 'base_url' (or legacy 'url') in its config",
                    connection_name
                )
            }
        } else {
            anyhow::bail!("Connection {} must be of type HttpApi", connection_name)
        }
    } else {
        anyhow::bail!("Connection {} not found", connection_name)
    }
}

/// Publishes a StepCompletedEvent for the given step, through the outbox.
pub async fn publish_step_completed_event(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    outputs: Option<std::collections::HashMap<String, serde_json::Value>>,
    pool: &PgPool,
    _nats_client: async_nats::Client,
) -> Result<()> {
    let event = StepCompletedEvent {
        run_id,
        step_id,
        fencing_token,
        completion_token: crate::db::step_completion_token(pool, step_id).await,
        event_type: EventType::Step(StepEventType::Completed),
        runner_id: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs,
        exit_code: None,
        timestamp: Utc::now(),
    };

    crate::db::outbox::insert_outbox_event(
        pool,
        NatsSubject::StepCompleted(Some(compute_shard_id(&run_id))),
        EventType::Step(StepEventType::Completed),
        EventSource::System,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;
    Ok(())
}

/// Publishes a StepFailedEvent for the given step, through the outbox.
pub async fn publish_step_failed_event(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    error: String,
    pool: &PgPool,
    _nats_client: async_nats::Client,
) -> Result<()> {
    let event = StepFailedEvent {
        run_id,
        step_id,
        fencing_token,
        completion_token: crate::db::step_completion_token(pool, step_id).await,
        event_type: EventType::Step(StepEventType::Failed),
        error,
        runner_id: None,
        exit_code: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: None,
        timestamp: Utc::now(),
    };

    crate::db::outbox::insert_outbox_event(
        pool,
        NatsSubject::StepFailed(Some(compute_shard_id(&run_id))),
        EventType::Step(StepEventType::Failed),
        EventSource::System,
        serde_json::to_value(event)?,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await?;
    Ok(())
}
