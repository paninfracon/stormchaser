//! Starting an HTTP service: its TLS, database, NATS, policy, log backend and identity
//! provider, each from the service's [`Config`].

use crate::auth::jwks::{fetch_jwks, JwksCache, OidcConfig};
use crate::config::Config;
use sqlx::postgres::PgPoolOptions;
use sqlx::{ConnectOptions, PgPool};
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::auth::OpaAuthorizer;
use stormchaser_model::LogBackend;
use stormchaser_opa::PolicyConfig;
use stormchaser_tls::{TlsConfig, TlsReloader};

/// The client certificate and trusted CAs the service uses for its outbound connections.
pub async fn tls_reloader(config: &Config) -> anyhow::Result<Arc<TlsReloader>> {
    let tls_config = TlsConfig {
        ca_cert_path: config.tls_ca_cert_path.clone(),
        cert_path: config.tls_cert_path.clone(),
        key_path: config.tls_key_path.clone(),
        server_name: config.tls_server_name.clone(),
    };
    Ok(Arc::new(TlsReloader::new(tls_config).await?))
}

/// The database pool, connected with TLS as `STORMCHASER_DB_SSL` and `DATABASE_URL` say.
pub async fn connect_database(config: &Config) -> anyhow::Result<PgPool> {
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

    Ok(PgPoolOptions::new()
        .max_connections(stormchaser_model::db_tls::max_connections()?)
        .connect_with(db_options)
        .await?)
}

/// The NATS client, authenticated from the environment.
pub async fn connect_nats(
    config: &Config,
    tls_reloader: &TlsReloader,
) -> anyhow::Result<async_nats::Client> {
    let nats_options = async_nats::ConnectOptions::new()
        .retry_on_initial_connect()
        .tls_client_config((*tls_reloader.client_config()).clone());
    let nats_options = stormchaser_model::nats::apply_auth_from_env(nats_options).await?;
    Ok(async_nats::connect_with_options(config.nats_url.clone(), nats_options).await?)
}

/// The policy the service enforces: OPA at a URL, a WebAssembly policy, or the shipped one.
pub fn policy(
    config: &Config,
    tls_reloader: &TlsReloader,
) -> anyhow::Result<Arc<dyn OpaAuthorizer>> {
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

/// Where step logs are read from: Loki if `LOKI_URL` is set, else Elasticsearch.
pub fn log_backend(config: &Config) -> Option<LogBackend> {
    if let Some(url) = config.loki_url.clone() {
        tracing::info!("Configuring Loki log backend: {}", url);
        Some(LogBackend::Loki { url })
    } else if let (Some(url), Some(index)) = (
        config.elasticsearch_url.clone(),
        config.elasticsearch_index.clone(),
    ) {
        tracing::info!(
            "Configuring Elasticsearch log backend: {} (index: {})",
            url,
            index
        );
        Some(LogBackend::Elasticsearch { url, index })
    } else {
        None
    }
}

/// The identity provider whose tokens are accepted, and its signing keys as fetched now.
///
/// The provider is configured whenever its issuer, client id and secret are set, even if
/// its keys cannot be fetched yet: they are fetched again when a token names a key that
/// is not known.
pub async fn identity_provider(config: &Config) -> (Option<OidcConfig>, JwksCache) {
    let (Some(issuer), Some(client_id), Some(client_secret)) = (
        config.oidc_issuer.clone(),
        config.oidc_client_id.clone(),
        config.oidc_client_secret.clone(),
    ) else {
        return (None, JwksCache::new());
    };
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
    let jwks = fetch_jwks(&jwks_url).await;
    (
        Some(OidcConfig {
            issuer,
            external_issuer,
            client_id,
            client_secret,
            jwks_url,
        }),
        jwks,
    )
}
