//! What the shared HTTP code needs from a service's state.

use crate::auth::jwks::{JwksCache, OidcConfig};
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::auth::OpaAuthorizer;
use tokio::sync::RwLock;

/// The parts of an HTTP service's state that token checks, policy and access control
/// use. The API and the query service each keep their own state and implement this.
pub trait ServiceState: Clone + Send + Sync + 'static {
    /// The database pool.
    fn pool(&self) -> &PgPool;
    /// The policy the service enforces.
    fn opa(&self) -> &Arc<dyn OpaAuthorizer>;
    /// The identity provider whose tokens are accepted, if one is configured.
    fn oidc_config(&self) -> Option<&OidcConfig>;
    /// The identity provider's signing keys.
    fn jwks(&self) -> &Arc<RwLock<JwksCache>>;
}
