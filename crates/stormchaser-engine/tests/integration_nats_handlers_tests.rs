#![allow(clippy::explicit_auto_deref)]
use futures::StreamExt;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::env::var;
use stormchaser_engine::handler::step::events::query::handle_step_query;
use stormchaser_model::{RunId, StepInstanceId};

async fn setup_db() -> sqlx::PgPool {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD")
                .expect("STORMCHASER_DEV_PASSWORD must be set if DATABASE_URL is not set")
        )
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .unwrap()
}

#[tokio::test]
async fn test_handle_step_query_ephemeral_reply() {
    let pool = setup_db().await;
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());
    let nats_client = async_nats::connect(nats_url).await.unwrap();

    let run_id = RunId::new_v4();
    let step_id = StepInstanceId::new_v4();

    // Insert dummy run and step so that the query handler finds something
    sqlx::query(
        "INSERT INTO workflow_runs (id, workflow_name, initiating_user, status, fencing_token, repo_url, workflow_path, git_ref) VALUES ($1, 'test', 'test', 'running', 1, 'http://example.com', 'test.storm', 'main')"
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO step_instances (id, run_id, step_name, step_type, status, spec, params) VALUES ($1, $2, 'test-step', 'RunContainer', 'pending', '{}', '{}')"
    )
    .bind(step_id)
    .bind(run_id)
    .execute(&pool)
    .await
    .unwrap();

    // Create a temporary ephemeral subject simulating what a runner would use for Request-Reply
    let reply_subject = nats_client.new_inbox();
    let mut subscriber = nats_client.subscribe(reply_subject.clone()).await.unwrap();

    // Construct the payload that a runner sends
    let payload = json!({
        "step_id": step_id.to_string(),
    });

    // Call the handler directly, simulating the NATS listener loop calling it
    handle_step_query(
        payload,
        pool.clone(),
        nats_client.clone(),
        Some(reply_subject),
    )
    .await
    .unwrap();

    // We expect to receive the StepQueryResponse on the ephemeral subject
    let timeout = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.next()).await;
    let msg = timeout
        .expect("Timed out waiting for NATS reply")
        .expect("NATS stream closed");

    let response: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
    assert_eq!(
        response["data"]["step_id"].as_str(),
        Some(step_id.to_string().as_str())
    );
    assert_eq!(response["data"]["exists"].as_bool(), Some(true));
    assert_eq!(response["data"]["status"].as_str(), Some("pending"));

    // Cleanup
    sqlx::query("DELETE FROM step_instances WHERE run_id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("DELETE FROM workflow_runs WHERE id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .unwrap();
}
