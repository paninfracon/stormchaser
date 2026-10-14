//! How services connect to PostgreSQL over TLS.

use anyhow::Result;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use std::path::Path;

/// The TLS files a service was started with.
pub struct DbTls<'a> {
    /// `STORMCHASER_DB_SSL=true`: the operator asked for TLS to the database.
    pub requested: bool,
    /// The CA that signed the database's certificate, if it is not a public one.
    pub ca: Option<&'a Path>,
    /// The service's own certificate, offered to the database as a client certificate.
    pub cert: &'a Path,
    /// The key of `cert`.
    pub key: &'a Path,
}

/// Builds the connection options for `url`.
///
/// Asking for TLS means a verified connection: the database's certificate must chain
/// to the CA and carry the host name in the URL. It used to mean that only when a CA
/// was configured; without one the connection fell back to `prefer`, which encrypts if
/// the server offers it, verifies nothing, and so stops no one who can get between the
/// service and its database.
///
/// Without `requested`, an `sslmode` written in the URL is respected (it was
/// overridden with `disable` before, which silently downgraded a URL that asked for
/// verification), and a URL that names none connects without TLS as it always has.
pub fn connect_options(url: &str, tls: &DbTls<'_>) -> Result<PgConnectOptions> {
    let mut options: PgConnectOptions = url.parse()?;
    if tls.requested {
        options = options.ssl_mode(PgSslMode::VerifyFull);
        if let Some(ca) = tls.ca {
            options = options.ssl_root_cert(ca);
        }
        // A client certificate is offered when the service has one. A managed
        // database that authenticates by password alone needs none.
        if tls.cert.is_file() && tls.key.is_file() {
            options = options.ssl_client_cert(tls.cert).ssl_client_key(tls.key);
        }
    } else if !url_names_ssl_mode(url) {
        options = options.ssl_mode(PgSslMode::Disable);
    }
    Ok(options)
}

/// Whether a connection made with these options proves it reached the right database.
pub fn is_verified(options: &PgConnectOptions) -> bool {
    matches!(options.get_ssl_mode(), PgSslMode::VerifyFull)
}

fn url_names_ssl_mode(url: &str) -> bool {
    url.split_once('?').is_some_and(|(_, query)| {
        query.split('&').any(|pair| {
            let key = pair.split('=').next().unwrap_or_default();
            key.eq_ignore_ascii_case("sslmode") || key.eq_ignore_ascii_case("ssl-mode")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "postgres://user:pw@db.internal/stormchaser";

    fn tls(requested: bool, ca: Option<&'static str>) -> DbTls<'static> {
        DbTls {
            requested,
            ca: ca.map(Path::new),
            cert: Path::new("/nonexistent/tls.crt"),
            key: Path::new("/nonexistent/tls.key"),
        }
    }

    #[test]
    fn asking_for_tls_always_means_a_verified_connection() {
        // With a CA, and without one (where it used to fall back to `prefer`)
        for ca in [Some("/etc/ca.crt"), None] {
            let options = connect_options(URL, &tls(true, ca)).unwrap();
            assert!(is_verified(&options), "not verified with ca {ca:?}");
        }
        // Even if the URL asks for less
        let weaker = format!("{URL}?sslmode=prefer");
        assert!(is_verified(
            &connect_options(&weaker, &tls(true, None)).unwrap()
        ));
    }

    #[test]
    fn a_url_that_asks_for_verification_is_not_downgraded() {
        let url = format!("{URL}?sslmode=verify-full");
        let options = connect_options(&url, &tls(false, None)).unwrap();
        assert!(is_verified(&options));
    }

    #[test]
    fn a_url_that_names_no_mode_connects_without_tls_as_before() {
        let options = connect_options(URL, &tls(false, None)).unwrap();
        assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
        assert!(!is_verified(&options));
        // A parameter that merely contains the word is not a mode
        let url = format!("{URL}?application_name=sslmode");
        let options = connect_options(&url, &tls(false, None)).unwrap();
        assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
    }
}
