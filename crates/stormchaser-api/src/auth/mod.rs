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

/// `STORMCHASER_OIDC_REQUIRE_EMAIL_VERIFIED`: `true` (the default) refuses an identity
/// provider's token that names an email without `"email_verified": true`.
pub const REQUIRE_EMAIL_VERIFIED_ENV: &str = "STORMCHASER_OIDC_REQUIRE_EMAIL_VERIFIED";

/// The claims Stormchaser reads from an identity provider's token.
#[derive(Debug, Deserialize)]
pub struct IdpClaims {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub exp: usize,
}

impl IdpClaims {
    /// The user this token identifies, if it may be trusted to.
    ///
    /// A user's email is what owners, admins and approvers are recorded by, so an email
    /// the provider has not verified would let someone who registered another person's
    /// address there act as them. Such a token is refused, unless the operator turned
    /// the check off for a provider that only issues verified addresses but does not
    /// say so. A token with no email is identified by its subject alone.
    pub fn into_claims(self) -> Result<Claims, StatusCode> {
        let required = !matches!(
            std::env::var(REQUIRE_EMAIL_VERIFIED_ENV).as_deref(),
            Ok("false") | Ok("FALSE") | Ok("0")
        );
        if required && self.email.is_some() && self.email_verified != Some(true) {
            tracing::warn!(
                "Refusing a token for subject {}: its email is not marked verified ({}=false accepts it)",
                self.sub,
                REQUIRE_EMAIL_VERIFIED_ENV
            );
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(Claims {
            sub: self.sub,
            email: self.email,
            exp: self.exp,
        })
    }
}

/// Extractor for authenticated user claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims(pub Claims);

/// Validates the bearer token in `headers` and returns its claims.
///
/// Tokens issued by the configured OIDC provider are checked against its keys, audience
/// and issuer; otherwise the token must be signed with the local secret.
pub async fn authenticate(
    headers: &axum::http::HeaderMap,
    state: &AppState,
) -> Result<Claims, StatusCode> {
    let auth_header = headers
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
                let jwk_opt =
                    crate::auth::jwks::key_for(&state.jwks, &oidc_config.jwks_url, &kid).await;

                if let Some(jwk) = jwk_opt {
                    let mut validation = Validation::new(header.alg);
                    validation.set_audience(std::slice::from_ref(&oidc_config.client_id));
                    validation.set_issuer(&[
                        oidc_config.issuer.as_str(),
                        oidc_config.external_issuer.as_str(),
                    ]);

                    if let Ok(decoding_key) = DecodingKey::from_jwk(&jwk) {
                        if let Ok(token_data) =
                            decode::<IdpClaims>(token, &decoding_key, &validation)
                        {
                            return token_data.claims.into_claims();
                        }
                    }
                }
            }
        }
    }

    // 2. Fallback to local secret for legacy/dev tokens
    let mut validation = Validation {
        validate_exp: true,
        ..Default::default()
    };
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

    Ok(token_data.claims)
}

/// Middleware that rejects any request without a valid bearer token.
///
/// It guards the whole protected route group, so a handler that does not ask for
/// [`AuthClaims`] is still unreachable to unauthenticated callers. The validated claims
/// are stored on the request for the extractor to reuse.
pub async fn require_auth(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let claims = authenticate(request.headers(), &state).await?;
    request.extensions_mut().insert(AuthClaims(claims));
    Ok(next.run(request).await)
}

impl FromRequestParts<AppState> for AuthClaims {
    type Rejection = StatusCode;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Already validated by `require_auth` when the route sits behind it.
        if let Some(claims) = parts.extensions.get::<AuthClaims>() {
            return Ok(claims.clone());
        }
        authenticate(&parts.headers, state).await.map(AuthClaims)
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_jwt_secret, IdpClaims, JWT_SECRET, REQUIRE_EMAIL_VERIFIED_ENV};
    use std::sync::{Mutex, OnceLock};

    fn idp(email: Option<&str>, verified: Option<bool>) -> IdpClaims {
        IdpClaims {
            sub: "sub-1".to_string(),
            email: email.map(str::to_string),
            email_verified: verified,
            exp: 0,
        }
    }

    #[test]
    fn an_identity_providers_email_is_trusted_only_when_verified() {
        let _guard = jwt_secret_env_lock().lock().expect("lock poisoned");
        unsafe {
            std::env::remove_var(REQUIRE_EMAIL_VERIFIED_ENV);
        }
        let claims = idp(Some("a@example.com"), Some(true))
            .into_claims()
            .unwrap();
        assert_eq!(claims.email.as_deref(), Some("a@example.com"));
        assert!(idp(Some("a@example.com"), Some(false))
            .into_claims()
            .is_err());
        assert!(idp(Some("a@example.com"), None).into_claims().is_err());
        // No email: identified by subject, nothing to impersonate
        assert_eq!(idp(None, None).into_claims().unwrap().sub, "sub-1");

        unsafe {
            std::env::set_var(REQUIRE_EMAIL_VERIFIED_ENV, "false");
        }
        assert!(idp(Some("a@example.com"), None).into_claims().is_ok());
        unsafe {
            std::env::remove_var(REQUIRE_EMAIL_VERIFIED_ENV);
        }
    }

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
