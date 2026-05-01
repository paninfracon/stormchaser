use axum::{response::IntoResponse, Json};
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn test_get_schema() {
        let response = get_schema().await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
