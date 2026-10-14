pub mod runs;
pub use runs::*;
pub mod connections;
pub use connections::*;
pub mod webhooks;
pub use webhooks::*;
pub mod rules;
pub use rules::*;
pub mod cron;
pub use cron::*;
pub mod dsl;
pub use dsl::*;
pub mod schema;
pub use schema::*;
pub mod storage;
pub use storage::*;

use leptos::prelude::*;
use leptos::server_fn::codec::Json;
#[cfg(feature = "ssr")]
use url::{Host, Url};

#[cfg(feature = "ssr")]
async fn get_cookie_header() -> Result<Option<String>, ServerFnError> {
    use axum::http::{header::COOKIE, HeaderMap};
    use leptos_axum::extract;

    let headers = extract::<HeaderMap>().await?;
    if let Some(cookie) = headers.get(COOKIE) {
        let s = cookie
            .to_str()
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        Ok(Some(s.to_string()))
    } else {
        Ok(None)
    }
}

#[cfg(feature = "ssr")]
pub(crate) async fn require_auth() -> Result<String, ServerFnError> {
    match get_cookie_header().await? {
        Some(c) => {
            for part in c.split(';') {
                let part = part.trim();
                if let Some(token) = part.strip_prefix("auth_token=") {
                    return Ok(token.to_string());
                }
            }
            Err(ServerFnError::new("Unauthorized: No active session"))
        }
        None => Err(ServerFnError::new("Unauthorized: No active session")),
    }
}

#[server(input = Json, output = Json)]
pub async fn check_auth() -> Result<bool, ServerFnError> {
    let _ = require_auth().await?;
    Ok(true)
}

#[server(input = Json, output = Json)]
pub async fn get_grafana_url() -> Result<Option<String>, ServerFnError> {
    Ok(std::env::var("GRAFANA_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && is_valid_grafana_url(s)))
}

/// The origin of the configured Grafana, which pages embed in a frame, if there is one.
#[cfg(feature = "ssr")]
pub fn grafana_origin() -> Option<String> {
    let url = std::env::var("GRAFANA_URL").ok()?;
    let url = url.trim();
    if url.is_empty() || !is_valid_grafana_url(url) {
        return None;
    }
    Some(Url::parse(url).ok()?.origin().ascii_serialization())
}

#[cfg(feature = "ssr")]
fn is_valid_grafana_url(url: &str) -> bool {
    Url::parse(url).ok().is_some_and(|parsed| {
        matches!(parsed.scheme(), "http" | "https")
            && match parsed.host() {
                Some(Host::Domain(domain)) => {
                    !domain.is_empty()
                        && domain
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
                }
                Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => true,
                None => false,
            }
    })
}

#[cfg(feature = "ssr")]
pub(crate) fn http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(build_http_client).clone()
}

/// Builds the client the web tier calls the API and the query service with.
///
/// Those services may serve TLS with a certificate from the deployment's own CA
/// (`TLS_CA_CERT_PATH`). It is trusted in addition to the system's certificate
/// authorities, not instead of them. A CA file that is named but cannot be used stops
/// the service: carrying on without it would only fail later, on every request.
#[cfg(feature = "ssr")]
pub fn build_http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    if let Some(path) = std::env::var("TLS_CA_CERT_PATH")
        .ok()
        .filter(|path| !path.is_empty())
    {
        let pem = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("cannot read TLS_CA_CERT_PATH {path}: {e}"));
        let certs = reqwest::Certificate::from_pem_bundle(&pem)
            .unwrap_or_else(|e| panic!("TLS_CA_CERT_PATH {path} is not a PEM bundle: {e}"));
        builder = builder.tls_certs_merge(certs);
    }
    builder.build().expect("failed to build the HTTP client")
}

#[cfg(test)]
#[cfg(feature = "ssr")]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvVarGuard {
        fn new(key: &'static str) -> Self {
            Self {
                key,
                original: std::env::var(key).ok(),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(original) = self.original.clone() {
                std::env::set_var(self.key, original);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[tokio::test]
    async fn test_get_grafana_url() {
        static ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        let _lock = ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let _env_guard = EnvVarGuard::new("GRAFANA_URL");

        std::env::set_var("GRAFANA_URL", "http://grafana");
        let result = get_grafana_url().await.unwrap();
        assert_eq!(result.unwrap(), "http://grafana");

        std::env::set_var("GRAFANA_URL", "ftp://grafana");
        let result = get_grafana_url().await.unwrap();
        assert!(result.is_none());

        std::env::set_var("GRAFANA_URL", "http://");
        let result = get_grafana_url().await.unwrap();
        assert!(result.is_none());

        std::env::set_var("GRAFANA_URL", "https://;malicious");
        let result = get_grafana_url().await.unwrap();
        assert!(result.is_none());

        std::env::set_var("GRAFANA_URL", "");
        let result = get_grafana_url().await.unwrap();
        assert!(result.is_none());

        std::env::remove_var("GRAFANA_URL");
        let result = get_grafana_url().await.unwrap();
        assert!(result.is_none());
    }
}
