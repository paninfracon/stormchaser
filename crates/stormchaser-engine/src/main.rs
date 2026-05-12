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

async fn setup_nats_consumers(
    nats_client: &async_nats::Client,
) -> anyhow::Result<(
    async_nats::jetstream::consumer::pull::Stream,
    async_nats::Subscriber,
)> {
    let js = nats::init_jetstream(nats_client).await?;
    let stream = js.get_stream("stormchaser").await?;
    let consumer = stream
        .get_or_create_consumer(
            "orchestration-engine",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("orchestration-engine".to_string()),
                filter_subject: "stormchaser.v1.>".to_string(),
                ..Default::default()
            },
        )
        .await?;

    let messages = consumer.messages().await?;
    let query_subscriber = nats_client.subscribe("stormchaser.v1.step.query").await?;

    Ok((messages, query_subscriber))
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
    mut messages: async_nats::jetstream::consumer::pull::Stream,
    mut query_subscriber: async_nats::Subscriber,
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
            message = messages.next() => {
                match message {
                    Some(Ok(message)) => {
                        let subject = message.subject.to_string();
                        if subject.starts_with("stormchaser.v1.step.scheduled.") {
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
                            .get(subject.as_str())
                            .and_then(|name| event_schemas.get(*name));
                        if let Err(e) = stormchaser_model::nats::validate_against_schema(&payload, schema) {
                            tracing::error!("Rejecting CloudEvent on {}: schema validation failed: {}", subject, e);
                            let _ = message.ack().await;
                            continue;
                        }

                        handle_message(
                            subject.as_str(),
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
            message = query_subscriber.next() => {
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

    let (messages, query_subscriber) = setup_nats_consumers(&nats_client).await?;

    info!("Engine listening for events and queries");

    run_event_loop(
        messages,
        query_subscriber,
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
