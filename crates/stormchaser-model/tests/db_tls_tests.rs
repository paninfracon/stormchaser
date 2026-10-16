//! Connects to a real PostgreSQL server that offers TLS and checks what gets through.
//!
//! Ignored by default. Start a server with the test certificate (it names `localhost`
//! and `127.0.0.1`) and point the test at it:
//!
//! ```text
//! docker run -d --name sc-tls-pg -p 127.0.0.1:5499:5432 -e POSTGRES_PASSWORD=pw \
//!   -v "$PWD/tests/certs:/certs:ro" postgres:16-alpine sh -c \
//!   'cp /certs/tls.crt /certs/tls.key /var/lib/postgresql/ &&
//!    chown postgres /var/lib/postgresql/tls.* && chmod 600 /var/lib/postgresql/tls.key &&
//!    exec docker-entrypoint.sh postgres -c ssl=on \
//!      -c ssl_cert_file=/var/lib/postgresql/tls.crt -c ssl_key_file=/var/lib/postgresql/tls.key'
//! STORMCHASER_TLS_DB_PORT=5499 STORMCHASER_TLS_DB_PASSWORD=pw \
//!   cargo test -p stormchaser-model --test db_tls_tests -- --ignored
//! ```

use sqlx::ConnectOptions;
use std::path::{Path, PathBuf};
use stormchaser_model::db_tls::{connect_options, is_verified, DbTls};

fn certs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/certs")
}

fn url(host: &str) -> String {
    format!(
        "postgres://postgres:{}@{}:{}/postgres",
        std::env::var("STORMCHASER_TLS_DB_PASSWORD").expect("STORMCHASER_TLS_DB_PASSWORD"),
        host,
        std::env::var("STORMCHASER_TLS_DB_PORT").expect("STORMCHASER_TLS_DB_PORT"),
    )
}

/// Whether a connection made the way a service would make it succeeds, and whether it
/// ended up encrypted.
async fn connect(url: &str, setting: Option<bool>, ca: Option<&Path>) -> Result<bool, sqlx::Error> {
    let options = connect_options(
        url,
        &DbTls {
            setting,
            ca,
            cert: Path::new("/nonexistent/tls.crt"),
            key: Path::new("/nonexistent/tls.key"),
        },
    )
    .unwrap();
    assert_eq!(is_verified(&options), setting != Some(false));
    let mut connection = options.connect().await?;
    let (encrypted,): (bool,) =
        sqlx::query_as("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()")
            .fetch_one(&mut connection)
            .await?;
    Ok(encrypted)
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server with TLS"]
async fn by_default_only_a_database_that_can_be_verified_is_accepted() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let ca = certs().join("ca.crt");

    // The right CA and a name the certificate carries: connected, and encrypted,
    // whether TLS is left at its default or asked for
    assert!(connect(&url("localhost"), None, Some(&ca)).await.unwrap());
    assert!(connect(&url("localhost"), Some(true), Some(&ca))
        .await
        .unwrap());

    // No CA configured: the certificate is from no public authority, so the
    // connection is refused. This used to fall back to an unverified one.
    assert!(connect(&url("localhost"), None, None).await.is_err());

    // The right CA but a name the certificate does not carry
    assert!(connect(&url("127.0.0.2"), None, Some(&ca)).await.is_err());

    // A CA that did not sign the certificate (the server's own, here)
    let wrong_ca = certs().join("tls.crt");
    assert!(connect(&url("localhost"), None, Some(&wrong_ca))
        .await
        .is_err());
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server with TLS"]
async fn a_deployment_can_choose_its_own_mode() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Switched off, with no mode in the URL: in clear
    assert!(!connect(&url("localhost"), Some(false), None).await.unwrap());

    // A URL that asks for verification is no longer downgraded to no TLS
    let strict = format!(
        "{}?sslmode=verify-full&sslrootcert={}",
        url("localhost"),
        certs().join("ca.crt").display()
    );
    let options = connect_options(
        &strict,
        &DbTls {
            setting: None,
            ca: None,
            cert: Path::new("/nonexistent/tls.crt"),
            key: Path::new("/nonexistent/tls.key"),
        },
    )
    .unwrap();
    assert!(is_verified(&options));
    let mut connection = options.connect().await.unwrap();
    let (encrypted,): (bool,) =
        sqlx::query_as("SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert!(encrypted);
}
