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

/// Fetches JSON Web Key Set (JWKS) from a specified URL
pub async fn fetch_jwks(jwks_url: &str) -> JwksCache {
    let mut jwks = HashMap::new();
    let retry_policy =
        reqwest_retry::policies::ExponentialBackoff::builder().build_with_max_retries(3);
    let client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
        .with(reqwest_retry::RetryTransientMiddleware::new_with_policy(
            retry_policy,
        ))
        .build();

    match client.get(jwks_url).send().await {
        Ok(resp) => {
            if let Ok(jwks_set) = resp.json::<jsonwebtoken::jwk::JwkSet>().await {
                for jwk in jwks_set.keys {
                    if let Some(kid) = &jwk.common.key_id {
                        jwks.insert(kid.clone(), jwk);
                    }
                }
                tracing::info!("Successfully fetched {} keys from JWKS", jwks.len());
            } else {
                tracing::error!("Failed to parse JWKS response from {}", jwks_url);
            }
        }
        Err(e) => {
            tracing::error!("Failed to fetch JWKS from {}: {:?}", jwks_url, e);
        }
    }
    jwks
}
