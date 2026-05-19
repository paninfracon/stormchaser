use serde_json::Value;

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

fn build_teams_payload(spec: &stormchaser_model::dsl::TeamsMessageSpec) -> Value {
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
                    text: spec.message.clone(),
                    wrap: true,
                }],
            },
        }],
    };
    serde_json::to_value(payload).unwrap()
}

#[cfg(feature = "chatops-teams")]
use crate::handler::fetch_step_instance;
#[cfg(feature = "chatops-teams")]
use anyhow::Result;
#[cfg(feature = "chatops-teams")]
use sqlx::PgPool;
#[cfg(feature = "chatops-teams")]
pub async fn handle_teams_message(
    run_id: stormchaser_model::RunId,
    step_instance_id: stormchaser_model::StepInstanceId,
    fencing_token: i64,
    spec: Value,
    pool: PgPool,
    nats_client: async_nats::Client,
) -> Result<()> {
    use crate::step_machine::{state::Pending, StepMachine};
    use std::collections::HashMap;
    use stormchaser_model::dsl::TeamsMessageSpec;

    let spec: TeamsMessageSpec = serde_json::from_value(spec)?;

    // Fetch instance
    let instance = fetch_step_instance(step_instance_id, &pool).await?;

    let machine = StepMachine::<Pending>::from_instance(instance);
    let mut conn = pool.acquire().await?;
    let _machine = machine.start("system".to_string(), &mut *conn).await?;

    let webhook_url =
        super::utils::fetch_http_api_url_from_connection(&pool, &spec.connection).await?;

    // Post to Teams Webhook
    let client = reqwest::Client::new();
    let payload = build_teams_payload(&spec);

    let res = client.post(&webhook_url).json(&payload).send().await?;

    let status = res.status();
    if status.is_success() {
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
        anyhow::bail!("Teams API error {}: {}", status, error_body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::dsl::TeamsMessageSpec;

    #[test]
    fn test_build_teams_payload() {
        let spec = TeamsMessageSpec {
            connection: "teams_conn".to_string(),
            message: "Hello, Teams!".to_string(),
        };

        let payload = build_teams_payload(&spec);

        assert_eq!(payload["type"], "message");
        let attachments = payload["attachments"]
            .as_array()
            .expect("attachments array");
        assert_eq!(attachments.len(), 1);

        let card = &attachments[0]["content"];
        assert_eq!(card["type"], "AdaptiveCard");
        assert_eq!(card["version"], "1.2");

        let body = card["body"].as_array().expect("body array");
        assert_eq!(body.len(), 1);
        assert_eq!(body[0]["type"], "TextBlock");
        assert_eq!(body[0]["text"], "Hello, Teams!");
        assert_eq!(body[0]["wrap"], true);
    }
}
