use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;

use crate::handler::fetch_step_instance;

#[cfg(feature = "chatops-teams")]
pub async fn handle_teams_message(
    run_id: stormchaser_model::RunId,
    step_instance_id: stormchaser_model::StepInstanceId,
    fencing_token: i64,
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
    #[derive(serde::Serialize)]
    struct TeamsTextBlock {
        #[serde(rename = "type")]
        block_type: String,
        text: String,
        wrap: bool,
    }

    #[derive(serde::Serialize)]
    struct TeamsAdaptiveCard {
        #[serde(rename = "$schema")]
        schema: String,
        #[serde(rename = "type")]
        card_type: String,
        version: String,
        body: Vec<TeamsTextBlock>,
    }

    #[derive(serde::Serialize)]
    struct TeamsAttachment {
        #[serde(rename = "contentType")]
        content_type: String,
        #[serde(rename = "contentUrl")]
        content_url: Option<String>,
        content: TeamsAdaptiveCard,
    }

    #[derive(serde::Serialize)]
    struct TeamsPayload {
        #[serde(rename = "type")]
        payload_type: String,
        attachments: Vec<TeamsAttachment>,
    }

    let payload = TeamsPayload {
        payload_type: "message".to_string(),
        attachments: vec![TeamsAttachment {
            content_type: "application/vnd.microsoft.card.adaptive".to_string(),
            content_url: None,
            content: TeamsAdaptiveCard {
                schema: "http://adaptivecards.io/schemas/adaptive-card.json".to_string(),
                card_type: "AdaptiveCard".to_string(),
                version: "1.2".to_string(),
                body: vec![TeamsTextBlock {
                    block_type: "TextBlock".to_string(),
                    text: spec.message,
                    wrap: true,
                }],
            },
        }],
    };

    let res = client.post(&webhook_url).json(&payload).send().await?;

    let status = res.status();
    if status.is_success() {
        let instance = fetch_step_instance(step_instance_id, &pool).await?;
        let machine =
            crate::step_machine::StepMachine::<crate::step_machine::state::Running>::from_instance(
                instance,
            );
        let _ = machine.succeed(&mut *pool.acquire().await?).await?;

        let event = stormchaser_model::events::StepCompletedEvent {
            run_id,
            step_id: step_instance_id,
            fencing_token,
            event_type: stormchaser_model::events::EventType::Step(
                stormchaser_model::events::StepEventType::Completed,
            ),
            runner_id: None,
            storage_hashes: None,
            artifacts: None,
            test_reports: None,
            outputs: Some(std::collections::HashMap::new()),
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
    } else {
        let error_body = res
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        anyhow::bail!("Teams API error {}: {}", status, error_body);
    }
}
