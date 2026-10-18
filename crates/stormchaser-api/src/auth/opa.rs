use super::AuthClaims;
use crate::AppState;
use anyhow::Result;
use axum::{
    extract::{OriginalUri, Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::Response,
};
use stormchaser_model::auth::ApiOpaContext;
use tracing::{debug, error, info};

/// Middleware to evaluate Open Policy Agent (OPA) rules for incoming requests
pub async fn opa_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let authorizer = &*state.opa;
    if !authorizer.is_configured() {
        return Ok(next.run(request).await);
    }

    // The path the client asked for. Inside the nested `/api/v1` router the request's
    // own URI has that prefix stripped, and policies are written against the full path.
    // It is percent-decoded, as handlers see it, so that a policy matching
    // `/api/v1/runs/direct` also matches `/api/v1/runs/dire%63t`.
    let raw_path = request
        .extensions()
        .get::<OriginalUri>()
        .map(|original| original.0.path().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let path =
        String::from_utf8_lossy(&urlencoding::decode_binary(raw_path.as_bytes())).into_owned();
    let method = request.method().to_string();

    // Policy is told who is asking only as far as their token has been verified: by
    // `require_auth` on the authenticated routes, or here on routes without it (the query
    // service). The token itself never goes to policy.
    let claims = match request.extensions().get::<AuthClaims>() {
        Some(AuthClaims(claims)) => Some(claims.clone()),
        None if request.headers().contains_key(header::AUTHORIZATION) => {
            // Policy sees an anonymous caller; say why, or a rejected token is a mystery
            super::authenticate(request.headers(), &state)
                .await
                .inspect_err(|e| tracing::debug!("Token not accepted for policy: {:?}", e))
                .ok()
        }
        None => None,
    };

    let context = ApiOpaContext {
        path: &path,
        method: &method,
        subject: claims.as_ref().map(|claims| claims.opa_subject()),
    };

    match authorizer.check(context).await {
        Ok(true) => {
            debug!("OPA allowed access to {} {}", method, path);
            Ok(next.run(request).await)
        }
        Ok(false) => {
            info!("OPA denied access to {} {}", method, path);
            Err(StatusCode::FORBIDDEN)
        }
        Err(e) => {
            error!("OPA check failed for {} {}: {:?}", method, path, e);
            // Hard failure if OPA is unreachable or errors
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
