use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;

use crate::handler::fetch_step_instance;

#[cfg(feature = "chatops-teams")]
pub async fn handle_teams_message(
    run_id: stormchaser_model::RunId,
    step_instance_id: stormchaser_model::StepInstanceId,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use stormchaser_model::dsl::TeamsMessageSpec;
    use stormchaser_model::Connection;
    let spec: TeamsMessageSpec = serde_json::from_value(spec)?;

    // Fetch instance
    let instance = fetch_step_instance(step_instance_id, &pool).await?;

    let machine =
        crate::step_machine::StepMachine::<crate::step_machine::state::Pending>::from_instance(
            instance,
        );
    let mut conn = pool.acquire().await?;
    let _machine = machine.start("system".to_string(), &mut *conn).await?;

    let webhook_url = if let Some(connection) =
        crate::db::connections::get_storage_backend_by_name::<&mut sqlx::PgConnection, Connection>(
            &mut *conn,
            &spec.connection,
        )
        .await?
    {
        if connection.connection_type == stormchaser_model::ConnectionType::HttpApi {
            if let Some(url) = connection.config.get("url").and_then(|u| u.as_str()) {
                url.to_string()
            } else {
                anyhow::bail!(
                    "Connection {} is missing 'url' in its config",
                    spec.connection
                );
            }
        } else {
            anyhow::bail!("Connection {} must be of type HttpApi", spec.connection);
        }
    } else {
        anyhow::bail!("Connection {} not found", spec.connection);
    };

    // Post to Teams Webhook
    let client = reqwest::Client::new();

    // Adaptive Card format for generic text
    let payload = serde_json::json!({
        "type": "message",
        "attachments": [
            {
                "contentType": "application/vnd.microsoft.card.adaptive",
                "contentUrl": null,
                "content": {
                    "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                    "type": "AdaptiveCard",
                    "version": "1.2",
                    "body": [
                        {
                            "type": "TextBlock",
                            "text": spec.message,
                            "wrap": true
                        }
                    ]
                }
            }
        ]
    });

    let res = client.post(&webhook_url).json(&payload).send().await?;

    let status = res.status();
    if status.is_success() {
        let instance = fetch_step_instance(step_instance_id, &pool).await?;
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
                instance,
            );
        let _ = machine.succeed(&mut *pool.acquire().await?).await?;

        let event = serde_json::json!({
            "run_id": run_id,
            "step_id": step_instance_id,
            "event_type": "step_completed",
            "outputs": {},
            "timestamp": Utc::now(),
        });
        let js = async_nats::jetstream::new(nats_client);
        js.publish("stormchaser.step.completed", event.to_string().into())
            .await?;
        Ok(())
    } else {
        let error_body = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        anyhow::bail!("Teams API error {}: {}", status, error_body);
    }
}
