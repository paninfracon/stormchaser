//! Short-lived registry credentials for pushing a run's OCI artifacts.
//!
//! An OCI artifact backend stores a registry username and password. Those never leave the
//! engine: when a step is ready to push, the engine exchanges them at the registry's token
//! endpoint for a token that is only good for that run's repositories, for a few minutes,
//! and hands the runner that instead.

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use stormchaser_core::net_guard::{guarded_client, read_body_capped};
use stormchaser_model::connections::Connection;
use uuid::Uuid;

/// Token responses are a few kilobytes; anything much larger is not one.
const MAX_TOKEN_RESPONSE_BYTES: usize = 256 * 1024;

/// Where an OCI artifact backend's `registry` setting points.
#[derive(Debug, PartialEq, Eq)]
pub struct RegistryLocation {
    /// Host, with port if any, as it appears in image references and Docker configs.
    pub host: String,
    /// Path under the host that every repository of this backend starts with, if any.
    pub prefix: String,
    /// True when the registry is spoken to over plain HTTP.
    pub plain_http: bool,
}

impl RegistryLocation {
    /// Reads the location from a backend's configuration.
    pub fn of(backend: &Connection) -> Result<Self> {
        let registry = backend.config["registry"]
            .as_str()
            .context("Missing registry in OCI backend config")?;
        let plain_http = backend.config["plain_http"].as_bool().unwrap_or(false);
        Self::parse(registry, plain_http)
    }

    fn parse(registry: &str, plain_http: bool) -> Result<Self> {
        let (scheme, rest) = match registry.split_once("://") {
            Some((scheme, rest)) => (Some(scheme), rest),
            None => (None, registry),
        };
        let rest = rest.trim_matches('/');
        let (host, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        if host.is_empty() || host.contains('@') {
            bail!("OCI backend registry '{}' has no usable host", registry);
        }
        Ok(Self {
            host: host.to_ascii_lowercase(),
            prefix: prefix.to_string(),
            plain_http: plain_http || scheme == Some("http"),
        })
    }

    fn base_url(&self) -> String {
        let scheme = if self.plain_http { "http" } else { "https" };
        format!("{}://{}", scheme, self.host)
    }

    /// The repository a run's artifacts from one storage volume are pushed to.
    pub fn artifact_repository(&self, run_id: Uuid, storage_name: &str) -> String {
        if self.prefix.is_empty() {
            format!("{}/{}", run_id, storage_name)
        } else {
            format!("{}/{}/{}", self.prefix, run_id, storage_name)
        }
    }
}

/// The parameters of a `WWW-Authenticate: Bearer` challenge.
#[derive(Debug, PartialEq, Eq)]
struct BearerChallenge {
    realm: String,
    service: Option<String>,
}

/// Parses a `WWW-Authenticate` header, returning `None` unless it asks for a bearer token.
fn parse_bearer_challenge(header: &str) -> Option<BearerChallenge> {
    let (scheme, params) = header.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }

    let mut realm = None;
    let mut service = None;
    let mut rest = params.trim();
    while !rest.is_empty() {
        let (key, after_key) = rest.split_once('=')?;
        let key = key.trim().trim_start_matches(',').trim();
        let after_key = after_key.trim_start();
        let (value, remainder) = if let Some(quoted) = after_key.strip_prefix('"') {
            let end = quoted.find('"')?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = after_key.find(',').unwrap_or(after_key.len());
            (after_key[..end].trim(), &after_key[end..])
        };
        match key.to_ascii_lowercase().as_str() {
            "realm" => realm = Some(value.to_string()),
            "service" => service = Some(value.to_string()),
            _ => {}
        }
        rest = remainder.trim_start().trim_start_matches(',').trim_start();
    }

    Some(BearerChallenge {
        realm: realm?,
        service,
    })
}

/// Why a scoped token could not be had.
#[derive(Debug)]
pub enum TokenError {
    /// The registry does not hand out bearer tokens (basic auth only, or no auth at all).
    NotSupported,
    /// The registry does, but the exchange failed.
    Failed(anyhow::Error),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSupported => write!(f, "the registry does not issue scoped tokens"),
            Self::Failed(e) => write!(f, "{:#}", e),
        }
    }
}

/// Exchanges the backend's stored credentials for a token that can push to `repositories`
/// and nothing else.
pub async fn mint_push_token(
    location: &RegistryLocation,
    username: &str,
    password: &str,
    repositories: &[String],
) -> Result<String, TokenError> {
    let challenge = fetch_challenge(location)
        .await
        .map_err(TokenError::Failed)?
        .ok_or(TokenError::NotSupported)?;

    exchange(location, &challenge, username, password, repositories)
        .await
        .map_err(TokenError::Failed)
}

async fn fetch_challenge(location: &RegistryLocation) -> Result<Option<BearerChallenge>> {
    let (client, url) = guarded_client(&format!("{}/v2/", location.base_url())).await?;
    let response = client.get(url).send().await?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(None);
    }
    Ok(response
        .headers()
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(parse_bearer_challenge))
}

async fn exchange(
    location: &RegistryLocation,
    challenge: &BearerChallenge,
    username: &str,
    password: &str,
    repositories: &[String],
) -> Result<String> {
    // The password is about to be sent to the realm, so it gets the registry's own
    // transport security at least.
    if !location.plain_http && !challenge.realm.starts_with("https://") {
        bail!(
            "registry {} names a token endpoint that is not HTTPS",
            location.host
        );
    }

    let (client, mut url) = guarded_client(&challenge.realm).await?;
    {
        let mut query = url.query_pairs_mut();
        if let Some(service) = &challenge.service {
            query.append_pair("service", service);
        }
        for repository in repositories {
            query.append_pair("scope", &format!("repository:{}:pull,push", repository));
        }
    }

    let response = client
        .get(url)
        .basic_auth(username, Some(password))
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        bail!("token endpoint answered {}", status);
    }
    let body: Value =
        serde_json::from_slice(&read_body_capped(response, MAX_TOKEN_RESPONSE_BYTES).await?)
            .context("token endpoint did not answer with JSON")?;

    body.get("token")
        .or_else(|| body.get("access_token"))
        .and_then(|t| t.as_str())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("token endpoint answered without a token"))
}

/// A Docker client configuration that authenticates to `host` with a bearer token.
pub fn token_docker_config(host: &str, token: &str) -> Value {
    json!({ "auths": { host: { "registrytoken": token } } })
}

/// A Docker client configuration that authenticates to `host` with a username and password.
pub fn basic_docker_config(host: &str, username: &str, password: &str) -> Value {
    let auth =
        base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password));
    json!({ "auths": { host: { "auth": auth } } })
}

/// True when the operator lets the engine hand a registry's stored username and password
/// to a runner's helper because the registry cannot issue scoped tokens
/// (`STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS=true`). Off by default.
pub fn static_credentials_allowed() -> bool {
    std::env::var("STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_settings() {
        assert_eq!(
            RegistryLocation::parse("registry.example.com", false).unwrap(),
            RegistryLocation {
                host: "registry.example.com".into(),
                prefix: "".into(),
                plain_http: false
            }
        );
        assert_eq!(
            RegistryLocation::parse("GHCR.io/org/team/", false).unwrap(),
            RegistryLocation {
                host: "ghcr.io".into(),
                prefix: "org/team".into(),
                plain_http: false
            }
        );
        let local = RegistryLocation::parse("http://localhost:5000/x", false).unwrap();
        assert!(local.plain_http);
        assert_eq!(local.host, "localhost:5000");
        assert!(RegistryLocation::parse("", false).is_err());
        assert!(RegistryLocation::parse("user@evil.example/x", false).is_err());
    }

    #[test]
    fn names_a_runs_repository() {
        let run_id = Uuid::nil();
        let plain = RegistryLocation::parse("r.example.com", false).unwrap();
        assert_eq!(
            plain.artifact_repository(run_id, "ws"),
            format!("{}/ws", run_id)
        );
        let nested = RegistryLocation::parse("r.example.com/org", false).unwrap();
        assert_eq!(
            nested.artifact_repository(run_id, "ws"),
            format!("org/{}/ws", run_id)
        );
    }

    #[test]
    fn parses_bearer_challenges() {
        assert_eq!(
            parse_bearer_challenge(
                r#"Bearer realm="https://auth.example.com/token",service="registry.example.com",scope="repository:a:pull""#
            ),
            Some(BearerChallenge {
                realm: "https://auth.example.com/token".into(),
                service: Some("registry.example.com".into()),
            })
        );
        assert_eq!(
            parse_bearer_challenge(r#"bearer realm="https://a/t""#),
            Some(BearerChallenge {
                realm: "https://a/t".into(),
                service: None,
            })
        );
        assert_eq!(parse_bearer_challenge(r#"Basic realm="Registry""#), None);
        assert_eq!(parse_bearer_challenge(r#"Bearer service="x""#), None);
        assert_eq!(parse_bearer_challenge("garbage"), None);
    }

    #[test]
    fn builds_docker_configs() {
        assert_eq!(
            token_docker_config("r.example.com", "tok")["auths"]["r.example.com"]["registrytoken"],
            "tok"
        );
        // base64("user:pass")
        assert_eq!(
            basic_docker_config("r.example.com", "user", "pass")["auths"]["r.example.com"]["auth"],
            "dXNlcjpwYXNz"
        );
    }
}
