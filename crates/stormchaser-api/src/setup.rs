use crate::auth::jwks::{fetch_jwks, OidcConfig};
use crate::config::Config;
use crate::AppState;
use async_nats::connect_with_options;
use sqlx::postgres::PgPoolOptions;
use sqlx::ConnectOptions;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::LogBackend;
use stormchaser_opa::PolicyConfig;
use stormchaser_tls::{TlsConfig, TlsReloader};
use tokio::sync;

/// Builds the `AppState` from the provided `Config`.
pub async fn build_app_state(config: Config) -> anyhow::Result<AppState> {
    let tls_config = TlsConfig {
        ca_cert_path: config.tls_ca_cert_path.clone(),
        cert_path: config.tls_cert_path.clone(),
        key_path: config.tls_key_path.clone(),
        server_name: config.tls_server_name.clone(),
    };

    let tls_reloader = Arc::new(TlsReloader::new(tls_config).await?);

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
        .max_connections(5)
        .connect_with(db_options)
        .await?;

    let nats_options = async_nats::ConnectOptions::new()
        .retry_on_initial_connect()
        .tls_client_config((*tls_reloader.client_config()).clone());
    let nats_options = stormchaser_model::nats::apply_auth_from_env(nats_options).await?;

    let nats_client = connect_with_options(config.nats_url.clone(), nats_options).await?;

    let opa_client = Arc::new(stormchaser_opa::policy_client(
        &PolicyConfig {
            url: config.opa_url.clone(),
            wasm_path: config.opa_wasm_path.clone(),
            data_path: config.opa_data_path.clone(),
            entrypoint: config.opa_entrypoint.clone(),
            disabled: config.authz_disabled,
        },
        Some(tls_reloader.client_config()),
    )?);

    // Log Backend Configuration
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

    // OIDC Configuration
    let mut oidc_config = None;
    let mut jwks = HashMap::new();

    if let (Some(issuer), Some(client_id), Some(client_secret)) = (
        config.oidc_issuer.clone(),
        config.oidc_client_id.clone(),
        config.oidc_client_secret.clone(),
    ) {
        let external_issuer = config
            .oidc_external_issuer
            .clone()
            .unwrap_or_else(|| issuer.clone());
        let jwks_url = format!("{}/keys", issuer.trim_end_matches('/'));
        tracing::info!(
            "Configuring OIDC with issuer: {}, external: {}, and JWKS: {}",
            issuer,
            external_issuer,
            jwks_url
        );

        // Fetch JWKS on startup
        jwks = fetch_jwks(&jwks_url).await;

        // Always set oidc_config if issuer and client_id are provided
        // This allows the bypass to work even if the OIDC provider is temporarily down
        oidc_config = Some(OidcConfig {
            issuer,
            external_issuer,
            client_id,
            client_secret,
            jwks_url,
        });
    }

    Ok(AppState {
        pool,
        nats: nats_client,
        opa: opa_client,
        oidc_config,
        jwks: Arc::new(sync::RwLock::new(jwks)),
        log_backend,
        api_base_url: config.api_base_url.clone(),
        mcp_allowed_hosts: config.mcp_allowed_hosts.clone(),
    })
}
