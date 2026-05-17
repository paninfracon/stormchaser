use crate::config::Config;
use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::ConnectOptions;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::LogBackend;
use stormchaser_opa::OpaWasmInstance;
use stormchaser_tls::TlsConfig;
use stormchaser_tls::TlsReloader;

pub async fn setup_tls(config: &Config) -> anyhow::Result<Arc<TlsReloader>> {
    let tls_config = TlsConfig {
        ca_cert_path: config.tls_ca_cert_path.clone(),
        cert_path: config.tls_cert_path.clone(),
        key_path: config.tls_key_path.clone(),
        server_name: config.tls_server_name.clone(),
    };

    Ok(Arc::new(TlsReloader::new(tls_config).await?))
}

pub async fn setup_database(config: &Config) -> anyhow::Result<sqlx::PgPool> {
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

pub fn setup_opa(config: &Config, tls_reloader: &TlsReloader) -> anyhow::Result<Arc<OpaClient>> {
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

pub fn setup_log_backend(config: &Config) -> Arc<Option<LogBackend>> {
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
