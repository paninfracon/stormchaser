use anyhow::Result;
use chrono::Utc;
use sqlx::PgPool;
use stormchaser_model::events::{EventType, StepCompletedEvent, StepEventType};
use stormchaser_model::{Connection, RunId, StepInstanceId};

/// Fetches the URL from a configured HttpApi connection by name.
pub async fn fetch_http_api_url_from_connection(
    pool: &PgPool,
    connection_name: &str,
) -> Result<String> {
    let mut conn = pool.acquire().await?;
    if let Some(connection) = crate::db::connections::get_storage_backend_by_name::<
        &mut sqlx::PgConnection,
        Connection,
    >(&mut *conn, connection_name)
    .await?
    {
        if connection.connection_type == stormchaser_model::ConnectionType::HttpApi {
            if let Some(url) = connection.config.get("url").and_then(|u| u.as_str()) {
                Ok(url.to_string())
            } else {
                anyhow::bail!(
                    "Connection {} is missing 'url' in its config",
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

/// Publishes a StepCompletedEvent to NATS for the given step.
pub async fn publish_step_completed_event(
    run_id: RunId,
    step_id: StepInstanceId,
    fencing_token: i64,
    outputs: Option<std::collections::HashMap<String, serde_json::Value>>,
    nats_client: async_nats::Client,
) -> Result<()> {
    let event = StepCompletedEvent {
        run_id,
        step_id,
        fencing_token,
        event_type: EventType::Step(StepEventType::Completed),
        runner_id: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs,
        exit_code: None,
        timestamp: Utc::now(),
    };

    let js = async_nats::jetstream::new(nats_client);
    js.publish(
        "stormchaser.step.completed",
        serde_json::to_vec(&event)?.into(),
    )
    .await?;
    Ok(())
}
