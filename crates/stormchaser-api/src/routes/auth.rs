use super::{AuthExchangeRequest, AuthExchangeResponse, AuthRefreshRequest};
use crate::auth;
use crate::{AppState, Claims};
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

#[derive(Debug, Deserialize)]
/// Loginquery.
pub struct LoginQuery {
    /// The callback url.
    pub callback_url: String,
    /// Optional OAuth state parameter.
    pub state: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/login",
    params(
        ("callback_url" = String, Query, description = "The local URL to redirect back to after login")
    ),
    responses(
        (status = 303, description = "Redirect to OIDC provider")
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

    let mut auth_url = format!(
        "{}/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email+offline_access",
        oidc_config.external_issuer,
        oidc_config.client_id,
        urlencoding::encode(&query.callback_url)
    );

    if let Some(state) = &query.state {
        auth_url.push_str(&format!("&state={}", urlencoding::encode(state)));
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

    // 1. Exchange code for tokens
    let client = Client::new();
    let token_url = format!("{}/token", oidc_config.issuer.trim_end_matches('/'));

    let params = [
        ("grant_type", "authorization_code"),
        ("code", &payload.sso_token),
        ("client_id", &oidc_config.client_id),
        ("client_secret", &oidc_config.client_secret),
        ("redirect_uri", &payload.callback_url),
    ];

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

    let jwk_opt = state.jwks.read().await.get(&kid).cloned();
    let jwk = match jwk_opt {
        Some(j) => j,
        None => {
            tracing::warn!("kid {} not found in JWKS cache, attempting refresh", kid);
            let new_jwks = auth::jwks::fetch_jwks(&oidc_config.jwks_url).await;
            let mut jwks_write = state.jwks.write().await;
            *jwks_write = new_jwks;

            match jwks_write.get(&kid) {
                Some(j) => j.clone(),
                None => {
                    tracing::error!("kid {} not found in JWKS cache even after refresh", kid);
                    return Err(StatusCode::UNAUTHORIZED);
                }
            }
        }
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

    let token_data = match decode::<Claims>(&token_res.id_token, &decoding_key, &validation) {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to validate id_token: {:?}", e);
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    // 3. Generate Stormchaser Access Token
    let user_id = token_data.claims.sub;
    let email = token_data.claims.email;

    let expires_in = 3600;
    let expiration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + expires_in;

    let claims = Claims {
        sub: user_id,
        email,
        exp: expiration,
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

    let jwk_opt = state.jwks.read().await.get(&kid).cloned();
    let jwk = match jwk_opt {
        Some(j) => j,
        None => {
            tracing::warn!("kid {} not found in JWKS cache, attempting refresh", kid);
            let new_jwks = auth::jwks::fetch_jwks(&oidc_config.jwks_url).await;
            let mut jwks_write = state.jwks.write().await;
            *jwks_write = new_jwks;

            match jwks_write.get(&kid) {
                Some(j) => j.clone(),
                None => {
                    tracing::error!("kid {} not found in JWKS cache even after refresh", kid);
                    return Err(StatusCode::UNAUTHORIZED);
                }
            }
        }
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

    let token_data =
        decode::<Claims>(&token_res.id_token, &decoding_key, &validation).map_err(|e| {
            tracing::error!("Failed to validate id_token: {:?}", e);
            StatusCode::UNAUTHORIZED
        })?;

    let user_id = token_data.claims.sub;
    let email = token_data.claims.email;
    let expires_in = 3600;
    let expiration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + expires_in;

    let claims = Claims {
        sub: user_id,
        email,
        exp: expiration,
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
