use crate::config::Config;
use crate::{git_cache::GitCache, handler};
use anyhow::Context;
use futures::StreamExt;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::LogBackend;
use tokio::time::sleep;
use tracing::info;

use crate::hcl_eval;
use crate::secrets;
use crate::secrets::VaultBackend;
use stormchaser_tls::TlsReloader;

use crate::router::handle_message;
use crate::setup::*;
use crate::workers::*;

pub async fn setup_nats_consumers(
    nats_client: &async_nats::Client,
    assigned_shards: &[u32],
) -> anyhow::Result<(
    tokio::sync::mpsc::Receiver<
        Result<
            async_nats::jetstream::message::Message,
            async_nats::jetstream::consumer::pull::MessagesError,
        >,
    >,
    tokio::sync::mpsc::Receiver<async_nats::Message>,
)> {
    let js = crate::nats::init_jetstream(nats_client).await?;
    let stream = js.get_stream("stormchaser").await?;

    let (tx, rx) = tokio::sync::mpsc::channel(1000);

    for shard in assigned_shards {
        let consumer_name = format!("orchestration-engine-shard-{}", shard);
        let filter_subject = format!("stormchaser.v1.{}.>", shard);

        let consumer = stream
            .get_or_create_consumer(
                &consumer_name,
                async_nats::jetstream::consumer::pull::Config {
                    durable_name: Some(consumer_name.clone()),
                    filter_subject,
                    ..Default::default()
                },
            )
            .await?;

        use futures::StreamExt;
        let mut messages = consumer.messages().await?;
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = messages.next().await {
                if tx_clone.send(msg).await.is_err() {
                    break;
                }
            }
        });

        if *shard == 0 {
            let global_consumer_name = "orchestration-engine-global";
            let consumer = stream
                .get_or_create_consumer(
                    global_consumer_name,
                    async_nats::jetstream::consumer::pull::Config {
                        durable_name: Some(global_consumer_name.to_string()),
                        filter_subject: "stormchaser.v1.global.>".to_string(),
                        ..Default::default()
                    },
                )
                .await?;
            let mut messages = consumer.messages().await?;
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                while let Some(msg) = messages.next().await {
                    if tx_clone.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        }
    }

    for (consumer_name, filter_subject) in [
        ("orchestration-engine-legacy-run", "stormchaser.v1.run.>"),
        (
            "orchestration-engine-legacy-runner",
            "stormchaser.v1.runner.>",
        ),
        ("orchestration-engine-legacy-step", "stormchaser.v1.step.>"),
    ] {
        let consumer = stream
            .get_or_create_consumer(
                consumer_name,
                async_nats::jetstream::consumer::pull::Config {
                    durable_name: Some(consumer_name.to_string()),
                    filter_subject: filter_subject.to_string(),
                    ..Default::default()
                },
            )
            .await?;

        let mut messages = consumer.messages().await?;
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = messages.next().await {
                if tx_clone.send(msg).await.is_err() {
                    break;
                }
            }
        });
    }

    let (query_tx, query_rx) = tokio::sync::mpsc::channel(1000);
    for subject in ["stormchaser.v1.*.step.query", "stormchaser.v1.step.query"] {
        let mut query_subscriber = nats_client
            .queue_subscribe(subject.to_string(), "engine-query-group".into())
            .await?;
        let query_tx_clone = query_tx.clone();
        tokio::spawn(async move {
            while let Some(message) = query_subscriber.next().await {
                if query_tx_clone.send(message).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(query_tx);

    Ok((rx, query_rx))
}

fn parse_assigned_shards(assigned_shards_env: &str) -> anyhow::Result<Vec<u32>> {
    let mut assigned_shards = Vec::new();
    for raw_entry in assigned_shards_env.split(',') {
        let entry = raw_entry.trim();
        if entry.is_empty() {
            anyhow::bail!(
                "Invalid STORMCHASER_ASSIGNED_SHARDS value '{}': empty shard entry",
                assigned_shards_env
            );
        }
        let shard = entry.parse::<u32>().with_context(|| {
            format!(
                "Invalid STORMCHASER_ASSIGNED_SHARDS value '{}': '{}' is not a valid shard ID",
                assigned_shards_env, entry
            )
        })?;
        assigned_shards.push(shard);
    }

    if assigned_shards.is_empty() {
        anyhow::bail!(
            "Invalid STORMCHASER_ASSIGNED_SHARDS value '{}': at least one shard is required",
            assigned_shards_env
        );
    }

    Ok(assigned_shards)
}

fn normalize_subject(subject: &str) -> String {
    let parts: Vec<&str> = subject.split('.').collect();
    if parts.len() > 3 && (parts[2] == "global" || parts[2].parse::<u32>().is_ok()) {
        format!("{}.{}.{}", parts[0], parts[1], parts[3..].join("."))
    } else {
        subject.to_string()
    }
}

fn process_query_message(
    message: async_nats::Message,
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
) {
    let ce: cloudevents::Event = match serde_json::from_slice(&message.payload) {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("Failed to parse CloudEvent payload: {:?}", e);
            return;
        }
    };
    let payload: Value = if let Some(cloudevents::Data::Json(v)) = ce.data() {
        v.clone()
    } else {
        tracing::error!("Query CloudEvent data is not JSON");
        return;
    };
    let reply = message.reply.clone().map(|r| r.to_string());
    tokio::spawn(async move {
        if let Err(e) = handler::handle_step_query(payload, pool, nats_client, reply).await {
            tracing::error!("Failed to handle step query: {:?}", e);
        }
    });
}

fn build_subject_schema_map() -> std::collections::HashMap<&'static str, &'static str> {
    [
        ("stormchaser.v1.run.queued", "WorkflowQueuedEvent"),
        (
            "stormchaser.v1.run.start_pending",
            "WorkflowStartPendingEvent",
        ),
        ("stormchaser.v1.runner.register", "RunnerRegisterEvent"),
        ("stormchaser.v1.runner.heartbeat", "RunnerHeartbeatEvent"),
        ("stormchaser.v1.runner.offline", "RunnerOfflineEvent"),
        ("stormchaser.v1.step.initializing", "StepInitializingEvent"),
        ("stormchaser.v1.step.running", "StepRunningEvent"),
        ("stormchaser.v1.step.completed", "StepCompletedEvent"),
        ("stormchaser.v1.step.failed", "StepFailedEvent"),
    ]
    .into_iter()
    .collect()
}

#[allow(clippy::too_many_arguments)]
async fn run_event_loop(
    mut messages: tokio::sync::mpsc::Receiver<
        Result<
            async_nats::jetstream::message::Message,
            async_nats::jetstream::consumer::pull::MessagesError,
        >,
    >,
    mut query_messages: tokio::sync::mpsc::Receiver<async_nats::Message>,
    pool: sqlx::PgPool,
    git_cache: Arc<GitCache>,
    opa_client: Arc<OpaClient>,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
    log_backend: Arc<Option<LogBackend>>,
) {
    let event_schemas = stormchaser_model::schema_gen::generate_event_schemas();
    let subject_schema_map = build_subject_schema_map();

    loop {
        tokio::select! {
            message = messages.recv() => {
                match message {
                    Some(Ok(message)) => {
                        let subject = message.subject.to_string();
                        let normalized_subject = normalize_subject(&subject);
                        if normalized_subject.starts_with("stormchaser.v1.step.scheduled.") {
                            let _ = message.ack().await;
                            continue;
                        }

                        tracing::debug!("Received event on {}: {:?}", subject, message.payload);

                        let ce: cloudevents::Event = match serde_json::from_slice(&message.payload) {
                            Ok(e) => e,
                            Err(e) => {
                                tracing::error!("Failed to parse CloudEvent from {}: {:?}. Payload: {:?}", subject, e, String::from_utf8_lossy(&message.payload));
                                let _ = message.ack().await;
                                continue;
                            }
                        };

                        let payload: Value = if let Some(cloudevents::Data::Json(v)) = ce.data() {
                            v.clone()
                        } else {
                            tracing::error!("CloudEvent data from {} is not JSON", subject);
                            let _ = message.ack().await;
                            continue;
                        };

                        let schema = subject_schema_map
                            .get(normalized_subject.as_str())
                            .and_then(|name| event_schemas.get(*name));
                        if let Err(e) = stormchaser_model::nats::validate_against_schema(&payload, schema) {
                            tracing::error!("Rejecting CloudEvent on {}: schema validation failed: {}", subject, e);
                            let _ = message.ack().await;
                            continue;
                        }

                        handle_message(
                            normalized_subject.as_str(),
                            payload,
                            message,
                            pool.clone(),
                            git_cache.clone(),
                            opa_client.clone(),
                            nats_client.clone(),
                            tls_reloader.clone(),
                            log_backend.clone(),
                        ).await;
                    }
                    Some(Err(e)) => {
                        tracing::error!("JetStream consumer error: {:?}", e);
                        sleep(Duration::from_secs(1)).await;
                    }
                    None => {
                        tracing::error!("JetStream consumer closed");
                        break;
                    }
                }
            }
            message = query_messages.recv() => {
                if let Some(message) = message {
                    process_query_message(message, pool.clone(), nats_client.clone());
                }
            }
        }
    }
}

/// Run engine.
pub async fn run_engine(config: Config) -> anyhow::Result<()> {
    let tls_reloader = setup_tls(&config).await?;
    let pool = setup_database(&config).await?;

    let git_cache = Arc::new(GitCache::new(config.git_cache_dir.clone()));

    let opa_client = setup_opa(&config, &tls_reloader)?;
    let log_backend = setup_log_backend(&config);

    // Initialize Secret Backend
    let secret_backend = Arc::new(VaultBackend::new(
        config.vault_addr.clone(),
        config.vault_token.clone(),
    )?) as secrets::SharedSecretBackend;
    hcl_eval::set_secrets_backend(secret_backend);

    let nats_options = async_nats::ConnectOptions::new()
        .retry_on_initial_connect()
        .tls_client_config((*tls_reloader.client_config()).clone());

    let nats_client =
        async_nats::connect_with_options(config.nats_url.clone(), nats_options).await?;

    tracing::info!(
        "Stormchaser Orchestration Engine {} starting (rev: {}, branch: {}, built: {})",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_BUILD_TIMESTAMP")
    );

    let assigned_shards_env =
        std::env::var("STORMCHASER_ASSIGNED_SHARDS").unwrap_or_else(|_| "0".to_string());
    let assigned_shards = parse_assigned_shards(&assigned_shards_env)?;

    if assigned_shards.contains(&0) {
        start_liveness_worker(pool.clone(), nats_client.clone());
        start_timeout_worker(pool.clone(), nats_client.clone(), tls_reloader.clone());
        start_resolver_crash_recovery_worker(pool.clone(), nats_client.clone());
        let _outbox_relay_worker = start_outbox_relay_worker(pool.clone(), nats_client.clone());
    }

    let (messages, query_messages) = setup_nats_consumers(&nats_client, &assigned_shards).await?;

    info!("Engine listening for events and queries");

    run_event_loop(
        messages,
        query_messages,
        pool,
        git_cache,
        opa_client,
        nats_client,
        tls_reloader,
        log_backend,
    )
    .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_subject, parse_assigned_shards};

    #[test]
    fn parse_assigned_shards_trims_values() {
        let shards = parse_assigned_shards("0, 1,2").expect("shards should parse");
        assert_eq!(shards, vec![0, 1, 2]);
    }

    #[test]
    fn parse_assigned_shards_rejects_invalid_entries() {
        let error = parse_assigned_shards("0, nope").expect_err("parsing should fail");
        assert!(error.to_string().contains("not a valid shard ID"));
    }

    #[test]
    fn normalize_subject_removes_shard_segment() {
        assert_eq!(
            normalize_subject("stormchaser.v1.3.step.completed"),
            "stormchaser.v1.step.completed"
        );
        assert_eq!(
            normalize_subject("stormchaser.v1.global.runner.heartbeat"),
            "stormchaser.v1.runner.heartbeat"
        );
        assert_eq!(
            normalize_subject("stormchaser.v1.step.completed"),
            "stormchaser.v1.step.completed"
        );
    }
}
