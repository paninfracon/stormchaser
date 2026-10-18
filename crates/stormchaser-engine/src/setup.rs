use crate::config::Config;
use sqlx::postgres::PgPoolOptions;
use sqlx::ConnectOptions;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::LogBackend;
use stormchaser_opa::PolicyConfig;
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
    let mut db_options = stormchaser_model::db_tls::connect_options(
        &config.database_url,
        &stormchaser_model::db_tls::DbTls {
            setting: config.db_ssl,
            ca: config.tls_ca_cert_path.as_deref(),
            cert: &config.tls_cert_path,
            key: &config.tls_key_path,
        },
    )?;
    if !stormchaser_model::db_tls::is_verified(&db_options) {
        tracing::warn!(
            "The database connection is not verified TLS, because STORMCHASER_DB_SSL=false \
             or DATABASE_URL asks for a weaker sslmode. Only do this where the database \
             is reached over a network nothing else can use"
        );
    }

    db_options = db_options
        .log_statements(log::LevelFilter::Debug)
        .log_slow_statements(log::LevelFilter::Warn, Duration::from_secs(1));

    let pool = PgPoolOptions::new()
        .max_connections(stormchaser_model::db_tls::max_connections()?)
        .connect_with(db_options)
        .await?;

    tracing::info!("Running database migrations...");
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("Database migrations completed successfully");

    Ok(pool)
}

pub fn setup_opa(config: &Config, tls_reloader: &TlsReloader) -> anyhow::Result<Arc<OpaClient>> {
    Ok(Arc::new(stormchaser_opa::policy_client(
        &PolicyConfig {
            url: config.opa_url.clone(),
            wasm_path: config.opa_wasm_path.clone(),
            data_path: config.opa_data_path.clone(),
            entrypoint: config.opa_entrypoint.clone(),
            disabled: config.authz_disabled,
        },
        Some(tls_reloader.client_config()),
    )?))
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
