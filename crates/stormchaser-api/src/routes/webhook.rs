use super::{CreateWebhookRequest, UpdateWebhookRequest};
use crate::db;
use crate::{AppState, AuthClaims};
use async_nats::jetstream;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::collections::HashMap;
use stormchaser_model::event_rules::WebhookConfig;
use stormchaser_model::events::WorkflowQueuedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::WebhookId;

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
    let id = stormchaser_model::WebhookId::new_v4();
    db::insert_webhook(
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
    let webhooks = db::list_webhooks(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(webhooks))
}

/// Gets a webhook.
#[utoipa::path(
    get,
    path = "/api/v1/webhooks/{id}",
    params(("id" = stormchaser_model::WebhookId, Path, description="Webhook ID")),
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
    Path(id): Path<WebhookId>,
) -> Result<impl IntoResponse, StatusCode> {
    let webhook = db::get_webhook(&state.pool, id)
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
    params(("id" = stormchaser_model::WebhookId, Path, description="Webhook ID")),
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
    Path(id): Path<WebhookId>,
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

    db::update_webhook(
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
    params(("id" = stormchaser_model::WebhookId, Path, description="Webhook ID")),
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
    Path(id): Path<WebhookId>,
) -> Result<impl IntoResponse, StatusCode> {
    db::delete_webhook(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/webhooks/{id}",
    params(
        ("id" = stormchaser_model::WebhookId, Path, description = "Webhook ID")
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
    Path(webhook_id): Path<WebhookId>,
    headers: HeaderMap,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    // 1. Fetch WebhookConfig
    let webhook: WebhookConfig = db::get_active_webhook(&state.pool, webhook_id)
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
    let rules = db::get_active_event_rules_by_webhook(&state.pool, webhook_id)
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
        let run_id = stormchaser_model::RunId::new_v4();

        tracing::info!(run_id = %run_id, "Enqueuing webhook workflow: {}", rule.workflow_name);
        let fencing_token = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let mut tx = state
            .pool
            .begin()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        db::insert_workflow_run(
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

        let inputs_value = serde_json::Value::Object(inputs);

        db::insert_run_context(
            &mut *tx,
            run_id,
            "v1",
            serde_json::json!({}),
            "",
            &inputs_value,
            serde_json::json!({}),
            vec![],
        )
        .await
        .map_err(|e| {
            tracing::error!(run_id = %run_id, "Failed to insert run context: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        db::insert_run_quotas(&mut tx, run_id, 10, "1", "4Gi", "10Gi", "1h")
            .await
            .map_err(|e| {
                tracing::error!(run_id = %run_id, "Failed to insert run quotas: {:?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        tx.commit()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let event = WorkflowQueuedEvent {
            run_id,
            event_type: EventType::Workflow(WorkflowEventType::Queued),
            timestamp: chrono::Utc::now(),
            status: stormchaser_model::workflow::RunStatus::Queued,
            dsl: None,
            inputs: None,
            initiating_user: None,
            sops_file: None,
            sops_role_arn: None,
        };

        publish_cloudevent(
            &jetstream::new(state.nats.clone()),
            NatsSubject::RunQueued(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
            EventType::Workflow(WorkflowEventType::Queued),
            EventSource::System,
            serde_json::to_value(event).unwrap(),
            Some(SchemaVersion::new("1.0".to_string())),
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
