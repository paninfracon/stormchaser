use anyhow::Context;
use futures::StreamExt;
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use sqlx::ConnectOptions;
use std::path::PathBuf;
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
use tokio::time::sleep;
use tracing::info;
use uuid::Uuid;

use stormchaser_engine::db;
use stormchaser_engine::git_cache;
use stormchaser_engine::hcl_eval;
use stormchaser_engine::parse_duration;
use stormchaser_engine::secrets;
use stormchaser_engine::secrets::VaultBackend;
use stormchaser_model::auth;
use stormchaser_model::LogBackend;
use stormchaser_opa::OpaWasmInstance;
use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

#[derive(Debug, Clone)]
/// Config.
pub struct Config {
    /// The database url.
    pub database_url: String,
    /// The tls ca cert path.
    pub tls_ca_cert_path: Option<PathBuf>,
    /// The tls cert path.
    pub tls_cert_path: PathBuf,
    /// The tls key path.
    pub tls_key_path: PathBuf,
    /// The tls server name.
    pub tls_server_name: Option<String>,
    /// The db ssl.
    pub db_ssl: bool,
    /// The git cache dir.
    pub git_cache_dir: PathBuf,
    /// The opa url.
    pub opa_url: Option<String>,
    /// The opa wasm path.
    pub opa_wasm_path: Option<PathBuf>,
    /// The opa entrypoint.
    pub opa_entrypoint: Option<String>,
    /// The loki url.
    pub loki_url: Option<String>,
    /// The elasticsearch url.
    pub elasticsearch_url: Option<String>,
    /// The elasticsearch index.
    pub elasticsearch_index: Option<String>,
    /// The vault addr.
    pub vault_addr: String,
    /// The vault token.
    pub vault_token: String,
    /// The nats url.
    pub nats_url: String,
    /// The rust log.
    pub rust_log: String,
}

impl Config {
    /// From env.
    pub fn from_env<I, K, V>(env: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut database_url = None;
        let mut tls_ca_cert_path = None;
        let mut tls_cert_path = PathBuf::from("/etc/engine/certs/tls.crt");
        let mut tls_key_path = PathBuf::from("/etc/engine/certs/tls.key");
        let mut tls_server_name = None;
        let mut db_ssl = false;
        let mut git_cache_dir = PathBuf::from("/tmp/stormchaser/git-cache");
        let mut opa_url = None;
        let mut opa_wasm_path = None;
        let mut opa_entrypoint = None;
        let mut loki_url = None;
        let mut elasticsearch_url = None;
        let mut elasticsearch_index = None;
        let mut vault_addr = "http://localhost:8200".to_string();
        let mut vault_token = "root".to_string();
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut rust_log = "stormchaser_engine=debug".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "DATABASE_URL" => database_url = Some(v.as_ref().to_string()),
                "TLS_CA_CERT_PATH" => tls_ca_cert_path = Some(PathBuf::from(v.as_ref())),
                "TLS_CERT_PATH" => tls_cert_path = PathBuf::from(v.as_ref()),
                "TLS_KEY_PATH" => tls_key_path = PathBuf::from(v.as_ref()),
                "TLS_SERVER_NAME" => tls_server_name = Some(v.as_ref().to_string()),
                "STORMCHASER_DB_SSL" => db_ssl = v.as_ref() == "true",
                "GIT_CACHE_DIR" => git_cache_dir = PathBuf::from(v.as_ref()),
                "OPA_URL" => opa_url = Some(v.as_ref().to_string()),
                "OPA_WASM_PATH" => opa_wasm_path = Some(PathBuf::from(v.as_ref())),
                "OPA_ENTRYPOINT" => opa_entrypoint = Some(v.as_ref().to_string()),
                "LOKI_URL" => loki_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_URL" => elasticsearch_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_INDEX" => elasticsearch_index = Some(v.as_ref().to_string()),
                "VAULT_ADDR" => vault_addr = v.as_ref().to_string(),
                "VAULT_TOKEN" => vault_token = v.as_ref().to_string(),
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                _ => {}
            }
        }

        Ok(Self {
            database_url: database_url.context("DATABASE_URL must be set")?,
            tls_ca_cert_path,
            tls_cert_path,
            tls_key_path,
            tls_server_name,
            db_ssl,
            git_cache_dir,
            opa_url,
            opa_wasm_path,
            opa_entrypoint,
            loki_url,
            elasticsearch_url,
            elasticsearch_index,
            vault_addr,
            vault_token,
            nats_url,
            rust_log,
        })
    }
}

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

/// Run engine.
pub async fn run_engine(config: Config) -> anyhow::Result<()> {
    let tls_config = TlsConfig {
        ca_cert_path: config.tls_ca_cert_path.clone(),
        cert_path: config.tls_cert_path.clone(),
        key_path: config.tls_key_path.clone(),
        server_name: config.tls_server_name.clone(),
    };

    let tls_reloader = Arc::new(TlsReloader::new(tls_config).await?);

    let mut db_options: sqlx::postgres::PgConnectOptions = config.database_url.parse()?;
    if config.db_ssl {
        if let Some(ca) = &config.tls_ca_cert_path {
            db_options = db_options
                .ssl_mode(sqlx::postgres::PgSslMode::VerifyFull)
                .ssl_root_cert(ca.to_string_lossy().to_string());
        }

        // sqlx 0.8 supports providing client certs for mTLS
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

    let git_cache = Arc::new(GitCache::new(config.git_cache_dir.clone()));

    let mut opa_client = OpaClient::new(config.opa_url.clone(), Some(tls_reloader.client_config()));

    if let Some(wasm_path) = &config.opa_wasm_path {
        tracing::info!("Loading OPA WASM policy from {:?}", wasm_path);
        let wasm_bytes = std::fs::read(wasm_path).context("Failed to read OPA WASM policy")?;
        let executor = OpaWasmInstance::new(&wasm_bytes)?;
        opa_client = opa_client.with_wasm_executor(Arc::new(executor));
    }

    if let Some(entrypoint) = config.opa_entrypoint {
        opa_client = opa_client.with_entrypoint(entrypoint);
    }

    let opa_client = Arc::new(opa_client);

    // Log Backend Configuration
    let mut log_backend = None;
    if let Some(url) = config.loki_url {
        tracing::info!("Configuring Loki log backend: {}", url);
        log_backend = Some(LogBackend::Loki { url });
    } else if let (Some(url), Some(index)) = (config.elasticsearch_url, config.elasticsearch_index)
    {
        tracing::info!(
            "Configuring Elasticsearch log backend: {} (index: {})",
            url,
            index
        );
        log_backend = Some(LogBackend::Elasticsearch { url, index });
    }
    let log_backend = Arc::new(log_backend);

    // Initialize Secret Backend
    let secret_backend = Arc::new(VaultBackend::new(config.vault_addr, config.vault_token)?)
        as secrets::SharedSecretBackend;
    hcl_eval::set_secrets_backend(secret_backend);

    let nats_options = async_nats::ConnectOptions::new()
        .tls_client_config((*tls_reloader.client_config()).clone());

    let nats_client = async_nats::connect_with_options(config.nats_url, nats_options).await?;

    tracing::info!(
        "Stormchaser Orchestration Engine {} starting (rev: {}, branch: {}, built: {})",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_BUILD_TIMESTAMP")
    );

    // Background task for runner liveness (marking as offline after inactivity)
    let liveness_pool = pool.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let result = db::mark_stale_runners_offline(
                &liveness_pool,
                RunnerStatus::Offline,
                RunnerStatus::Online,
            )
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

    // Background task for workflow timeouts
    let timeout_pool = pool.clone();
    let timeout_nats = nats_client.clone();
    let timeout_tls_reloader = tls_reloader.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;

            // Find all non-terminal runs and check their timeouts
            #[derive(sqlx::FromRow)]
            struct TimeoutCheck {
                id: Uuid,
                #[sqlx(rename = "status")]
                _status: RunStatus,
                created_at: chrono::DateTime<chrono::Utc>,
                started_at: Option<chrono::DateTime<chrono::Utc>>,
                timeout: String,
            }

            let result = db::get_active_workflow_runs_with_quotas(&timeout_pool)
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
                                run.id,
                                timeout_pool.clone(),
                                timeout_nats.clone(),
                                timeout_tls_reloader.clone(),
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

    let js = nats::init_jetstream(&nats_client).await?;

    // Use JetStream for events
    let stream = js.get_stream("stormchaser").await?;
    let consumer = stream
        .get_or_create_consumer(
            "orchestration-engine",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("orchestration-engine".to_string()),
                filter_subject: "stormchaser.>".to_string(),
                ..Default::default()
            },
        )
        .await?;

    let mut messages = consumer.messages().await?;

    // Standard NATS subscriber for Request/Reply (queries)
    let mut query_subscriber = nats_client.subscribe("stormchaser.step.query").await?;

    info!("Engine listening for events and queries");

    loop {
        tokio::select! {
            message = messages.next() => {
                match message {
                    Some(Ok(message)) => {
                        let subject = message.subject.to_string();
                        // Ignore tasks meant for runners
                        if subject.starts_with("stormchaser.step.scheduled.") {
                            let _ = message.ack().await;
                            continue;
                        }

                        tracing::debug!("Received event on {}: {:?}", subject, message.payload);

                        let payload: Value = match serde_json::from_slice(&message.payload) {
                            Ok(p) => p,
                            Err(e) => {
                                tracing::error!(
                                    "Failed to parse payload from {}: {:?}. Payload: {:?}",
                                    subject,
                                    e,
                                    String::from_utf8_lossy(&message.payload)
                                );
                                let _ = message.ack().await;
                                continue;
                            }
                        };


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
                    let payload: Value = match serde_json::from_slice(&message.payload) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("Failed to parse query payload: {:?}", e);
                            continue;
                        }
                    };
                    let pool = pool.clone();
                    let nats_client = nats_client.clone();
                    let reply = message.reply.clone().map(|r| r.to_string());
                    tokio::spawn(async move {
                        if let Err(e) = handler::handle_step_query(payload, pool, nats_client, reply).await
                        {
                            tracing::error!("Failed to handle step query: {:?}", e);
                        }
                    });
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    subject: &str,
    payload: Value,
    message: async_nats::jetstream::message::Message,
    pool: sqlx::PgPool,
    git_cache: Arc<git_cache::GitCache>,
    opa_client: Arc<auth::OpaClient>,
    nats_client: async_nats::Client,
    tls_reloader: Arc<TlsReloader>,
    log_backend: Arc<Option<LogBackend>>,
) {
    match subject {
        "stormchaser.run.queued" => {
            let run_id_str = match payload["run_id"].as_str() {
                Some(id) => id,
                None => {
                    let _ = message.double_ack().await;
                    return;
                }
            };
            let run_id = match Uuid::parse_str(run_id_str) {
                Ok(id) => id,
                Err(_) => {
                    let _ = message.double_ack().await;
                    return;
                }
            };

            tokio::spawn(async move {
                if let Err(e) = handler::handle_workflow_queued(
                    run_id,
                    pool,
                    git_cache,
                    opa_client,
                    nats_client,
                    tls_reloader,
                )
                .await
                {
                    tracing::error!(
                        "Failed to handle workflow queued event for {}: {:?}",
                        run_id,
                        e
                    );
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.run.direct" => {
            let run_id_str = match payload["run_id"].as_str() {
                Some(id) => id,
                None => {
                    let _ = message.double_ack().await;
                    return;
                }
            };
            let run_id = match Uuid::parse_str(run_id_str) {
                Ok(id) => id,
                Err(_) => {
                    let _ = message.double_ack().await;
                    return;
                }
            };

            tokio::spawn(async move {
                if let Err(e) =
                    handler::handle_workflow_direct(payload, pool, opa_client, nats_client).await
                {
                    tracing::error!(
                        "Failed to handle workflow direct event for {}: {:?}",
                        run_id,
                        e
                    );
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.run.start_pending" => {
            let run_id_str = match payload["run_id"].as_str() {
                Some(id) => id,
                None => {
                    let _ = message.double_ack().await;
                    return;
                }
            };
            let run_id = match Uuid::parse_str(run_id_str) {
                Ok(id) => id,
                Err(_) => {
                    let _ = message.double_ack().await;
                    return;
                }
            };

            tokio::spawn(async move {
                if let Err(e) =
                    handler::handle_workflow_start_pending(run_id, pool, nats_client, tls_reloader)
                        .await
                {
                    tracing::error!(
                        "Failed to handle workflow start_pending event for {}: {:?}",
                        run_id,
                        e
                    );
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.runner.register" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_runner_registration(payload, pool).await {
                    tracing::error!("Failed to handle runner registration: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.runner.heartbeat" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_runner_heartbeat(payload, pool).await {
                    tracing::error!("Failed to handle runner heartbeat: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.runner.offline" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_runner_offline(payload, pool).await {
                    tracing::error!("Failed to handle runner offline: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.step.register_wasm" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_wasm_registration(payload, pool).await {
                    tracing::error!("Failed to handle WASM step registration: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.step.unpacking_sfs" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_unpacking_sfs(payload, pool).await {
                    tracing::error!("Failed to handle step unpacking_sfs event: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.step.packing_sfs" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_packing_sfs(payload, pool).await {
                    tracing::error!("Failed to handle step packing_sfs event: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.step.running" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_running(payload, pool).await {
                    tracing::error!("Failed to handle step running event: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.step.completed" => {
            tokio::spawn(async move {
                if let Err(e) = handler::handle_step_completed(
                    payload,
                    pool,
                    nats_client,
                    log_backend,
                    tls_reloader,
                )
                .await
                {
                    tracing::error!("Failed to handle step completed event: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        "stormchaser.step.failed" => {
            tokio::spawn(async move {
                if let Err(e) =
                    handler::handle_step_failed(payload, pool, nats_client, tls_reloader).await
                {
                    tracing::error!("Failed to handle step failed event: {:?}", e);
                }
                let _ = message.double_ack().await;
            });
        }
        _ => {
            let _ = message.double_ack().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_missing_database_url() {
        let env: Vec<(&str, &str)> = vec![];
        let config = Config::from_env(env);
        assert!(config.is_err());
        assert_eq!(config.unwrap_err().to_string(), "DATABASE_URL must be set");
    }

    #[test]
    fn test_config_from_env_valid() {
        let env = vec![
            ("DATABASE_URL", "postgres://user:pass@localhost/db"),
            ("TLS_SERVER_NAME", "engine.example.com"),
            ("STORMCHASER_DB_SSL", "true"),
            ("LOKI_URL", "http://loki:3100"),
            ("VAULT_ADDR", "http://vault:8200"),
        ];
        let config = Config::from_env(env).unwrap();
        assert_eq!(config.database_url, "postgres://user:pass@localhost/db");
        assert_eq!(
            config.tls_server_name.as_deref(),
            Some("engine.example.com")
        );
        assert!(config.db_ssl);
        assert_eq!(config.nats_url, "nats://localhost:4222");
        assert_eq!(config.loki_url.as_deref(), Some("http://loki:3100"));
        assert_eq!(config.vault_addr, "http://vault:8200");
        assert_eq!(config.vault_token, "root");
        assert_eq!(
            config.tls_cert_path,
            PathBuf::from("/etc/engine/certs/tls.crt")
        );
    }
}
