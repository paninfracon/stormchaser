pub mod jwks;
/// OPA integration for authorization
pub mod opa;

use crate::AppState;
use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
pub use stormchaser_model::auth::Claims;

/// Fallback JWT secret for local development
pub const JWT_SECRET: &[u8] = b"stormchaser-secret-dev-only"; // Fallback for local dev

/// Resolves the JWT secret used for signing and validating local Stormchaser tokens.
pub fn resolve_jwt_secret() -> Result<Vec<u8>, &'static str> {
    match std::env::var("STORMCHASER_JWT_SECRET") {
        Ok(value) if !value.is_empty() => Ok(value.into_bytes()),
        Ok(_) => Err("STORMCHASER_JWT_SECRET must not be empty"),
        Err(_) if cfg!(debug_assertions) => Ok(JWT_SECRET.to_vec()),
        Err(_) => Err("STORMCHASER_JWT_SECRET must be set"),
    }
}

/// Extractor for authenticated user claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims(pub Claims);

#[axum::async_trait]
impl FromRequestParts<AppState> for AuthClaims {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;

        if !auth_header.starts_with("Bearer ") {
            return Err(StatusCode::UNAUTHORIZED);
        }

        let token = &auth_header["Bearer ".len()..];

        // 1. Try OIDC/Dex validation if configured
        if let Some(oidc_config) = &state.oidc_config {
            if let Ok(header) = decode_header(token) {
                if let Some(kid) = header.kid {
                    let mut jwk_opt = state.jwks.read().await.get(&kid).cloned();

                    if jwk_opt.is_none() {
                        tracing::warn!("kid {} not found in JWKS cache, attempting refresh", kid);
                        let new_jwks = crate::auth::jwks::fetch_jwks(&oidc_config.jwks_url).await;
                        let mut jwks_write = state.jwks.write().await;
                        *jwks_write = new_jwks;
                        jwk_opt = jwks_write.get(&kid).cloned();
                    }

                    if let Some(jwk) = jwk_opt {
                        let mut validation = Validation::new(header.alg);
                        validation.set_audience(std::slice::from_ref(&oidc_config.client_id));
                        validation.set_issuer(&[
                            oidc_config.issuer.as_str(),
                            oidc_config.external_issuer.as_str(),
                        ]);

                        if let Ok(decoding_key) = DecodingKey::from_jwk(&jwk) {
                            if let Ok(token_data) =
                                decode::<Claims>(token, &decoding_key, &validation)
                            {
                                return Ok(AuthClaims(token_data.claims));
                            }
                        }
                    }
                }
            }
        }

        // 2. Fallback to local secret for legacy/dev tokens
        let mut validation = Validation::default();
        validation.validate_exp = true;
        // Skip audience/issuer check for local tokens as they don't have them set usually in the current model
        validation.required_spec_claims.remove("aud");

        let jwt_secret = resolve_jwt_secret()
            .inspect_err(|e| tracing::error!("JWT secret configuration error: {}", e))
            .map_err(|_| StatusCode::UNAUTHORIZED)?;

        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(jwt_secret.as_slice()),
            &validation,
        )
        .inspect_err(|e| tracing::error!("JWT decode failed: {:?}", e))
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

        Ok(AuthClaims(token_data.claims))
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_jwt_secret, JWT_SECRET};
    use std::sync::{Mutex, OnceLock};

    fn jwt_secret_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_jwt_secret_uses_env_when_present() {
        let _guard = jwt_secret_env_lock().lock().expect("lock poisoned");
        unsafe {
            std::env::set_var("STORMCHASER_JWT_SECRET", "test-secret");
        }
        let resolved = resolve_jwt_secret().expect("secret should resolve from env");
        assert_eq!(resolved, b"test-secret".to_vec());
        unsafe {
            std::env::remove_var("STORMCHASER_JWT_SECRET");
        }
    }

    #[test]
    fn resolve_jwt_secret_rejects_empty_env() {
        let _guard = jwt_secret_env_lock().lock().expect("lock poisoned");
        unsafe {
            std::env::set_var("STORMCHASER_JWT_SECRET", "");
        }
        let err = resolve_jwt_secret().expect_err("empty secret should be rejected");
        assert_eq!(err, "STORMCHASER_JWT_SECRET must not be empty");
        unsafe {
            std::env::remove_var("STORMCHASER_JWT_SECRET");
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    fn resolve_jwt_secret_debug_fallback_matches_constant() {
        let _guard = jwt_secret_env_lock().lock().expect("lock poisoned");
        unsafe {
            std::env::remove_var("STORMCHASER_JWT_SECRET");
        }
        let resolved = resolve_jwt_secret().expect("debug fallback should resolve");
        assert_eq!(resolved, JWT_SECRET.to_vec());
    }
}
