//! How services connect to PostgreSQL: over TLS, with a pool of a configured size.

use anyhow::Result;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use std::path::Path;

/// The largest connection pool a service opens to the database when
/// `STORMCHASER_DB_MAX_CONNECTIONS` is not set
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;

/// How many connections a service may open to the database: `STORMCHASER_DB_MAX_CONNECTIONS`,
/// or [`DEFAULT_MAX_CONNECTIONS`].
pub fn max_connections() -> Result<u32> {
    match std::env::var("STORMCHASER_DB_MAX_CONNECTIONS") {
        Ok(value) => match value.trim().parse::<u32>() {
            Ok(n) if n > 0 => Ok(n),
            _ => anyhow::bail!(
                "STORMCHASER_DB_MAX_CONNECTIONS must be a whole number above 0, not '{}'",
                value
            ),
        },
        Err(_) => Ok(DEFAULT_MAX_CONNECTIONS),
    }
}

/// The TLS files a service was started with.
pub struct DbTls<'a> {
    /// `STORMCHASER_DB_SSL`, if the deployment set it.
    pub setting: Option<bool>,
    /// The CA that signed the database's certificate, if it is not a public one.
    pub ca: Option<&'a Path>,
    /// The service's own certificate, offered to the database as a client certificate.
    pub cert: &'a Path,
    /// The key of `cert`.
    pub key: &'a Path,
}

/// Builds the connection options for `url`.
///
/// The connection is verified TLS unless the deployment says otherwise: the database's
/// certificate must chain to the CA (or, with none configured, to a public authority)
/// and carry the host name in the URL. `prefer`, which encrypts if the server offers
/// it but verifies nothing, stops no one who can get between the service and its
/// database, so it is never chosen here.
///
/// - `STORMCHASER_DB_SSL=true`: verified, whatever the URL says.
/// - `STORMCHASER_DB_SSL=false`: the `sslmode` written in the URL, or no TLS.
/// - Unset: the `sslmode` written in the URL, or verified.
pub fn connect_options(url: &str, tls: &DbTls<'_>) -> Result<PgConnectOptions> {
    let mut options: PgConnectOptions = url.parse()?;
    let verify = match tls.setting {
        Some(setting) => setting,
        None => !url_names_ssl_mode(url),
    };
    if verify {
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
        // Only reached with STORMCHASER_DB_SSL=false
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

    fn tls(setting: Option<bool>, ca: Option<&'static str>) -> DbTls<'static> {
        DbTls {
            setting,
            ca: ca.map(Path::new),
            cert: Path::new("/nonexistent/tls.crt"),
            key: Path::new("/nonexistent/tls.key"),
        }
    }

    #[test]
    fn unset_means_a_verified_connection() {
        let options = connect_options(URL, &tls(None, None)).unwrap();
        assert!(is_verified(&options));
    }

    #[test]
    fn asking_for_tls_always_means_a_verified_connection() {
        // With a CA, and without one (where it used to fall back to `prefer`)
        for ca in [Some("/etc/ca.crt"), None] {
            let options = connect_options(URL, &tls(Some(true), ca)).unwrap();
            assert!(is_verified(&options), "not verified with ca {ca:?}");
        }
        // Even if the URL asks for less
        let weaker = format!("{URL}?sslmode=prefer");
        assert!(is_verified(
            &connect_options(&weaker, &tls(Some(true), None)).unwrap()
        ));
    }

    #[test]
    fn a_mode_written_in_the_url_is_a_deployment_choice() {
        for setting in [None, Some(false)] {
            let url = format!("{URL}?sslmode=verify-full");
            assert!(is_verified(
                &connect_options(&url, &tls(setting, None)).unwrap()
            ));

            let url = format!("{URL}?sslmode=disable");
            let options = connect_options(&url, &tls(setting, None)).unwrap();
            assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
        }
    }

    #[test]
    fn switching_tls_off_with_no_mode_in_the_url_means_no_tls() {
        let options = connect_options(URL, &tls(Some(false), None)).unwrap();
        assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
        assert!(!is_verified(&options));
        // A parameter that merely contains the word is not a mode
        let url = format!("{URL}?application_name=sslmode");
        let options = connect_options(&url, &tls(Some(false), None)).unwrap();
        assert!(matches!(options.get_ssl_mode(), PgSslMode::Disable));
        let options = connect_options(&url, &tls(None, None)).unwrap();
        assert!(is_verified(&options));
    }
}
