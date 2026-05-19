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
    std::env::set_var("RUST_LOG", "debug");
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

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

    std::env::set_var("TLS_CERT_PATH", "../../tests/certs/tls.crt");
    std::env::set_var("TLS_KEY_PATH", "../../tests/certs/tls.key");
    std::env::set_var("TLS_CA_CERT_PATH", "../../tests/certs/ca.crt");

    let mut config = Config::from_env(std::env::vars())?;

    // NATS configuration
    let nats_url = var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    config.nats_url = nats_url.clone();
    config.rust_log = "debug".to_string();

    std::env::set_var("STORMCHASER_ASSIGNED_SHARDS", "999");
    std::env::set_var("RUST_LOG", "debug");

    let nats_client = async_nats::connect(&nats_url).await?;

    // Spawn the engine in the background
    tokio::spawn(async move {
        if let Err(e) = run_engine(config).await {
            tracing::error!("Engine failed to start: {:?}", e);
        }
    });

    // Poll for the consumer to become available rather than using a fixed sleep.
    // When the consumer exists the engine has finished setup_nats_consumers and is
    // ready to receive messages.
    const MAX_CONSUMER_READY_RETRIES: u32 = 60;
    const CONSUMER_POLL_INTERVAL_MS: u64 = 200;
    let js_poll = async_nats::jetstream::new(nats_client.clone());
    let consumer_name = "orchestration-engine-shard-999";
    let mut consumer_ready = false;
    for _ in 0..MAX_CONSUMER_READY_RETRIES {
        let consumer_res: Result<
            async_nats::jetstream::consumer::Consumer<
                async_nats::jetstream::consumer::pull::Config,
            >,
            _,
        > = js_poll
            .get_consumer_from_stream(consumer_name, "stormchaser")
            .await;
        if consumer_res.is_ok() {
            consumer_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(CONSUMER_POLL_INTERVAL_MS)).await;
    }
    assert!(
        consumer_ready,
        "NATS consumer '{}' did not become ready within {}ms",
        consumer_name,
        MAX_CONSUMER_READY_RETRIES as u64 * CONSUMER_POLL_INTERVAL_MS
    );

    // We generate a unique RunId
    let run_id = RunId::new_v4();

    // Insert dummy run into the DB manually so the engine finds it
    stormchaser_engine::db::insert_workflow_run(
        &pool,
        run_id,
        "TestWorkflow",
        Some("user@example.com"),
        Some("git-local:///tmp/nonexistent-repo"),
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
        "status": "queued",
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
    let subject = stormchaser_model::nats::NatsSubject::RunQueued(Some(999))
        .as_str()
        .to_string();

    js.publish(subject, payload.into()).await?;

    // Wait for the engine to process it
    let mut run_updated = false;
    for _ in 0..100 {
        let run = stormchaser_engine::handler::fetch_run(run_id, &pool)
            .await
            .unwrap();
        if run.status != RunStatus::Queued {
            run_updated = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        run_updated,
        "The router failed to process the NATS message. Run {} status did not change from Queued.",
        run_id
    );

    Ok(())
}
