use anyhow::Context;
use async_nats::connect_with_options;
use sqlx::migrate;
use sqlx::postgres;
use sqlx::postgres::PgPoolOptions;
use sqlx::ConnectOptions;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_api::{
    app,
    telemetry::{init_telemetry, shutdown_telemetry},
    AppState,
};
use stormchaser_model::auth::OpaClient;
use stormchaser_model::LogBackend;
use tokio::net::TcpListener;
use tokio::signal;
use tokio::sync;

use stormchaser_api::auth::jwks::fetch_jwks;
use stormchaser_api::auth::jwks::OidcConfig;
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
    /// The nats url.
    pub nats_url: String,
    /// The opa url.
    pub opa_url: Option<String>,
    /// The opa wasm path.
    pub opa_wasm_path: Option<String>,
    /// The opa entrypoint.
    pub opa_entrypoint: Option<String>,
    /// The loki url.
    pub loki_url: Option<String>,
    /// The elasticsearch url.
    pub elasticsearch_url: Option<String>,
    /// The elasticsearch index.
    pub elasticsearch_index: Option<String>,
    /// The oidc issuer.
    pub oidc_issuer: Option<String>,
    /// The oidc external issuer.
    pub oidc_external_issuer: Option<String>,
    /// The oidc client id.
    pub oidc_client_id: Option<String>,
    /// The oidc client secret.
    pub oidc_client_secret: Option<String>,
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
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut opa_url = None;
        let mut opa_wasm_path = None;
        let mut opa_entrypoint = None;
        let mut loki_url = None;
        let mut elasticsearch_url = None;
        let mut elasticsearch_index = None;
        let mut oidc_issuer = None;
        let mut oidc_external_issuer = None;
        let mut oidc_client_id = None;
        let mut oidc_client_secret = None;

        for (k, v) in env {
            match k.as_ref() {
                "DATABASE_URL" => database_url = Some(v.as_ref().to_string()),
                "TLS_CA_CERT_PATH" => tls_ca_cert_path = Some(PathBuf::from(v.as_ref())),
                "TLS_CERT_PATH" => tls_cert_path = PathBuf::from(v.as_ref()),
                "TLS_KEY_PATH" => tls_key_path = PathBuf::from(v.as_ref()),
                "TLS_SERVER_NAME" => tls_server_name = Some(v.as_ref().to_string()),
                "STORMCHASER_DB_SSL" => db_ssl = v.as_ref() == "true",
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "OPA_URL" => opa_url = Some(v.as_ref().to_string()),
                "OPA_WASM_PATH" => opa_wasm_path = Some(v.as_ref().to_string()),
                "OPA_ENTRYPOINT" => opa_entrypoint = Some(v.as_ref().to_string()),
                "LOKI_URL" => loki_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_URL" => elasticsearch_url = Some(v.as_ref().to_string()),
                "ELASTICSEARCH_INDEX" => elasticsearch_index = Some(v.as_ref().to_string()),
                "OIDC_ISSUER" => oidc_issuer = Some(v.as_ref().to_string()),
                "OIDC_EXTERNAL_ISSUER" => oidc_external_issuer = Some(v.as_ref().to_string()),
                "OIDC_CLIENT_ID" => oidc_client_id = Some(v.as_ref().to_string()),
                "OIDC_CLIENT_SECRET" => oidc_client_secret = Some(v.as_ref().to_string()),
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
            nats_url,
            opa_url,
            opa_wasm_path,
            opa_entrypoint,
            loki_url,
            elasticsearch_url,
            elasticsearch_index,
            oidc_issuer,
            oidc_external_issuer,
            oidc_client_id,
            oidc_client_secret,
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    init_telemetry()?;

    tracing::info!(
        "Stormchaser API {} starting (rev: {}, branch: {}, built: {})",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_BUILD_TIMESTAMP")
    );

    let config = Config::from_env(env::vars())?;
    run_server(config).await
}

/// Run server.
pub async fn run_server(config: Config) -> anyhow::Result<()> {
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
            .ssl_client_cert(config.tls_cert_path)
            .ssl_client_key(config.tls_key_path);
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

    tracing::info!("Running database migrations...");
    migrate!("./migrations").run(&pool).await?;

    let nats_options = async_nats::ConnectOptions::new()
        .retry_on_initial_connect()
        .tls_client_config((*tls_reloader.client_config()).clone());

    let nats_client = connect_with_options(config.nats_url, nats_options).await?;

    let mut opa_client = OpaClient::new(config.opa_url, Some(tls_reloader.client_config()));

    if let Some(wasm_path) = config.opa_wasm_path {
        tracing::info!("Loading OPA WASM policy from {}", wasm_path);
        let wasm_bytes = fs::read(&wasm_path).context("Failed to read OPA WASM policy")?;
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

    // OIDC Configuration
    let mut oidc_config = None;
    let mut jwks = HashMap::new();

    if let (Some(issuer), Some(client_id), Some(client_secret)) = (
        config.oidc_issuer,
        config.oidc_client_id,
        config.oidc_client_secret,
    ) {
        let external_issuer = config
            .oidc_external_issuer
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

    let state = AppState {
        pool,
        nats: nats_client,
        opa: opa_client,
        oidc_config,
        jwks: Arc::new(sync::RwLock::new(jwks)),
        log_backend,
    };

    let app = app(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("listening on {}", addr);
    let listener = TcpListener::bind(addr).await?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    shutdown_telemetry();
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutting down...");
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
            ("TLS_SERVER_NAME", "example.com"),
            ("STORMCHASER_DB_SSL", "true"),
            ("LOKI_URL", "http://loki:3100"),
        ];
        let config = Config::from_env(env).unwrap();
        assert_eq!(config.database_url, "postgres://user:pass@localhost/db");
        assert_eq!(config.tls_server_name.as_deref(), Some("example.com"));
        assert!(config.db_ssl);
        assert_eq!(config.nats_url, "nats://localhost:4222");
        assert_eq!(config.loki_url.as_deref(), Some("http://loki:3100"));
        assert_eq!(
            config.tls_cert_path,
            PathBuf::from("/etc/engine/certs/tls.crt")
        );
    }
}
