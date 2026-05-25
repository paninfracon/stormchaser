use axum::{response::IntoResponse, Json};
use stormchaser_model::schema_gen::generate_dsl_schema;
use crate::{AppState, AuthClaims};

/// Retrieves the base JSON schema for the Stormchaser DSL.
#[utoipa::path(
    get,
    path = "/api/v1/schema",
    tag = "stormchaser",
    responses(
        (status = 200, description = "JSON schema retrieved successfully", body = serde_json::Value)
    )
)]
pub async fn get_schema() -> impl IntoResponse {
    let schema = generate_dsl_schema();
    Json(schema)
}

#[utoipa::path(
    post,
    path = "/api/v1/schema/parse-git",
    request_body = crate::routes::ParseGitRequest,
    responses(
        (status = 200, description = "Parsed successfully", body = String),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Connection not found"),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
pub async fn parse_git(
    axum::extract::State(state): axum::extract::State<AppState>,
    AuthClaims(_claims): AuthClaims,
    Json(payload): Json<crate::routes::ParseGitRequest>,
) -> Result<String, axum::http::StatusCode> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    let conn = if let Ok(id) = uuid::Uuid::parse_str(&payload.connection) {
        crate::db::get_connection(&mut *tx, stormchaser_model::ConnectionId::new(id))
            .await
            .map_err(|_| axum::http::StatusCode::NOT_FOUND)?
            .ok_or(axum::http::StatusCode::NOT_FOUND)?
    } else {
        crate::db::get_connection_by_name(&mut *tx, &payload.connection)
            .await
            .map_err(|_| axum::http::StatusCode::NOT_FOUND)?
            .ok_or(axum::http::StatusCode::NOT_FOUND)?
    };

    if conn.connection_type != stormchaser_model::connections::ConnectionType::Git {
        tracing::error!("Connection {} is not a Git connection", payload.connection);
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }

    let repo_url = conn
        .config
        .get("repo_url")
        .and_then(|v| v.as_str())
        .or_else(|| conn.config.get("url").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();

    if repo_url.is_empty() {
        tracing::error!(
            "Git connection {} is missing repo_url in config",
            payload.connection
        );
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }

    // Validate workflow_path to prevent path traversal
    let wf_path = std::path::Path::new(&payload.workflow_path);
    if wf_path.is_absolute()
        || wf_path
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        tracing::error!(
            "Rejected unsafe workflow_path: {:?}",
            payload.workflow_path
        );
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }

    // Use GitCache to clone/fetch the file
    let git_cache = stormchaser_engine::git_cache::GitCache::new("/tmp/stormchaser-api-git-cache");
    let target_dir = git_cache
        .ensure_files(
            &repo_url,
            &payload.git_ref,
            std::slice::from_ref(&payload.workflow_path),
        )
        .map_err(|e| {
            tracing::error!("Failed to fetch git repo: {:?}", e);
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let file_path = target_dir.join(&payload.workflow_path);
    let dsl = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
        tracing::error!("Failed to read workflow file {:?}: {:?}", file_path, e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(dsl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::{routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_get_schema() {
        let app = Router::new().route("/api/v1/schema", get(get_schema));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/schema")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(
            json.get("properties").is_some(),
            "Should contain properties"
        );
    }
}
