//! The HTTP listener a service faces its clients with.
//!
//! It serves HTTPS unless the deployment switches TLS off (`STORMCHASER_LISTEN_TLS=false`),
//! using the service's own certificate and key, so tokens and secrets are not in clear
//! on the network between a proxy and the pod, or between services in the cluster.
//! Plain HTTP is for deployments where an ingress or a service mesh terminates TLS in
//! front of the service and the network behind it is trusted.

use crate::{TlsConfig, TlsReloader};
use anyhow::Context;
use axum::extract::ConnectInfo;
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// The setting that switches the listener's TLS off.
pub const LISTEN_TLS_VAR: &str = "STORMCHASER_LISTEN_TLS";

/// How long a client has to finish the TLS handshake. Without a limit, connections
/// that never send anything would be held open for free.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long requests in flight get to finish when the service is shutting down.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Reads `STORMCHASER_LISTEN_TLS`: on when unset.
pub fn listen_tls_setting(value: Option<&str>) -> anyhow::Result<bool> {
    value.map_or(Ok(true), |value| crate::parse_switch(LISTEN_TLS_VAR, value))
}

/// Loads the listener's certificate unless the deployment switched TLS off.
///
/// A service told to serve TLS that cannot read its certificate refuses to start,
/// rather than falling back to plain HTTP. The returned reloader must be kept alive
/// for as long as the listener runs: it is what follows the certificate files when
/// they are renewed.
pub async fn tls_for_listener(
    listen_tls: bool,
    config: TlsConfig,
) -> anyhow::Result<Option<Arc<TlsReloader>>> {
    if !listen_tls {
        tracing::warn!(
            "Listening without TLS because {LISTEN_TLS_VAR}=false. Only do this where \
             something in front of this service terminates TLS and the network between \
             them is trusted"
        );
        return Ok(None);
    }
    let (cert, key) = (config.cert_path.clone(), config.key_path.clone());
    let reloader = TlsReloader::new(config).await.with_context(|| {
        format!(
            "This service serves TLS unless {LISTEN_TLS_VAR}=false, and could not load its \
             certificate ({}) and key ({}); set TLS_CERT_PATH and TLS_KEY_PATH",
            cert.display(),
            key.display()
        )
    })?;
    Ok(Some(Arc::new(reloader)))
}

/// Serves `app` on `listener` until `shutdown` resolves, over TLS if `tls` is given.
///
/// Handlers see the client's address as `ConnectInfo<SocketAddr>` either way.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    tls: Option<Arc<TlsReloader>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let Some(tls) = tls else {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown)
        .await?;
        return Ok(());
    };

    let acceptor = TlsAcceptor::from(tls.listener_config());
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);

    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(e) => {
                    // Out of file descriptors, most likely; do not spin on it
                    tracing::warn!("Failed to accept a connection: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
            _ = &mut shutdown => break,
        };

        let acceptor = acceptor.clone();
        let app = app.clone();
        let watcher = graceful.watcher();
        // The handshake runs in its own task, so a slow client holds up no one else
        tokio::spawn(async move {
            let stream =
                match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                    Ok(Ok(stream)) => stream,
                    Ok(Err(e)) => {
                        tracing::debug!("TLS handshake with {peer} failed: {e}");
                        return;
                    }
                    Err(_) => {
                        tracing::debug!("TLS handshake with {peer} timed out");
                        return;
                    }
                };

            let service = hyper::service::service_fn(move |mut request: hyper::Request<_>| {
                request.extensions_mut().insert(ConnectInfo(peer));
                app.clone()
                    .oneshot(request.map(axum::body::Body::new::<hyper::body::Incoming>))
            });
            let builder = Builder::new(TokioExecutor::new());
            let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
            if let Err(e) = watcher.watch(connection.into_owned()).await {
                tracing::debug!("Connection from {peer} ended with an error: {e}");
            }
        });
    }

    drop(tls);
    if tokio::time::timeout(DRAIN_TIMEOUT, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("Shut down with requests still in flight");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_is_on_unless_switched_off() {
        assert!(listen_tls_setting(None).unwrap());
        assert!(listen_tls_setting(Some("true")).unwrap());
        assert!(!listen_tls_setting(Some("false")).unwrap());
        assert!(!listen_tls_setting(Some("FALSE")).unwrap());
    }

    #[test]
    fn a_setting_that_is_neither_on_nor_off_is_refused() {
        for value in ["", "flase", "0", "off"] {
            assert!(listen_tls_setting(Some(value)).is_err(), "{value:?}");
        }
    }

    #[tokio::test]
    async fn a_listener_without_its_certificate_refuses_to_start() {
        let Err(err) = tls_for_listener(
            true,
            TlsConfig {
                ca_cert_path: None,
                cert_path: "/nonexistent/tls.crt".into(),
                key_path: "/nonexistent/tls.key".into(),
                server_name: None,
            },
        )
        .await
        else {
            panic!("started without a certificate");
        };
        let message = format!("{err:#}");
        assert!(message.contains(LISTEN_TLS_VAR), "{message}");
        assert!(message.contains("/nonexistent/tls.crt"), "{message}");
    }
}
