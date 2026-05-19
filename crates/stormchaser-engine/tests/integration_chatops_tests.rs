#![allow(clippy::explicit_auto_deref)]
use futures::StreamExt;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use stormchaser_model::{RunId, StepInstanceId};
use uuid::Uuid;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_slack_message_integration() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let mock_server = MockServer::start().await;

    // Create an HttpApi connection
    let connection_id = Uuid::new_v4();
    let backend_name = format!("test-slack-conn-{}", connection_id);
    sqlx::query(
        r#"
        INSERT INTO connections (id, name, connection_type, config, encrypted_credentials, is_default_sfs)
        VALUES ($1, $2, 'http_api', $3, $4, FALSE)
        "#,
    )
    .bind(connection_id)
    .bind(&backend_name)
    .bind(json!({
        "url": format!("{}/services/hooks/slack", mock_server.uri())
    }))
    .bind("secret_token_123")
    .execute(&pool)
    .await
    .unwrap();

    Mock::given(method("POST"))
        .and(path("/services/hooks/slack"))
        .and(header("Content-Type", "application/json"))
        .and(body_json(json!({
            "text": "Hello Slack",
            "blocks": [{"type": "section"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&mock_server)
        .await;

    let run_id = RunId::new_v4();
    let step_instance_id = StepInstanceId::new_v4();

    // Insert mock workflow run
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, $3, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .bind("slack_test_wf")
        .bind("test")
        .execute(&pool)
        .await
        .unwrap();

    // Insert mock step instance
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status) VALUES ($1, $2, 'slack_step', 'SlackMessage', 'pending')")
        .bind(step_instance_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    // Subscribe to nats
    let mut completion_sub = nats_client
        .subscribe("stormchaser.v1.*.step.>")
        .await
        .unwrap();

    let spec = json!({
        "connection": backend_name,
        "message": "Hello Slack",
        "blocks": [{"type": "section"}]
    });

    #[cfg(feature = "chatops-slack")]
    {
        stormchaser_engine::handler::handle_slack_message(
            run_id,
            step_instance_id,
            1,
            spec,
            pool.clone(),
            nats_client.clone(),
        )
        .await
        .expect("handle_slack_message failed");
    }

    // Await completion event
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), completion_sub.next())
            .await
            .expect("Timed out waiting for NATS step completed event")
            .unwrap();

        let event: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
        if event["data"]["step_id"].as_str().unwrap() == step_instance_id.to_string() {
            break;
        }
    }
}

#[tokio::test]
async fn test_teams_message_integration() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap();

    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let mock_server = MockServer::start().await;

    // Create an HttpApi connection
    let connection_id = Uuid::new_v4();
    let backend_name = format!("test-teams-conn-{}", connection_id);
    sqlx::query(
        r#"
        INSERT INTO connections (id, name, connection_type, config, encrypted_credentials, is_default_sfs)
        VALUES ($1, $2, 'http_api', $3, $4, FALSE)
        "#,
    )
    .bind(connection_id)
    .bind(&backend_name)
    .bind(json!({
        "url": format!("{}/webhookb2/teams", mock_server.uri())
    }))
    .bind("secret_token_123")
    .execute(&pool)
    .await
    .unwrap();

    Mock::given(method("POST"))
        .and(path("/webhookb2/teams"))
        .and(header("Content-Type", "application/json"))
        .and(body_json(json!({
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
                                "text": "Hello Teams",
                                "wrap": true
                            }
                        ]
                    }
                }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&mock_server)
        .await;

    let run_id = RunId::new_v4();
    let step_instance_id = StepInstanceId::new_v4();

    // Insert mock workflow run
    sqlx::query("INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, $2, $3, 'running', 1, 'http://git.local', 'workflow.storm', 'main')")
        .bind(run_id)
        .bind("teams_test_wf")
        .bind("test")
        .execute(&pool)
        .await
        .unwrap();

    // Insert mock step instance
    sqlx::query("INSERT INTO step_instances (id, run_id, step_name, step_type, status) VALUES ($1, $2, 'teams_step', 'TeamsMessage', 'pending')")
        .bind(step_instance_id)
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    // Subscribe to nats
    let mut completion_sub = nats_client
        .subscribe("stormchaser.v1.*.step.>")
        .await
        .unwrap();

    let spec = json!({
        "connection": backend_name,
        "message": "Hello Teams"
    });

    #[cfg(feature = "chatops-teams")]
    {
        stormchaser_engine::handler::handle_teams_message(
            run_id,
            step_instance_id,
            1,
            spec,
            pool.clone(),
            nats_client.clone(),
        )
        .await
        .expect("handle_teams_message failed");
    }

    // Await completion event
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), completion_sub.next())
            .await
            .expect("Timed out waiting for NATS step completed event")
            .unwrap();

        let event: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
        if event["data"]["step_id"].as_str().unwrap() == step_instance_id.to_string() {
            break;
        }
    }
}
