//! The HTTP listener serving TLS itself, exercised over real sockets.

#![expect(
    clippy::disallowed_methods,
    reason = "tests call the servers they start"
)]

use axum::extract::ConnectInfo;
use axum::routing::get;
use axum::Router;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_api::listener::serve;
use stormchaser_tls::{TlsConfig, TlsReloader};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn certs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/certs")
}

fn app() -> Router {
    Router::new().route(
        "/whoami",
        get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
    )
}

/// Starts the listener with TLS on a free port. Dropping the sender shuts it down.
async fn start_tls() -> (
    SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let reloader = TlsReloader::new(TlsConfig {
        ca_cert_path: Some(certs().join("ca.crt")),
        cert_path: certs().join("tls.crt"),
        key_path: certs().join("tls.key"),
        server_name: None,
    })
    .await
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, app(), Some(Arc::new(reloader)), async {
        let _ = stopped.await;
    }));
    (addr, stop, server)
}

/// A client that trusts the test CA and nothing else, and has no certificate of its own.
fn client() -> reqwest::Client {
    let ca = std::fs::read(certs().join("ca.crt")).unwrap();
    reqwest::Client::builder()
        .tls_certs_only(reqwest::Certificate::from_pem_bundle(&ca).unwrap())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

#[tokio::test]
async fn requests_are_served_over_tls_to_a_client_without_a_certificate() {
    let (addr, _stop, _server) = start_tls().await;
    let url = format!("https://localhost:{}/whoami", addr.port());

    let response = client().get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.version(), reqwest::Version::HTTP_2);
    // The handler is told who is calling, as it is without TLS
    assert_eq!(response.text().await.unwrap(), "127.0.0.1");

    // A client that does not trust the CA refuses the certificate
    let stranger = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    assert!(stranger.get(&url).send().await.is_err());
}

#[tokio::test]
async fn nothing_is_served_in_clear_on_a_tls_listener() {
    let (addr, _stop, _server) = start_tls().await;

    let mut plain = TcpStream::connect(addr).await.unwrap();
    plain
        .write_all(b"GET /whoami HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .await
        .unwrap();
    let mut answer = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(15), plain.read_to_end(&mut answer)).await;
    let answer = String::from_utf8_lossy(&answer);
    assert!(
        !answer.contains("127.0.0.1") && !answer.contains("200 OK"),
        "a plain HTTP request was answered: {answer}"
    );
}

#[tokio::test]
async fn a_client_that_stalls_in_the_handshake_holds_up_no_one() {
    let (addr, _stop, _server) = start_tls().await;

    // Connections that say nothing
    let mut silent = Vec::new();
    for _ in 0..20 {
        silent.push(TcpStream::connect(addr).await.unwrap());
    }

    let url = format!("https://localhost:{}/whoami", addr.port());
    let started = std::time::Instant::now();
    let response = client().get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(silent);
}

#[tokio::test]
async fn the_listener_stops_when_asked() {
    let (addr, stop, server) = start_tls().await;
    let url = format!("https://localhost:{}/whoami", addr.port());
    assert_eq!(client().get(&url).send().await.unwrap().status(), 200);

    drop(stop);
    tokio::time::timeout(Duration::from_secs(30), server)
        .await
        .expect("the listener did not stop")
        .unwrap()
        .unwrap();
    assert!(client().get(&url).send().await.is_err());
}

#[tokio::test]
async fn without_tls_the_listener_serves_plain_http_as_before() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, app(), None, std::future::pending()));

    let response = reqwest::get(format!("http://{addr}/whoami")).await.unwrap();
    assert_eq!(response.text().await.unwrap(), "127.0.0.1");
}
