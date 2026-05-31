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

/// Built-in development JWT secret. INSECURE — used only as a debug-build
/// fallback when `STORMCHASER_JWT_SECRET` is unset (see `resolve_jwt_secret`).
pub const JWT_SECRET: &[u8] = b"stormchaser-secret-dev-only";

/// Resolve the secret used to validate local (non-OIDC) HS256 tokens.
///
/// Production MUST set `STORMCHASER_JWT_SECRET`. When it is unset:
/// - **debug/test** builds fall back to the built-in dev constant (with a
///   warning) so local development and the test suite keep working;
/// - **release** builds REFUSE local-token validation (`None`) — failing
///   closed so a deploy that forgets the env var can't be bypassed using the
///   well-known compiled-in secret. (Previously the constant was always used,
///   letting anyone who read the public source forge tokens for any identity.)
fn resolve_jwt_secret() -> Option<Vec<u8>> {
    if let Ok(s) = std::env::var("STORMCHASER_JWT_SECRET") {
        if !s.is_empty() {
            return Some(s.into_bytes());
        }
    }
    #[cfg(debug_assertions)]
    {
        tracing::warn!(
            "STORMCHASER_JWT_SECRET not set — using the INSECURE built-in dev secret (debug build only)"
        );
        Some(JWT_SECRET.to_vec())
    }
    #[cfg(not(debug_assertions))]
    {
        tracing::error!(
            "STORMCHASER_JWT_SECRET not set — refusing local-token validation in a release build"
        );
        None
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

        // 2. Fallback to local secret for legacy/dev tokens.
        // The secret is env-provided (`STORMCHASER_JWT_SECRET`); release builds
        // fail closed if it is unset rather than trusting the well-known
        // compiled-in dev constant. See `resolve_jwt_secret`.
        let secret = resolve_jwt_secret().ok_or(StatusCode::UNAUTHORIZED)?;

        let mut validation = Validation::default();
        validation.validate_exp = true;
        // Skip audience/issuer check for local tokens as they don't have them set usually in the current model
        validation.required_spec_claims.remove("aud");

        let token_data =
            decode::<Claims>(token, &DecodingKey::from_secret(&secret), &validation)
                .inspect_err(|e| tracing::error!("JWT decode failed: {:?}", e))
                .map_err(|_| StatusCode::UNAUTHORIZED)?;

        Ok(AuthClaims(token_data.claims))
    }
}
