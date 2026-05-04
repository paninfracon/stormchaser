use super::{CreateEventRuleRequest, CreateWebhookRequest, UpdateWebhookRequest};
use crate::{AppState, AuthClaims};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use stormchaser_model::event_rules::WebhookConfig;
use stormchaser_model::workflow::RunStatus;
use uuid::Uuid;

/// Create webhook.
#[utoipa::path(
    post,
    path = "/api/v1/webhooks",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "webhook"
)]
pub async fn create_webhook(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<CreateWebhookRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let id = Uuid::new_v4();
    crate::db::insert_webhook(
        &state.pool,
        id,
        &payload.name,
        &payload.description,
        &payload.source_type,
        &payload.secret_token,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create webhook: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

/// List webhooks.
#[utoipa::path(
    get,
    path = "/api/v1/webhooks",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "webhook"
)]
pub async fn list_webhooks(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let webhooks = crate::db::list_webhooks(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(webhooks))
}

/// Gets a webhook.
#[utoipa::path(
    get,
    path = "/api/v1/webhooks/{id}",
    params(("id" = Uuid, Path, description="Webhook ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "webhook"
)]
pub async fn get_webhook(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, StatusCode> {
    let webhook = crate::db::get_webhook(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(webhook))
}

/// Updates a webhook.
#[utoipa::path(
    patch,
    path = "/api/v1/webhooks/{id}",
    request_body = UpdateWebhookRequest,
    params(("id" = Uuid, Path, description="Webhook ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "webhook"
)]
pub async fn update_webhook(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateWebhookRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let description = match payload.description {
        Some(s) if s.is_empty() => Some(None),
        Some(s) => Some(Some(s)),
        None => None,
    };
    let secret_token = match payload.secret_token {
        Some(s) if s.is_empty() => Some(None),
        Some(s) => Some(Some(s)),
        None => None,
    };

    crate::db::update_webhook(
        &state.pool,
        id,
        payload.name,
        description,
        payload.source_type,
        secret_token,
        payload.is_active,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::OK)
}

/// Deletes a webhook.
#[utoipa::path(
    delete,
    path = "/api/v1/webhooks/{id}",
    params(("id" = Uuid, Path, description="Webhook ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "webhook"
)]
pub async fn delete_webhook(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, StatusCode> {
    crate::db::delete_webhook(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

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
    tag = "webhook"
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
    tag = "webhook"
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
    tag = "webhook"
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

#[utoipa::path(
    post,
    path = "/api/v1/webhooks/{id}",
    params(
        ("id" = Uuid, Path, description = "Webhook ID")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Webhook handled"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Webhook not found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "webhook"
)]
/// Handle webhook.
pub async fn handle_webhook(
    Path(webhook_id): Path<Uuid>,
    headers: HeaderMap,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    // 1. Fetch WebhookConfig
    let webhook: WebhookConfig = crate::db::get_active_webhook(&state.pool, webhook_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch webhook: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // 2. Validate Source/Signature
    let payload: Value = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let event_type = match webhook.source_type.as_str() {
        "github" => {
            validate_github_signature(&headers, &body, webhook.secret_token.as_deref())?;
            headers
                .get("X-GitHub-Event")
                .and_then(|h| h.to_str().ok())
                .unwrap_or("unknown")
                .to_string()
        }
        "generic" => "generic".to_string(),
        _ => return Err(StatusCode::NOT_IMPLEMENTED),
    };

    tracing::info!(
        "Received webhook event '{}' for webhook '{}' ({})",
        event_type,
        webhook.name,
        webhook.id
    );

    // 3. Find matching EventRules
    let rules = crate::db::get_active_event_rules_by_webhook(&state.pool, webhook_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch rules: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut triggered_count = 0;
    let mut hcl_ctx = hcl::eval::Context::default();
    hcl_ctx.declare_var("event", hcl_eval::json_to_hcl(payload.clone()));
    hcl_ctx.declare_var(
        "headers",
        hcl_eval::json_to_hcl(
            serde_json::to_value(
                headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
                    .collect::<HashMap<_, _>>(),
            )
            .unwrap(),
        ),
    );

    for rule in rules {
        // 3a. Check event type pattern (simple regex for now)
        let re = regex::Regex::new(&rule.event_type_pattern).map_err(|e| {
            tracing::error!(
                "Invalid regex pattern '{}': {:?}",
                rule.event_type_pattern,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        if !re.is_match(&event_type) {
            continue;
        }

        // 3b. Evaluate condition expression
        if let Some(cond) = &rule.condition_expr {
            match hcl_eval::evaluate_raw_expr(cond, &hcl_ctx) {
                Ok(Value::Bool(true)) => {}
                Ok(_) => continue,
                Err(e) => {
                    tracing::error!("Rule '{}' condition evaluation failed: {:?}", rule.name, e);
                    continue;
                }
            }
        }

        // 3c. Map inputs
        let mut inputs = serde_json::Map::new();
        for (name, expr) in rule.get_input_mappings() {
            match hcl_eval::evaluate_raw_expr(&expr, &hcl_ctx) {
                Ok(val) => {
                    inputs.insert(name, val);
                }
                Err(e) => {
                    tracing::error!(
                        "Rule '{}' input mapping for '{}' failed: {:?}",
                        rule.name,
                        name,
                        e
                    );
                    continue;
                }
            }
        }

        // 4. Trigger Workflow
        let run_id = Uuid::new_v4();

        tracing::info!(run_id = %run_id, "Enqueuing webhook workflow: {}", rule.workflow_name);
        let fencing_token = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let mut tx = state
            .pool
            .begin()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        crate::db::insert_workflow_run(
            &mut tx,
            run_id,
            &rule.workflow_name,
            &format!("webhook:{}", webhook.name),
            &rule.repo_url,
            &rule.workflow_path,
            &rule.git_ref,
            RunStatus::Queued,
            fencing_token,
        )
        .await
        .map_err(|e| {
            tracing::error!(run_id = %run_id, "Failed to insert workflow run: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        crate::db::insert_run_context(
            &mut tx,
            run_id,
            "v1",
            serde_json::json!({}),
            "",
            &Value::Object(inputs),
        )
        .await
        .map_err(|e| {
            tracing::error!(run_id = %run_id, "Failed to insert run context: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        crate::db::insert_run_quotas(&mut tx, run_id, 10, "1", "4Gi", "10Gi", "1h")
            .await
            .map_err(|e| {
                tracing::error!(run_id = %run_id, "Failed to insert run quotas: {:?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        tx.commit()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let event = stormchaser_model::events::WorkflowQueuedEvent {
            run_id,
            event_type: "workflow_queued".to_string(),
            timestamp: chrono::Utc::now(),
            dsl: None,
            inputs: None,
            initiating_user: None,
        };

        stormchaser_model::nats::publish_cloudevent(
            &async_nats::jetstream::new(state.nats.clone()),
            "stormchaser.v1.run.queued",
            "stormchaser.v1.run.queued",
            "/stormchaser",
            serde_json::to_value(event).unwrap(),
            Some("1.0"),
            None,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        triggered_count += 1;
        tracing::info!(
            "Triggered workflow '{}' (run {}) from rule '{}'",
            rule.workflow_name,
            run_id,
            rule.name
        );
    }

    Ok(Json(serde_json::json!({
        "status": "ok",
        "event_type": event_type,
        "triggered_rules": triggered_count
    })))
}

use stormchaser_model::hcl_eval;

fn validate_github_signature(
    headers: &HeaderMap,
    body: &[u8],
    secret: Option<&str>,
) -> Result<(), StatusCode> {
    let secret = match secret {
        Some(s) => s,
        None => {
            tracing::debug!(
                "No secret token configured for webhook, skipping signature validation"
            );
            return Ok(());
        }
    };

    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            tracing::warn!("Missing X-Hub-Signature-256 header");
            StatusCode::UNAUTHORIZED
        })?;

    if !signature.starts_with("sha256=") {
        tracing::warn!("Invalid signature format: {}", signature);
        return Err(StatusCode::UNAUTHORIZED);
    }

    let signature_hex = &signature["sha256=".len()..];
    let signature_bytes = hex::decode(signature_hex).map_err(|e| {
        tracing::warn!("Failed to decode signature hex: {:?}", e);
        StatusCode::UNAUTHORIZED
    })?;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|e| {
        tracing::error!("Failed to initialize HMAC: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    mac.update(body);

    if let Err(e) = mac.verify_slice(&signature_bytes) {
        tracing::warn!("HMAC signature verification failed: {:?}", e);
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(())
}
