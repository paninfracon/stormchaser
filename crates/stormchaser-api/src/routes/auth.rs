use super::{AuthExchangeRequest, AuthExchangeResponse, AuthRefreshRequest, SessionResponse};
use crate::auth;
use crate::{AppState, AuthClaims, Claims};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    Json,
};
use jsonwebtoken::decode;
use jsonwebtoken::decode_header;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::Validation;
use jsonwebtoken::{encode, EncodingKey, Header};
use reqwest::Client;
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// `STORMCHASER_OIDC_REQUIRE_PKCE`: `true` (the default) refuses a login without a PKCE
/// `code_challenge`, and a code exchange without its `code_verifier`.
///
/// The exchange endpoint needs no token and adds the client secret itself, so without
/// PKCE an authorization code is as good as the user's tokens to whoever sees it first,
/// for instance in a proxy's access log of the callback URL. With a challenge on every
/// login, the identity provider redeems a code only with the verifier the client kept.
pub const REQUIRE_PKCE_ENV: &str = "STORMCHASER_OIDC_REQUIRE_PKCE";

fn pkce_required() -> bool {
    !matches!(
        std::env::var(REQUIRE_PKCE_ENV).as_deref(),
        Ok("false") | Ok("FALSE") | Ok("0")
    )
}

/// Whether `value` is a PKCE verifier or S256 challenge in form: 43 to 128 characters
/// from the unreserved set (RFC 7636 section 4.1).
fn is_pkce_value(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// `STORMCHASER_OIDC_EXTRA_SCOPES`: scopes to ask the identity provider for on top of
/// `openid profile email offline_access`, separated by spaces or commas.
///
/// Dex puts a user's groups in the ID token only when asked for the `groups` scope, and
/// the policy's `admin_groups`, `author_groups` and group approvers need them. Not every
/// provider accepts that scope (some refuse a login that asks for a scope they do not
/// know), so it is not asked for unless configured.
pub const EXTRA_SCOPES_ENV: &str = "STORMCHASER_OIDC_EXTRA_SCOPES";

/// The `scope` parameter of the authorization request, already form-encoded.
fn login_scopes(extra: Option<&str>) -> String {
    let mut scopes = vec!["openid", "profile", "email", "offline_access"];
    for scope in extra
        .unwrap_or_default()
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes
        .iter()
        .map(|s| urlencoding::encode(s).into_owned())
        .collect::<Vec<_>>()
        .join("+")
}

#[derive(Debug, Deserialize)]
/// Loginquery.
pub struct LoginQuery {
    /// The callback url.
    pub callback_url: String,
    /// Optional OAuth state parameter.
    pub state: Option<String>,
    /// Optional PKCE challenge (S256) from a client that keeps the verifier.
    pub code_challenge: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/session",
    responses(
        (status = 200, description = "The token is valid", body = SessionResponse),
        (status = 401, description = "No valid token")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
/// Who the bearer token identifies. A cheap way for a client holding a token to check
/// that it is still valid.
pub async fn session(AuthClaims(claims): AuthClaims) -> Json<SessionResponse> {
    Json(SessionResponse {
        sub: claims.sub,
        email: claims.email,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/login",
    params(
        ("callback_url" = String, Query, description = "The local URL to redirect back to after login"),
        ("state" = Option<String>, Query, description = "Opaque value returned to the callback"),
        ("code_challenge" = Option<String>, Query, description = "PKCE S256 challenge; send the verifier with the code")
    ),
    responses(
        (status = 303, description = "Redirect to OIDC provider"),
        (status = 400, description = "No PKCE code_challenge, or one not in form")
    ),
    tag = "stormchaser"
)]
/// Login.
pub async fn login(
    State(state): State<AppState>,
    Query(query): Query<LoginQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let oidc_config = state
        .oidc_config
        .as_ref()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    match query.code_challenge.as_deref() {
        Some(challenge) if !is_pkce_value(challenge) => return Err(StatusCode::BAD_REQUEST),
        None if pkce_required() => {
            tracing::warn!(
                "Refusing a login without a PKCE code_challenge ({}=false allows it)",
                REQUIRE_PKCE_ENV
            );
            return Err(StatusCode::BAD_REQUEST);
        }
        _ => {}
    }

    let mut auth_url = format!(
        "{}/auth?client_id={}&redirect_uri={}&response_type=code&scope={}",
        oidc_config.external_issuer,
        oidc_config.client_id,
        urlencoding::encode(&query.callback_url),
        login_scopes(std::env::var(EXTRA_SCOPES_ENV).ok().as_deref())
    );

    if let Some(state) = &query.state {
        auth_url.push_str(&format!("&state={}", urlencoding::encode(state)));
    }
    if let Some(challenge) = &query.code_challenge {
        auth_url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            urlencoding::encode(challenge)
        ));
    }

    Ok(Redirect::to(&auth_url))
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: String,
    refresh_token: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/exchange",
    request_body = AuthExchangeRequest,
    responses(
        (status = 200, description = "Token exchanged successfully", body = AuthExchangeResponse),
        (status = 400, description = "No PKCE code_verifier, or one not in form"),
        (status = 401, description = "Unauthorized")
    ),
    tag = "stormchaser"
)]
/// Exchange token.
pub async fn exchange_token(
    State(state): State<AppState>,
    Json(payload): Json<AuthExchangeRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let oidc_config = state
        .oidc_config
        .as_ref()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    match payload.code_verifier.as_deref() {
        Some(verifier) if !is_pkce_value(verifier) => return Err(StatusCode::BAD_REQUEST),
        None if pkce_required() => {
            tracing::warn!(
                "Refusing a code exchange without a PKCE code_verifier ({}=false allows it)",
                REQUIRE_PKCE_ENV
            );
            return Err(StatusCode::BAD_REQUEST);
        }
        _ => {}
    }

    // 1. Exchange code for tokens
    let client = Client::new();
    let token_url = format!("{}/token", oidc_config.issuer.trim_end_matches('/'));

    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", payload.sso_token.as_str()),
        ("client_id", oidc_config.client_id.as_str()),
        ("client_secret", oidc_config.client_secret.as_str()),
        ("redirect_uri", payload.callback_url.as_str()),
    ];
    if let Some(verifier) = &payload.code_verifier {
        params.push(("code_verifier", verifier.as_str()));
    }

    let res = client
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to send token exchange request: {:?}", e);
            StatusCode::UNAUTHORIZED
        })?;

    if !res.status().is_success() {
        let status = res.status();
        let err_body = res.text().await.unwrap_or_default();
        tracing::error!("Token exchange failed with status {}: {}", status, err_body);
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token_res: TokenResponse = res.json().await.map_err(|e| {
        tracing::error!("Failed to parse token response: {:?}", e);
        StatusCode::UNAUTHORIZED
    })?;

    // 2. Validate the ID Token
    let header = match decode_header(&token_res.id_token) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to decode id_token header: {:?}", e);
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    let kid = match header.kid {
        Some(k) => k,
        None => {
            tracing::error!("No kid in id_token header");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    let Some(jwk) = auth::jwks::key_for(&state.jwks, &oidc_config.jwks_url, &kid).await else {
        tracing::error!("kid {} is not one of the identity provider's keys", kid);
        return Err(StatusCode::UNAUTHORIZED);
    };

    let mut validation = Validation::new(header.alg);
    validation.set_audience(std::slice::from_ref(&oidc_config.client_id));
    validation.set_issuer(&[
        oidc_config.issuer.as_str(),
        oidc_config.external_issuer.as_str(),
    ]);

    let decoding_key = match DecodingKey::from_jwk(&jwk) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!("Failed to create decoding key from JWK: {:?}", e);
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    let token_data =
        match decode::<auth::IdpClaims>(&token_res.id_token, &decoding_key, &validation) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("Failed to validate id_token: {:?}", e);
                return Err(StatusCode::UNAUTHORIZED);
            }
        };
    let identity = token_data.claims.into_claims()?;

    // 3. Generate Stormchaser Access Token

    let expires_in = 3600;
    let expiration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + expires_in;

    // The policy reads groups from this token, so they are carried over from the
    // provider's verified ID token
    let claims = Claims {
        sub: identity.sub,
        email: identity.email,
        exp: expiration,
        groups: identity.groups,
    };

    let jwt_secret = auth::resolve_jwt_secret().map_err(|e| {
        tracing::error!("JWT secret configuration error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_slice()),
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(AuthExchangeResponse {
        access_token: token,
        refresh_token: token_res.refresh_token,
        token_type: "Bearer".to_string(),
        expires_in,
    }))
}

/// Refreshes the auth token.
#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    request_body = AuthRefreshRequest,
    responses(
        (status = 200, description = "Token refreshed successfully", body = AuthExchangeResponse),
        (status = 401, description = "Unauthorized")
    ),
    tag = "stormchaser"
)]
/// Refresh token.
pub async fn refresh_token(
    State(state): State<AppState>,
    Json(payload): Json<AuthRefreshRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let oidc_config = state
        .oidc_config
        .as_ref()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    // Exchange refresh token for new tokens
    let client = Client::new();
    let token_url = format!("{}/token", oidc_config.issuer.trim_end_matches('/'));

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", &payload.refresh_token),
        ("client_id", &oidc_config.client_id),
        ("client_secret", &oidc_config.client_secret),
    ];

    let res = client
        .post(&token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to send refresh request: {:?}", e);
            StatusCode::UNAUTHORIZED
        })?;

    if !res.status().is_success() {
        let status = res.status();
        let err_body = res.text().await.unwrap_or_default();
        tracing::error!("Token refresh failed with status {}: {}", status, err_body);
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token_res: TokenResponse = res.json().await.map_err(|e| {
        tracing::error!("Failed to parse refresh response: {:?}", e);
        StatusCode::UNAUTHORIZED
    })?;

    // Validate the new ID Token
    let header = match decode_header(&token_res.id_token) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("Failed to decode id_token header: {:?}", e);
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    let kid = header.kid.ok_or_else(|| {
        tracing::error!("No kid in id_token header");
        StatusCode::UNAUTHORIZED
    })?;

    let Some(jwk) = auth::jwks::key_for(&state.jwks, &oidc_config.jwks_url, &kid).await else {
        tracing::error!("kid {} is not one of the identity provider's keys", kid);
        return Err(StatusCode::UNAUTHORIZED);
    };

    let mut validation = Validation::new(header.alg);
    validation.set_audience(std::slice::from_ref(&oidc_config.client_id));
    validation.set_issuer(&[
        oidc_config.issuer.as_str(),
        oidc_config.external_issuer.as_str(),
    ]);

    let decoding_key = DecodingKey::from_jwk(&jwk).map_err(|e| {
        tracing::error!("Failed to create decoding key: {:?}", e);
        StatusCode::UNAUTHORIZED
    })?;

    let token_data = decode::<auth::IdpClaims>(&token_res.id_token, &decoding_key, &validation)
        .map_err(|e| {
            tracing::error!("Failed to validate id_token: {:?}", e);
            StatusCode::UNAUTHORIZED
        })?;
    let identity = token_data.claims.into_claims()?;

    let expires_in = 3600;
    let expiration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + expires_in;

    // The policy reads groups from this token, so they are carried over from the
    // provider's verified ID token
    let claims = Claims {
        sub: identity.sub,
        email: identity.email,
        exp: expiration,
        groups: identity.groups,
    };

    let jwt_secret = auth::resolve_jwt_secret().map_err(|e| {
        tracing::error!("JWT secret configuration error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_slice()),
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(AuthExchangeResponse {
        access_token: token,
        refresh_token: token_res.refresh_token,
        token_type: "Bearer".to_string(),
        expires_in,
    }))
}

#[cfg(test)]
mod tests {
    use super::login_scopes;

    #[test]
    fn extra_scopes_are_added_once_and_encoded() {
        assert_eq!(login_scopes(None), "openid+profile+email+offline_access");
        assert_eq!(
            login_scopes(Some("")),
            "openid+profile+email+offline_access"
        );
        assert_eq!(
            login_scopes(Some("groups, email  federated:id")),
            "openid+profile+email+offline_access+groups+federated%3Aid"
        );
        // Nothing can be smuggled into the rest of the query string
        assert!(!login_scopes(Some("groups&prompt=none")).contains('&'));
    }
}
