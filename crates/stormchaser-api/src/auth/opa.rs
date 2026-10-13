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
    let path = request
        .extensions()
        .get::<OriginalUri>()
        .map(|original| original.0.path().to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let method = request.method().to_string();

    // Extract token manually from headers if present
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let context = ApiOpaContext {
        path: &path,
        method: &method,
        token,
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
