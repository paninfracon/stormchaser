use sqlx::migrate;
use std::env;
use std::net::SocketAddr;
use stormchaser_api::{
    app,
    config::Config,
    setup::build_app_state,
    telemetry::{init_telemetry, shutdown_telemetry},
};
use tokio::net::TcpListener;
use tokio::signal;

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
    let state = build_app_state(config).await?;

    tracing::info!("Running database migrations...");
    migrate!("./migrations").run(&state.pool).await?;

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
    use stormchaser_api::config::Config;

    #[test]
    fn test_config_from_env_missing_database_url() {
        let env: Vec<(&str, &str)> = vec![];
        let config = Config::from_env(env);
        let err = config.unwrap_err();
        assert_eq!(err.to_string(), "DATABASE_URL must be set");
    }

    #[test]
    fn test_config_from_env_valid() {
        use std::path::PathBuf;
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
