//! The query service's state: what its handlers, token checks and policy need.

use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_core::auth::jwks::{JwksCache, OidcConfig};
use stormchaser_core::config::Config;
use stormchaser_core::service::ServiceState;
use stormchaser_core::setup;
use stormchaser_model::auth::OpaAuthorizer;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct QueryState {
    /// The database the queries' connections are read from
    pub pool: PgPool,
    /// The policy the service enforces
    pub opa: Arc<dyn OpaAuthorizer>,
    /// The identity provider whose tokens are accepted
    pub oidc_config: Option<OidcConfig>,
    /// The identity provider's signing keys
    pub jwks: Arc<RwLock<JwksCache>>,
}

impl QueryState {
    /// Connects to the database and sets up policy and token checks as the API does.
    pub async fn from_config(config: &Config) -> anyhow::Result<Self> {
        let tls_reloader = setup::tls_reloader(config).await?;
        let (oidc_config, jwks) = setup::identity_provider(config).await;
        Ok(Self {
            pool: setup::connect_database(config).await?,
            opa: setup::policy(config, &tls_reloader)?,
            oidc_config,
            jwks: Arc::new(RwLock::new(jwks)),
        })
    }
}

impl ServiceState for QueryState {
    fn pool(&self) -> &PgPool {
        &self.pool
    }
    fn opa(&self) -> &Arc<dyn OpaAuthorizer> {
        &self.opa
    }
    fn oidc_config(&self) -> Option<&OidcConfig> {
        self.oidc_config.as_ref()
    }
    fn jwks(&self) -> &Arc<RwLock<JwksCache>> {
        &self.jwks
    }
}
