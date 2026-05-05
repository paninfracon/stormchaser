use super::CreateEventRuleRequest;
use crate::{AppState, AuthClaims};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use uuid::Uuid;

/// Creates an event rule.
#[utoipa::path(
    post,
    path = "/api/v1/rules",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "event_rule"
)]
pub async fn create_event_rule(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<CreateEventRuleRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let id = Uuid::new_v4();
    crate::db::create_event_rule(
        &state.pool,
        id,
        &payload.name,
        &payload.description,
        Some(payload.webhook_id),
        &payload.event_type_pattern,
        &payload.condition_expr,
        &payload.workflow_name,
        &payload.repo_url,
        &payload.workflow_path,
        &payload.git_ref,
        serde_json::to_value(&payload.input_mappings).unwrap_or(serde_json::json!({})),
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create rule: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

/// Lists event rules.
#[utoipa::path(
    get,
    path = "/api/v1/rules",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "event_rule"
)]
pub async fn list_event_rules(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let rules = crate::db::list_event_rules(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(rules))
}

#[utoipa::path(
    delete,
    path = "/api/v1/rules/{id}",
    params(
        ("id" = Uuid, Path, description = "Event rule ID")
    ),
    responses(
        (status = 204, description = "Event rule deleted"),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "event_rule"
)]
/// Deletes an event rule.
pub async fn delete_event_rule(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, StatusCode> {
    crate::db::delete_event_rule(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}
