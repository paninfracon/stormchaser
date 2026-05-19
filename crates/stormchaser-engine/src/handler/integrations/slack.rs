use anyhow::Result;
use serde_json::Value;
use sqlx::PgPool;

use crate::handler::fetch_step_instance;

#[derive(serde::Serialize)]
struct SlackPayload {
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocks: Option<Value>,
}

fn build_slack_payload(spec: &stormchaser_model::dsl::SlackMessageSpec) -> Value {
    let payload = SlackPayload {
        text: spec.message.clone(),
        blocks: spec.blocks.clone(),
    };
    serde_json::to_value(payload).unwrap()
}

#[cfg(feature = "chatops-slack")]
pub async fn handle_slack_message(
    run_id: stormchaser_model::RunId,
    step_instance_id: stormchaser_model::StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use crate::step_machine::{
        state::{Pending, Running},
        StepMachine,
    };
    use std::collections::HashMap;
    use stormchaser_model::dsl::SlackMessageSpec;

    let spec: SlackMessageSpec = serde_json::from_value(spec)?;

    // Fetch instance
    let instance = fetch_step_instance(step_instance_id, &pool).await?;

    let machine = StepMachine::<Pending>::from_instance(instance);
    let mut conn = pool.acquire().await?;
    let _machine = machine.start("system".to_string(), &mut *conn).await?;

    let webhook_url =
        super::utils::fetch_http_api_url_from_connection(&pool, &spec.connection).await?;

    // Post to Slack Webhook
    let client = reqwest::Client::new();
    let payload = build_slack_payload(&spec);

    let res = client.post(&webhook_url).json(&payload).send().await?;

    let status = res.status();
    if status.is_success() {
        let instance = fetch_step_instance(step_instance_id, &pool).await?;
        let machine = StepMachine::<Running>::from_instance(instance);
        let _ = machine.succeed(&mut *pool.acquire().await?).await?;

        super::utils::publish_step_completed_event(
            run_id,
            step_instance_id,
            fencing_token,
            Some(HashMap::new()),
            nats_client,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stormchaser_model::dsl::SlackMessageSpec;

    #[test]
    fn test_build_slack_payload() {
        let spec = SlackMessageSpec {
            connection: "slack_conn".to_string(),
            message: "Hello, World!".to_string(),
            blocks: Some(json!([{"type": "section"}])),
        };

        let payload = build_slack_payload(&spec);

        assert_eq!(payload["text"], "Hello, World!");
        assert_eq!(payload["blocks"].as_array().unwrap().len(), 1);
        assert_eq!(payload["blocks"][0]["type"], "section");
    }

    #[test]
    fn test_build_slack_payload_no_blocks() {
        let spec = SlackMessageSpec {
            connection: "slack_conn".to_string(),
            message: "Hello, World!".to_string(),
            blocks: None,
        };

        let payload = build_slack_payload(&spec);

        assert_eq!(payload["text"], "Hello, World!");
        assert!(payload.get("blocks").is_none());
    }
}
