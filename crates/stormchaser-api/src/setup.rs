use crate::auth::jwks::{fetch_jwks, OidcConfig};
use crate::config::Config;
use crate::AppState;
use anyhow::Context;
use async_nats::connect_with_options;
use sqlx::postgres;
use sqlx::postgres::PgPoolOptions;
use sqlx::ConnectOptions;
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::auth::OpaClient;
use stormchaser_model::LogBackend;
use stormchaser_opa::OpaWasmInstance;
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

    let mut db_options: postgres::PgConnectOptions = config.database_url.parse()?;
    if config.db_ssl {
        if let Some(ca) = &config.tls_ca_cert_path {
            db_options = db_options
                .ssl_mode(postgres::PgSslMode::VerifyFull)
                .ssl_root_cert(ca.to_string_lossy().to_string());
        }
        db_options = db_options
            .ssl_client_cert(config.tls_cert_path.clone())
            .ssl_client_key(config.tls_key_path.clone());
    } else {
        db_options = db_options.ssl_mode(postgres::PgSslMode::Disable);
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

    let nats_client = connect_with_options(config.nats_url.clone(), nats_options).await?;

    let mut opa_client = OpaClient::new(config.opa_url.clone(), Some(tls_reloader.client_config()));

    if let Some(wasm_path) = config.opa_wasm_path.clone() {
        tracing::info!("Loading OPA WASM policy from {}", wasm_path);
        let wasm_bytes = fs::read(&wasm_path).context("Failed to read OPA WASM policy")?;
        let executor = OpaWasmInstance::new(&wasm_bytes)?;
        opa_client = opa_client.with_wasm_executor(Arc::new(executor));
    }

    if let Some(entrypoint) = config.opa_entrypoint.clone() {
        opa_client = opa_client.with_entrypoint(entrypoint);
    }

    let opa_client = Arc::new(opa_client);

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
    })
}
