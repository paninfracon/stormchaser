use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::events::{EventSource, SchemaVersion};
use stormchaser_model::events::{EventType, StepEventType, StepQueryResponseEvent};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::StepInstance;
use stormchaser_model::StepInstanceId;

/// Handles incoming queries for step status or output data over NATS.
pub async fn handle_step_query(
    payload: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    reply: Option<String>,
) -> Result<()> {
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = uuid::Uuid::parse_str(step_id_str).map(StepInstanceId::new)?;

    let step: Option<StepInstance> = crate::db::get_step_instance_by_id(&pool, step_id)
        .await
        .map(|v: Option<StepInstance>| v)?;

    if let Some(reply_subject) = reply {
        let response = if let Some(s) = step {
            StepQueryResponseEvent {
                step_id,
                status: Some(s.status),
                exists: true,
            }
        } else {
            StepQueryResponseEvent {
                step_id,
                status: None,
                exists: false,
            }
        };
        use stormchaser_model::nats::NatsSubject;
        publish_cloudevent(
            &async_nats::jetstream::new(nats_client.clone()),
            NatsSubject::Custom(reply_subject.clone()),
            EventType::Step(StepEventType::QueryResponse),
            EventSource::System,
            serde_json::to_value(response).unwrap(),
            Some(SchemaVersion::new("1.0".to_string())),
            None,
        )
        .await?;
    }

    Ok(())
}
