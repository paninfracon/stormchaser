use anyhow::Context;
use futures::StreamExt;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::ConnectOptions;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_engine::{
    git_cache::GitCache,
    handler, nats,
    telemetry::{init_telemetry, shutdown_telemetry},
};
use stormchaser_model::auth::OpaClient;
use stormchaser_model::runner::RunnerStatus;
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::LogBackend;
use stormchaser_model::RunId;
use tokio::time::sleep;
use tracing::info;
use uuid::Uuid;

use stormchaser_engine::db;
use stormchaser_engine::hcl_eval;
use stormchaser_engine::parse_duration;
use stormchaser_engine::secrets;
use stormchaser_engine::secrets::VaultBackend;
use stormchaser_opa::OpaWasmInstance;
use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

mod config;
mod router;

use config::Config;
use router::handle_message;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    let config = Config::from_env(std::env::vars())?;

    init_telemetry(&config.rust_log)?;

    let result = run_engine(config).await;
    shutdown_telemetry();
    result
}

async fn setup_tls(config: &Config) -> anyhow::Result<Arc<TlsReloader>> {
    let tls_config = TlsConfig {
        ca_cert_path: config.tls_ca_cert_path.clone(),
        cert_path: config.tls_cert_path.clone(),
        key_path: config.tls_key_path.clone(),
        server_name: config.tls_server_name.clone(),
    };

    Ok(Arc::new(TlsReloader::new(tls_config).await?))
}

async fn setup_database(config: &Config) -> anyhow::Result<sqlx::PgPool> {
    let mut db_options: sqlx::postgres::PgConnectOptions = config.database_url.parse()?;
    if config.db_ssl {
        if let Some(ca) = &config.tls_ca_cert_path {
            db_options = db_options
                .ssl_mode(sqlx::postgres::PgSslMode::VerifyFull)
                .ssl_root_cert(ca.to_string_lossy().to_string());
        }

        db_options = db_options
            .ssl_client_cert(config.tls_cert_path.clone())
            .ssl_client_key(config.tls_key_path.clone());
    } else {
        db_options = db_options.ssl_mode(sqlx::postgres::PgSslMode::Disable);
    }

    db_options = db_options
        .log_statements(log::LevelFilter::Debug)
        .log_slow_statements(log::LevelFilter::Warn, Duration::from_secs(1));

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(db_options)
        .await?;

    tracing::info!("Running database migrations...");
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("Database migrations completed successfully");

    Ok(pool)
}

fn setup_opa(config: &Config, tls_reloader: &TlsReloader) -> anyhow::Result<Arc<OpaClient>> {
    let mut opa_client = OpaClient::new(config.opa_url.clone(), Some(tls_reloader.client_config()));

    if let Some(wasm_path) = &config.opa_wasm_path {
        tracing::info!("Loading OPA WASM policy from {:?}", wasm_path);
        let wasm_bytes = std::fs::read(wasm_path).context("Failed to read OPA WASM policy")?;
        let executor = OpaWasmInstance::new(&wasm_bytes)?;
        opa_client = opa_client.with_wasm_executor(Arc::new(executor));
    }

    if let Some(entrypoint) = config.opa_entrypoint.clone() {
        opa_client = opa_client.with_entrypoint(entrypoint);
    }

    Ok(Arc::new(opa_client))
}

fn setup_log_backend(config: &Config) -> Arc<Option<LogBackend>> {
    let mut log_backend = None;
    if let Some(url) = config.loki_url.clone() {
        tracing::info!("Configuring Loki log backend: {}", url);
        log_backend = Some(LogBackend::Loki { url });
    } else if let (Some(url), Some(index)) = (
        config.elasticsearch_url.clone(),
        config.elasticsearch_index.clone(),
    ) {
        tracing::info!(
            "Configuring Elasticsearch log backend: {} (index: {})",
            url,
            index
        );
        log_backend = Some(LogBackend::Elasticsearch { url, index });
    }
    Arc::new(log_backend)
}

fn start_liveness_worker(pool: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let result =
                db::mark_stale_runners_offline(&pool, RunnerStatus::Offline, RunnerStatus::Online)
                    .await;

            match result {
                Ok(res) => {
                    let affected = res.rows_affected();
                    if affected > 0 {
                        tracing::info!(
                            "Marked {} runners as offline due to heartbeat timeout",
                            affected
                        );
                    }
                }
                Err(e) => tracing::error!("Failed to check runner liveness: {:?}", e),
            }
        }
    });
}

fn start_timeout_worker(
    pool: sqlx::PgPool,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;

            #[derive(sqlx::FromRow)]
            struct TimeoutCheck {
                id: Uuid,
                #[sqlx(rename = "status")]
                _status: RunStatus,
                created_at: chrono::DateTime<chrono::Utc>,
                started_at: Option<chrono::DateTime<chrono::Utc>>,
                timeout: String,
            }

            let result = db::get_active_workflow_runs_with_quotas(&pool)
                .await
                .map(|v: Vec<TimeoutCheck>| v);

            match result {
                Ok(runs) => {
                    for run in runs {
                        let duration_res = parse_duration(&run.timeout);
                        let duration = match duration_res {
                            Ok(d) => d,
                            Err(e) => {
                                tracing::error!(
                                    "Failed to parse timeout '{}' for run {}: {:?}",
                                    run.timeout,
                                    run.id,
                                    e
                                );
                                continue;
                            }
                        };

                        let start_time = run.started_at.unwrap_or(run.created_at);
                        let elapsed = chrono::Utc::now() - start_time;

                        if elapsed
                            > chrono::Duration::from_std(duration)
                                .unwrap_or_else(|_| chrono::Duration::zero())
                        {
                            if let Err(e) = handler::handle_workflow_timeout(
                                RunId::new(run.id),
                                pool.clone(),
                                nats_client.clone(),
                                tls_reloader.clone(),
                            )
                            .await
                            {
                                tracing::error!(
                                    "Failed to handle timeout for run {}: {:?}",
                                    run.id,
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("Failed to fetch runs for timeout check: {:?}", e),
            }
        }
    });
}

fn start_resolver_crash_recovery_worker(pool: sqlx::PgPool, nats_client: async_nats::Client) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;

            match db::get_stalled_resolving_runs(&pool, 5).await {
                Ok(run_ids) => {
                    for run_id in run_ids {
                        tracing::warn!("Recovering crashed resolution for run {}", run_id);
                        if let Ok(run) = handler::fetch_run(run_id, &pool).await {
                            if run.status == RunStatus::Resolving {
                                let machine = stormchaser_engine::workflow_machine::WorkflowMachine::<
                                    stormchaser_engine::workflow_machine::state::Resolving,
                                >::new_from_run(run);
                                if let Ok(mut tx) = pool.begin().await {
                                    let result = machine
                                        .fail(
                                            "Engine crashed during resolution".to_string(),
                                            &mut tx,
                                        )
                                        .await;
                                    if result.is_ok() && tx.commit().await.is_ok() {
                                        // Emit failed event
                                        let event = stormchaser_model::events::WorkflowFailedEvent {
                                            run_id,
                                            event_type: stormchaser_model::events::EventType::Workflow(stormchaser_model::events::WorkflowEventType::Failed),
                                            timestamp: chrono::Utc::now(),
                                        };
                                        let js = async_nats::jetstream::new(nats_client.clone());
                                        let _ = stormchaser_model::nats::publish_cloudevent(
                                            &js,
                                            stormchaser_model::nats::NatsSubject::RunFailed(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
                                            stormchaser_model::events::EventType::Workflow(stormchaser_model::events::WorkflowEventType::Failed),
                                            stormchaser_model::events::EventSource::System,
                                            serde_json::to_value(event).unwrap(),
                                            Some(stormchaser_model::events::SchemaVersion::new("1.0".to_string())),
                                            None,
                                        ).await;
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("Failed to fetch stalled resolving runs: {:?}", e),
            }
        }
    });
}

async fn setup_nats_consumers(
    nats_client: &async_nats::Client,
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

    let assigned_shards_env =
        std::env::var("STORMCHASER_ASSIGNED_SHARDS").unwrap_or_else(|_| "0".to_string());
    let assigned_shards = parse_assigned_shards(&assigned_shards_env)?;

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

        if shard == 0 {
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
        let mut query_subscriber = nats_client.subscribe(subject).await?;
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

    start_liveness_worker(pool.clone());
    start_timeout_worker(pool.clone(), nats_client.clone(), tls_reloader.clone());
    start_resolver_crash_recovery_worker(pool.clone(), nats_client.clone());

    let (messages, query_messages) = setup_nats_consumers(&nats_client).await?;

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
