//! The HTTP listener, with or without TLS.
//!
//! By default the listener speaks plain HTTP, for deployments where an ingress or a
//! service mesh terminates TLS in front of it. With `STORMCHASER_LISTEN_TLS=true` it
//! speaks HTTPS itself, using the service's own certificate and key (`TLS_CERT_PATH`,
//! `TLS_KEY_PATH`), so tokens and secrets are not in clear on the network between the
//! proxy and the pod, or between services in the cluster.

use crate::config::Config;
use axum::extract::ConnectInfo;
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_tls::{TlsConfig, TlsReloader};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// How long a client has to finish the TLS handshake. Without a limit, connections
/// that never send anything would be held open for free.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long requests in flight get to finish when the service is shutting down.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Loads the listener's certificate if the service is configured to serve TLS.
///
/// The returned reloader must be kept alive for as long as the listener runs: it is
/// what follows the certificate files when they are renewed.
pub async fn tls_from_config(config: &Config) -> anyhow::Result<Option<Arc<TlsReloader>>> {
    if !config.listen_tls {
        tracing::warn!(
            "Listening without TLS. Set STORMCHASER_LISTEN_TLS=true unless something in \
             front of this service terminates TLS and the network between them is trusted"
        );
        return Ok(None);
    }
    let reloader = TlsReloader::new(TlsConfig {
        ca_cert_path: config.tls_ca_cert_path.clone(),
        cert_path: config.tls_cert_path.clone(),
        key_path: config.tls_key_path.clone(),
        server_name: config.tls_server_name.clone(),
    })
    .await?;
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
