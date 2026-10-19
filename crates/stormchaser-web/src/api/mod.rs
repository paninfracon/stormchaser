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

/// A client of the API carrying the session's token. The API checks the token on each
/// call; [`require_auth`] only finds the cookie.
#[cfg(feature = "ssr")]
pub(crate) async fn authed_client() -> Result<stormchaser_client::Client, ServerFnError> {
    Ok(api_client().with_token(require_auth().await?))
}

/// A client of the API carrying the session's token, once the API has confirmed the
/// token is valid.
///
/// Work the web tier does itself, such as parsing a workflow, must not be done for
/// whoever sends a cookie with any value in it.
#[cfg(feature = "ssr")]
pub(crate) async fn require_session() -> Result<stormchaser_client::Client, ServerFnError> {
    let client = authed_client().await?;
    match client.session().await {
        Ok(_) => Ok(client),
        Err(_) => Err(ServerFnError::new("Unauthorized: No active session")),
    }
}

/// Turns a failed call into the error a server function returns: `what` and the status
/// the API answered with, or why it could not be reached.
#[cfg(feature = "ssr")]
pub(crate) fn api_error(what: &'static str) -> impl Fn(stormchaser_client::Error) -> ServerFnError {
    move |e| match e.status() {
        Some(status) => ServerFnError::new(format!("{what}: {status}")),
        None => ServerFnError::new(format!("{what}: {e}")),
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

/// Reads an object id passed to a server function. Ids are UUIDs; anything else is
/// refused rather than written into an API path, where `../` or `?` would send the
/// user's token to a different endpoint than the one the function calls.
#[cfg(feature = "ssr")]
pub(crate) fn parse_id(id: &str) -> Result<uuid::Uuid, leptos::prelude::ServerFnError> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| leptos::prelude::ServerFnError::new(format!("Not a valid id: {id:?}")))
}

#[cfg(feature = "ssr")]
fn ca_pem() -> Option<Vec<u8>> {
    let path = std::env::var("TLS_CA_CERT_PATH")
        .ok()
        .filter(|path| !path.is_empty())?;
    let pem =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read TLS_CA_CERT_PATH {path}: {e}"));
    Some(pem)
}

/// The client the server functions call the API (`API_URL`) and the query service
/// (`QUERY_URL`) with, without a token. It trusts `TLS_CA_CERT_PATH` as
/// [`build_http_client`] does.
#[cfg(feature = "ssr")]
pub fn api_client() -> stormchaser_client::Client {
    static CLIENT: std::sync::OnceLock<stormchaser_client::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let api_url =
                std::env::var("API_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
            let query_url =
                std::env::var("QUERY_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_string());
            let mut builder = stormchaser_client::Client::builder(api_url).query_url(query_url);
            if let Some(pem) = ca_pem() {
                builder = builder.ca_pem(pem);
            }
            builder
                .build()
                .unwrap_or_else(|e| panic!("cannot build the API client: {e}"))
        })
        .clone()
}

/// Builds the client the SSE proxies call the API with.
///
/// The API may serve TLS with a certificate from the deployment's own CA
/// (`TLS_CA_CERT_PATH`). It is trusted in addition to the system's certificate
/// authorities, not instead of them. A CA file that is named but cannot be used stops
/// the service: carrying on without it would only fail later, on every request.
#[cfg(feature = "ssr")]
pub fn build_http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    if let Some(pem) = ca_pem() {
        let certs = reqwest::Certificate::from_pem_bundle(&pem)
            .unwrap_or_else(|e| panic!("TLS_CA_CERT_PATH is not a PEM bundle: {e}"));
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

    #[test]
    fn only_uuids_are_accepted_as_ids() {
        let id = "0b9a4f9e-7c1e-4c55-9f1a-2a1e3c4d5e6f";
        assert_eq!(parse_id(id).unwrap().to_string(), id);
        for hostile in [
            "",
            "../../admin",
            "x?all=true",
            "abc/def",
            "%2e%2e",
            "1 OR 1",
        ] {
            assert!(parse_id(hostile).is_err(), "{hostile}");
        }
    }
}
