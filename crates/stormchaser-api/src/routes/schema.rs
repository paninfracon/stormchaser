use crate::AppState;
use axum::{response::IntoResponse, Json};
use stormchaser_core::git_source::read_workflow_from_git;
use stormchaser_model::schema_gen::generate_dsl_schema;

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
    caller: crate::authz::Caller,
    Json(payload): Json<crate::routes::ParseGitRequest>,
) -> Result<String, axum::http::StatusCode> {
    read_workflow_from_git(&state, &caller, &payload).await
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
