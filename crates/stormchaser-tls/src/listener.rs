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
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::{GracefulShutdown, Watcher};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;

/// The setting that switches the listener's TLS off.
pub const LISTEN_TLS_VAR: &str = "STORMCHASER_LISTEN_TLS";

/// How long a client has to finish the TLS handshake. Without a limit, connections
/// that never send anything would be held open for free.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a client has to send a request's headers, on HTTP/1.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);

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
    serve_with(listener, app, tls, shutdown, HEADER_READ_TIMEOUT).await
}

async fn serve_with(
    listener: TcpListener,
    app: Router,
    tls: Option<Arc<TlsReloader>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
    header_read_timeout: Duration,
) -> anyhow::Result<()> {
    let acceptor = tls
        .as_ref()
        .map(|tls| TlsAcceptor::from(tls.listener_config()));
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
        // Each connection, and its handshake, runs in its own task, so a slow client
        // holds up no one else
        tokio::spawn(async move {
            let Some(acceptor) = acceptor else {
                serve_connection(stream, peer, app, watcher, header_read_timeout).await;
                return;
            };
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
            serve_connection(stream, peer, app, watcher, header_read_timeout).await;
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

/// Serves HTTP/1 or HTTP/2 on one accepted connection until it closes.
async fn serve_connection<S>(
    stream: S,
    peer: SocketAddr,
    app: Router,
    watcher: Watcher,
    header_read_timeout: Duration,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |mut request: hyper::Request<_>| {
        request.extensions_mut().insert(ConnectInfo(peer));
        app.clone()
            .oneshot(request.map(axum::body::Body::new::<hyper::body::Incoming>))
    });
    let mut builder = Builder::new(TokioExecutor::new());
    // hyper applies its header read timeout only when it has a timer; without one a
    // client could hold the connection by sending its headers a byte at a time
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(header_read_timeout);
    builder.http2().timer(TokioTimer::new());
    let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
    if let Err(e) = watcher.watch(connection.into_owned()).await {
        tracing::debug!("Connection from {peer} ended with an error: {e}");
    }
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
    async fn a_client_that_never_finishes_its_headers_is_disconnected() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/", axum::routing::get(|| async { "ok" }));
        tokio::spawn(serve_with(
            listener,
            app,
            None,
            std::future::pending(),
            Duration::from_millis(200),
        ));

        // A complete request is still served
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client
            .write_all(b"GET / HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");

        let mut slow = tokio::net::TcpStream::connect(addr).await.unwrap();
        slow.write_all(b"GET / HTTP/1.1\r\nhost: x\r\n")
            .await
            .unwrap();
        let mut rest = Vec::new();
        let closed =
            tokio::time::timeout(Duration::from_secs(5), slow.read_to_end(&mut rest)).await;
        assert!(closed.is_ok(), "the connection was held open");
        assert!(!String::from_utf8_lossy(&rest).contains("200"));
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
