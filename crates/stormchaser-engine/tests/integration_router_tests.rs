use anyhow::Result;
use chrono::Utc;
use cloudevents::EventBuilder;
use serde_json::json;
use sqlx::PgPool;
use std::env::var;
use std::time::Duration;
use stormchaser_engine::config::Config;
use stormchaser_engine::server::run_engine;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use uuid::Uuid;

async fn setup_db() -> Result<PgPool> {
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        dotenvy::dotenv().ok();
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD").expect("STORMCHASER_DEV_PASSWORD must be set")
        )
    });
    let pool = PgPool::connect(&db_url).await?;
    Ok(pool)
}

#[tokio::test]
async fn test_router_end_to_end() -> Result<()> {
    let pool = setup_db().await?;

    // Load config from env
    dotenvy::dotenv().ok();

    // Ensure DATABASE_URL is set in env for Config
    let db_url = var("DATABASE_URL").unwrap_or_else(|_| {
        format!(
            "postgres://stormchaser:{}@localhost:5432/stormchaser",
            var("STORMCHASER_DEV_PASSWORD").expect("STORMCHASER_DEV_PASSWORD must be set")
        )
    });
    std::env::set_var("DATABASE_URL", db_url);

    let mut config = Config::from_env(std::env::vars())?;

    // NATS configuration
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    config.nats_url = nats_url.clone();
    config.rust_log = "debug".to_string();

    let nats_client = async_nats::connect(&nats_url).await?;

    // Spawn the engine in the background
    tokio::spawn(async move {
        let _ = run_engine(config).await;
    });

    // Give it a moment to boot and subscribe to JetStream
    tokio::time::sleep(Duration::from_secs(4)).await;

    // We generate a unique RunId
    let run_id = RunId::new_v4();

    // Insert dummy run into the DB manually so the engine finds it
    stormchaser_engine::db::insert_workflow_run(
        &pool,
        run_id,
        "TestWorkflow",
        Some("user@example.com"),
        Some("https://github.com/example/repo.git"),
        Some("test.storm"),
        Some("main"),
        RunStatus::Queued,
        Some(0),
        Utc::now(),
        Utc::now(),
        None,
    )
    .await?;

    // Construct a valid cloudevent payload
    let event = json!({
        "run_id": run_id.to_string(),
        "event_type": "queued",
        "timestamp": Utc::now().to_rfc3339(),
        "step_definitions": {},
        "inputs": {}
    });

    // We must wrap it in a proper CloudEvent format
    let ce = cloudevents::EventBuilderV10::new()
        .id(Uuid::new_v4().to_string())
        .source("integration_test")
        .ty("stormchaser.v1.run.queued")
        .data("application/json", cloudevents::Data::Json(event))
        .build()
        .unwrap();

    let payload = serde_json::to_vec(&ce).unwrap();

    // Send the message over NATS to the engine loop
    let js = async_nats::jetstream::new(nats_client);
    let subject = stormchaser_model::nats::NatsSubject::RunQueued(Some(
        stormchaser_model::nats::compute_shard_id(&run_id),
    ))
    .as_str()
    .to_string();

    js.publish(subject, payload.into()).await?;

    // Wait for the engine to process it
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Check if the run was updated by the engine
    let run = stormchaser_engine::handler::fetch_run(run_id, &pool)
        .await
        .unwrap();

    assert!(
        run.status != RunStatus::Queued,
        "The router failed to process the NATS message. Run {} status did not change from Queued.",
        run_id
    );

    Ok(())
}
