use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use sqlx::PgPool;

use crate::handler::fetch_step_instance;

#[cfg(feature = "chatops-slack")]
pub async fn handle_slack_message(
    run_id: stormchaser_model::RunId,
    step_instance_id: stormchaser_model::StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use stormchaser_model::dsl::SlackMessageSpec;
    use stormchaser_model::Connection;
    let spec: SlackMessageSpec = serde_json::from_value(spec)?;

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

    // Post to Slack Webhook
    let client = reqwest::Client::new();

    #[derive(serde::Serialize)]
    struct SlackPayload {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        blocks: Option<Value>,
    }

    let payload = SlackPayload {
        text: spec.message,
        blocks: spec.blocks,
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
        anyhow::bail!("Slack API error {}: {}", status, error_body);
    }
}
