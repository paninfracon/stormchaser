use super::CreateEventRuleRequest;
use crate::authz::{self, Caller, Kind};
use crate::db;
use crate::{AppState, AuthClaims};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use stormchaser_model::RuleId;

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
    caller: Caller,
    State(state): State<AppState>,
    Json(payload): Json<CreateEventRuleRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let id = RuleId::new_v4();

    let conn = if let Ok(id) = uuid::Uuid::parse_str(&payload.connection) {
        crate::db::get_connection(&state.pool, stormchaser_model::ConnectionId::new(id))
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?
            .ok_or(StatusCode::NOT_FOUND)?
    } else {
        crate::db::get_connection_by_name(&state.pool, &payload.connection)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?
            .ok_or(StatusCode::NOT_FOUND)?
    };
    authz::authorize_connection_use(&state, &caller, conn.id.into_inner()).await?;
    // A rule decides what the webhook's deliveries start, with their payloads as inputs,
    // so attaching one is a change to the webhook: only its owner (or an admin) may
    authz::authorize_action(
        &state,
        &caller,
        "update",
        Kind::Webhook,
        payload.webhook_id.into_inner(),
    )
    .await?;

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
        return Err(StatusCode::BAD_REQUEST);
    }

    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!("Failed to start a transaction: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    db::create_event_rule(
        &mut *tx,
        id,
        &payload.name,
        &payload.description,
        Some(payload.webhook_id),
        &payload.event_type_pattern,
        &payload.condition_expr,
        &payload.workflow_name,
        &repo_url,
        &payload.workflow_path,
        &payload.git_ref,
        serde_json::to_value(&payload.input_mappings).unwrap_or(serde_json::json!({})),
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create rule: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    authz::record_owner(&mut *tx, Kind::EventRule, id.into_inner(), &caller).await?;
    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
    caller: Caller,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let rules = db::list_event_rules(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let rules = authz::filter_readable(&state, &caller, Kind::EventRule, rules, |r| {
        r.id.into_inner()
    })
    .await?;

    Ok(Json(rules))
}

#[utoipa::path(
    delete,
    path = "/api/v1/rules/{id}",
    params(
        ("id" = stormchaser_model::RuleId, Path, description = "Event rule ID")
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
    Path(id): Path<RuleId>,
) -> Result<impl IntoResponse, StatusCode> {
    db::delete_event_rule(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}
