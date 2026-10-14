//! The API's and the query service's HTTP listener: HTTPS unless the deployment
//! switches it off. See [`stormchaser_tls::listener`].

use crate::config::Config;
use std::sync::Arc;
use stormchaser_tls::{TlsConfig, TlsReloader};

pub use stormchaser_tls::listener::serve;

/// Loads the listener's certificate unless `STORMCHASER_LISTEN_TLS=false`.
///
/// The returned reloader must be kept alive for as long as the listener runs.
pub async fn tls_from_config(config: &Config) -> anyhow::Result<Option<Arc<TlsReloader>>> {
    stormchaser_tls::listener::tls_for_listener(
        config.listen_tls,
        TlsConfig {
            ca_cert_path: config.tls_ca_cert_path.clone(),
            cert_path: config.tls_cert_path.clone(),
            key_path: config.tls_key_path.clone(),
            server_name: config.tls_server_name.clone(),
        },
    )
    .await
}
