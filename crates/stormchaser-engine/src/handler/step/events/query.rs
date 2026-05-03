use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::PgPool;
use stormchaser_model::step::StepInstance;
use uuid::Uuid;

/// Handles incoming queries for step status or output data over NATS.
pub async fn handle_step_query(
    payload: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
    reply: Option<String>,
) -> Result<()> {
    let step_id_str = payload["step_id"].as_str().context("Missing step_id")?;
    let step_id = Uuid::parse_str(step_id_str)?;

    let step: Option<StepInstance> = crate::db::get_step_instance_by_id(&pool, step_id)
        .await
        .map(|v: Option<StepInstance>| v)?;

    if let Some(reply_subject) = reply {
        let response = if let Some(s) = step {
            serde_json::json!({
                "step_id": step_id,
                "status": s.status,
                "exists": true
            })
        } else {
            serde_json::json!({
                "step_id": step_id,
                "exists": false
            })
        };
        nats_client
            .publish(reply_subject, response.to_string().into())
            .await?;
    }

    Ok(())
}
