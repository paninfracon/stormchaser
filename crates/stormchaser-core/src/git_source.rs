//! Reading a workflow file from the repository of a Git connection.

use crate::service::ServiceState;
use serde::Deserialize;
use utoipa::ToSchema;

/// Where a workflow file is: a Git connection, a path in its repository and a ref.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ParseGitRequest {
    /// The connection (ID or name) to the repository.
    pub connection: String,
    /// The workflow path.
    pub workflow_path: String,
    /// The git ref.
    pub git_ref: String,
}

/// Reads a workflow file from the repository of a Git connection, as `caller`.
///
/// The caller must be allowed to use the connection: this returns a file's contents, and
/// whoever may read one path of a repository through it may read them all.
pub async fn read_workflow_from_git<S: ServiceState>(
    state: &S,
    caller: &crate::access::Caller,
    payload: &ParseGitRequest,
) -> Result<String, axum::http::StatusCode> {
    let mut tx = state
        .pool()
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
    drop(tx);

    if conn.connection_type != stormchaser_model::connections::ConnectionType::Git {
        tracing::error!("Connection {} is not a Git connection", payload.connection);
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }

    crate::access::authorize_connection_use(state, caller, conn.id.into_inner()).await?;

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
        tracing::error!("Rejected unsafe workflow_path: {:?}", payload.workflow_path);
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }

    // Clone (if needed) and read the file from the git object database, off the async
    // executor. Reading the blob rather than the working tree means a symlink committed to
    // the repository cannot be used to read files from this host.
    let git_ref = payload.git_ref.clone();
    let workflow_path = payload.workflow_path.clone();
    let dsl = tokio::task::spawn_blocking(move || {
        crate::git_cache::GitCache::new("/tmp/stormchaser-api-git-cache").read_file(
            &repo_url,
            &git_ref,
            &workflow_path,
        )
    })
    .await
    .map_err(|e| {
        tracing::error!("Git read task failed: {:?}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?
    .map_err(|e| {
        tracing::error!("Failed to read workflow file from git: {:?}", e);
        axum::http::StatusCode::BAD_REQUEST
    })?;

    Ok(dsl)
}
