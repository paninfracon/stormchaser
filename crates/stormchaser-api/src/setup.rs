use crate::config::Config;
use crate::AppState;
use std::sync::Arc;
use stormchaser_core::setup;
use tokio::sync;

/// Builds the `AppState` from the provided `Config`.
pub async fn build_app_state(config: Config) -> anyhow::Result<AppState> {
    let tls_reloader = setup::tls_reloader(&config).await?;
    let pool = setup::connect_database(&config).await?;
    let nats = setup::connect_nats(&config, &tls_reloader).await?;
    let opa = setup::policy(&config, &tls_reloader)?;
    let (oidc_config, jwks) = setup::identity_provider(&config).await;

    Ok(AppState {
        pool,
        nats,
        opa,
        oidc_config,
        jwks: Arc::new(sync::RwLock::new(jwks)),
        log_backend: setup::log_backend(&config),
        api_base_url: config.api_base_url.clone(),
        mcp_allowed_hosts: config.mcp_allowed_hosts.clone(),
    })
}
