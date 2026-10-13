use std::collections::HashMap;

/// Configuration for OIDC authentication
#[derive(Clone)]
pub struct OidcConfig {
    /// OIDC issuer URL
    pub issuer: String,
    /// External issuer URL
    pub external_issuer: String,
    /// OIDC client ID
    pub client_id: String,
    /// OIDC client secret
    pub client_secret: String,
    /// URL to fetch JWKS
    pub jwks_url: String,
}

/// Type alias for JWKS cache
pub type JwksCache = HashMap<String, jsonwebtoken::jwk::Jwk>;

use std::sync::OnceLock;

static HTTP_CLIENT: OnceLock<reqwest_middleware::ClientWithMiddleware> = OnceLock::new();

fn get_client() -> reqwest_middleware::ClientWithMiddleware {
    HTTP_CLIENT
        .get_or_init(|| {
            let retry_policy =
                reqwest_retry::policies::ExponentialBackoff::builder().build_with_max_retries(3);
            reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
                .with(reqwest_retry::RetryTransientMiddleware::new_with_policy(
                    retry_policy,
                ))
                .build()
        })
        .clone()
}

/// Fetches JSON Web Key Set (JWKS) from a specified URL
pub async fn fetch_jwks(jwks_url: &str) -> JwksCache {
    try_fetch_jwks(jwks_url).await.unwrap_or_default()
}

/// Fetches the key set, or `None` if it could not be fetched or held no usable key.
async fn try_fetch_jwks(jwks_url: &str) -> Option<JwksCache> {
    let client = get_client();

    let resp = match client.get(jwks_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Failed to fetch JWKS from {}: {:?}", jwks_url, e);
            return None;
        }
    };
    let Ok(jwks_set) = resp.json::<jsonwebtoken::jwk::JwkSet>().await else {
        tracing::error!("Failed to parse JWKS response from {}", jwks_url);
        return None;
    };
    let jwks: JwksCache = jwks_set
        .keys
        .into_iter()
        .filter_map(|jwk| jwk.common.key_id.clone().map(|kid| (kid, jwk)))
        .collect();
    tracing::info!("Successfully fetched {} keys from JWKS", jwks.len());
    (!jwks.is_empty()).then_some(jwks)
}

/// Shortest time between two refreshes of one key set that were prompted by a token.
const REFRESH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

static LAST_REFRESH: OnceLock<tokio::sync::Mutex<HashMap<String, std::time::Instant>>> =
    OnceLock::new();

/// Looks up the key a token names, refreshing the cache if it is not there.
///
/// The key id comes from an unverified token header, so anyone can send one that is not
/// in the cache. The refresh is therefore limited to one per [`REFRESH_COOLDOWN`] for
/// each key set, and a refresh that fails or returns nothing leaves the cached keys in
/// place: otherwise a stream of made-up key ids would hammer the identity provider, and
/// one failed fetch would lock every user out.
pub async fn key_for(
    cache: &tokio::sync::RwLock<JwksCache>,
    jwks_url: &str,
    kid: &str,
) -> Option<jsonwebtoken::jwk::Jwk> {
    if let Some(jwk) = cache.read().await.get(kid).cloned() {
        return Some(jwk);
    }

    {
        let mut last = LAST_REFRESH.get_or_init(Default::default).lock().await;
        let now = std::time::Instant::now();
        if last
            .get(jwks_url)
            .is_some_and(|at| now.duration_since(*at) < REFRESH_COOLDOWN)
        {
            return None;
        }
        last.insert(jwks_url.to_string(), now);
    }

    tracing::warn!("kid {} not found in JWKS cache, attempting refresh", kid);
    if let Some(fresh) = try_fetch_jwks(jwks_url).await {
        *cache.write().await = fresh;
    }
    cache.read().await.get(kid).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn key_set(kid: &str) -> serde_json::Value {
        serde_json::json!({ "keys": [{
            "kty": "RSA", "kid": kid, "use": "sig", "alg": "RS256",
            "n": "sXchDaQebHnPiGvyDOAT4saGEUetSyo9MKLOoWFsueri23bOdgWp4Dy1WlUzewbgBHod5pcM9H95GQRV3JDXboIRROSBigeC5yjU1hGzHHyXss8UDprecbAYxknTcQkhslANGRUZmdTOQ5qTRsLAt6BTYuyvVRdhS8exSZEy_c4gs_7svlJJQ4H9_NxsiIoLwAEk7-Q3UXERGYw_75IDrGA84-lA_-Ct4eTlXHBIY2EaV7t7LjJaynVJCpkv4LKjTTAumiGUIuQhrNhZLuF_RJLqHpM2kgWFLU7-VTdL1VbC2tejvcI2BlMkEpk1BzBZI0KQB0GaDWFLN-aEAw3vRw",
            "e": "AQAB"
        }]})
    }

    #[tokio::test]
    async fn unknown_key_ids_cannot_force_repeated_refreshes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(key_set("current")))
            .expect(1)
            .mount(&server)
            .await;
        let url = format!("{}/keys", server.uri());
        let cache = tokio::sync::RwLock::new(JwksCache::new());

        // The first unknown key triggers the one refresh, which also finds the real key
        assert!(key_for(&cache, &url, "made-up-1").await.is_none());
        assert!(key_for(&cache, &url, "current").await.is_some());
        // More made-up ids inside the cooldown do not reach the identity provider:
        // the mock verifies it was called exactly once
        for n in 2..20 {
            assert!(key_for(&cache, &url, &format!("made-up-{n}"))
                .await
                .is_none());
        }
    }

    #[tokio::test]
    async fn a_failed_refresh_keeps_the_keys_already_cached() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"keys": []})))
            .mount(&server)
            .await;
        let url = format!("{}/keys", server.uri());
        let mut known = JwksCache::new();
        let jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(key_set("current")["keys"][0].clone()).unwrap();
        known.insert("current".to_string(), jwk);
        let cache = tokio::sync::RwLock::new(known);

        assert!(key_for(&cache, &url, "made-up").await.is_none());
        assert!(key_for(&cache, &url, "current").await.is_some());
    }
}
