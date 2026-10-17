use crate::ApiDoc;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp_openapi::Server;
use std::io::Error;
use tracing::error;
use url::Url;
use utoipa::OpenApi;

/// Hosts accepted regardless of configuration, so local clients keep working.
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// Builds the list of `Host` header values the MCP transport accepts.
///
/// The transport validates `Host` to guard against DNS rebinding. The list is the
/// loopback names, the host of the API's own base URL, and any operator-configured
/// extras; an entry without a port matches any port.
fn allowed_hosts(base_url: &Url, extra_hosts: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = LOOPBACK_HOSTS.iter().map(|h| h.to_string()).collect();
    let configured = base_url
        .host_str()
        .map(|host| host.trim_matches(['[', ']']).to_string())
        .into_iter()
        .chain(extra_hosts.iter().cloned());
    for host in configured {
        if !hosts.iter().any(|known| known.eq_ignore_ascii_case(&host)) {
            hosts.push(host);
        }
    }
    hosts
}

pub fn mcp_service(
    api_base_url: &str,
    extra_allowed_hosts: &[String],
) -> Option<StreamableHttpService<Server, LocalSessionManager>> {
    let openapi_val = match serde_json::to_value(ApiDoc::openapi()) {
        Ok(value) => value,
        Err(error) => {
            error!(?error, "Failed to serialize OpenAPI spec for MCP service");
            return None;
        }
    };
    let base_url = match Url::parse(api_base_url) {
        Ok(value) => value,
        Err(error) => {
            error!(?error, "Invalid API_BASE_URL; MCP service disabled");
            return None;
        }
    };

    let allowed_hosts = allowed_hosts(&base_url, extra_allowed_hosts);

    Some(StreamableHttpService::new(
        move || {
            let mut server = Server::new(
                openapi_val.clone(),
                base_url.clone(),
                None,
                None,
                false,
                false,
                false,
            );
            server.load_openapi_spec().map_err(|error| {
                error!(?error, "Failed to load MCP OpenAPI spec");
                Error::other(error.to_string())
            })?;
            Ok(server)
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_hosts_defaults_to_loopback() {
        let base_url = Url::parse("http://localhost:3000").unwrap();
        assert_eq!(
            allowed_hosts(&base_url, &[]),
            vec!["localhost", "127.0.0.1", "::1"]
        );
    }

    #[test]
    fn allowed_hosts_includes_base_url_host_and_extras() {
        let base_url = Url::parse("https://stormchaser.paninfracon.net/").unwrap();
        let extras = vec![
            "api.paninfracon.net:8443".to_string(),
            "LOCALHOST".to_string(),
        ];
        assert_eq!(
            allowed_hosts(&base_url, &extras),
            vec![
                "localhost",
                "127.0.0.1",
                "::1",
                "stormchaser.paninfracon.net",
                "api.paninfracon.net:8443"
            ]
        );
    }

    #[test]
    fn allowed_hosts_strips_ipv6_brackets_from_base_url() {
        let base_url = Url::parse("http://[2001:db8::1]:3000").unwrap();
        assert!(allowed_hosts(&base_url, &[]).contains(&"2001:db8::1".to_string()));
    }
}
