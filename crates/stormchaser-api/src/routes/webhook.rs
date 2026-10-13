use super::{CreateWebhookRequest, UpdateWebhookRequest};
use crate::authz::{self, Caller, Kind};
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
use hmac::{Hmac, KeyInit, Mac};
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
    caller: Caller,
    State(state): State<AppState>,
    Json(payload): Json<CreateWebhookRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    // A webhook URL is public; the secret is the only thing that says who is calling
    if !payload
        .secret_token
        .as_deref()
        .is_some_and(|secret| !secret.is_empty())
    {
        tracing::warn!(
            "Refusing to create webhook '{}' without a secret",
            payload.name
        );
        return Err(StatusCode::BAD_REQUEST);
    }
    if !matches!(
        payload.source_type.as_str(),
        "github" | "generic" | "signed"
    ) {
        return Err(StatusCode::BAD_REQUEST);
    }

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
    authz::record_owner(&state.pool, Kind::Webhook, id.into_inner(), &caller).await?;

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
    caller: Caller,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let webhooks = db::list_webhooks(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let webhooks = authz::filter_readable(&state, &caller, Kind::Webhook, webhooks, |w| {
        w.id.into_inner()
    })
    .await?;
    let webhooks: Vec<_> = webhooks.into_iter().map(crate::redact::webhook).collect();
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

    Ok(Json(crate::redact::webhook(webhook)))
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
        // The mask from a previous response means "keep the stored secret".
        Some(s) if s == crate::redact::MASK => None,
        // A secret can be replaced but not removed
        Some(s) if s.is_empty() => return Err(StatusCode::BAD_REQUEST),
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

    // 2. Validate Source/Signature, before anything in the body is trusted or parsed
    let secret = webhook.secret_token.as_deref();
    let delivery_key = match webhook.source_type.as_str() {
        "github" => {
            validate_github_signature(&headers, &body, secret)?;
            // The delivery id GitHub sends is not covered by its signature, so whoever
            // replays a delivery can change it. The body is what was signed.
            Some(format!(
                "sha256:{}",
                hex::encode(<Sha256 as sha2::Digest>::digest(&body))
            ))
        }
        "signed" => {
            let id = validate_signed_delivery(&headers, &body, secret, Utc::now().timestamp())?;
            Some(format!("id:{}", id))
        }
        "generic" => {
            // A bearer token proves nothing about one delivery in particular: whoever
            // holds it can send anything, so there is nothing to tell a replay by.
            validate_generic_token(&headers, secret)?;
            None
        }
        _ => return Err(StatusCode::NOT_IMPLEMENTED),
    };

    let payload: Value = serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let event_type = match webhook.source_type.as_str() {
        "github" => headers
            .get("X-GitHub-Event")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown")
            .to_string(),
        // Signed senders conventionally name the event in the payload's `type`
        "signed" => payload
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("generic")
            .to_string(),
        _ => "generic".to_string(),
    };

    // 2b. A delivery already acted on is acknowledged and otherwise ignored
    if let Some(key) = &delivery_key {
        let first_time = db::record_delivery(&state.pool, webhook_id, key, dedupe_hours())
            .await
            .map_err(|e| {
                tracing::error!("Failed to record webhook delivery: {:?}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
        if !first_time {
            tracing::info!(
                "Ignoring repeated delivery to webhook '{}' ({})",
                webhook.name,
                webhook.id
            );
            return Ok(Json(serde_json::json!({
                "status": "duplicate",
                "event_type": event_type,
                "triggered_rules": 0
            })));
        }
    }

    let result = run_rules(&state, &webhook, &headers, &payload, &event_type).await;
    if result.is_err() {
        // Nothing was done with it, so the sender's retry must be allowed through
        if let Some(key) = &delivery_key {
            if let Err(e) = db::forget_delivery(&state.pool, webhook_id, key).await {
                tracing::error!("Failed to forget an unprocessed webhook delivery: {:?}", e);
            }
        }
    }
    let triggered_count = result?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "event_type": event_type,
        "triggered_rules": triggered_count
    })))
}

/// Hours for which an accepted delivery is remembered and a repeat of it ignored.
fn dedupe_hours() -> i32 {
    std::env::var("STORMCHASER_WEBHOOK_DEDUPE_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|hours| *hours > 0)
        .unwrap_or(24)
}

/// Runs the webhook's rules against a verified delivery and returns how many fired.
async fn run_rules(
    state: &AppState,
    webhook: &WebhookConfig,
    headers: &HeaderMap,
    payload: &Value,
    event_type: &str,
) -> Result<usize, StatusCode> {
    let webhook_id = webhook.id;
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
        hcl_eval::json_to_hcl(serde_json::to_value(headers_for_rules(headers)).unwrap()),
    );

    for rule in rules {
        match process_rule(state, webhook, &rule, event_type, &hcl_ctx).await {
            Ok(true) => triggered_count += 1,
            Ok(false) => continue,
            Err(e) => return Err(e),
        }
    }

    Ok(triggered_count)
}

use stormchaser_model::hcl_eval;

/// A webhook without a secret accepts nothing: its URL is not a credential, and a
/// delivery starts workflows.
fn required_secret(secret: Option<&str>) -> Result<&str, StatusCode> {
    match secret {
        Some(secret) if !secret.is_empty() => Ok(secret),
        _ => {
            tracing::warn!("Refusing delivery to a webhook that has no secret configured");
            Err(StatusCode::FORBIDDEN)
        }
    }
}

/// Headers a rule may read. The ones that authenticate the delivery are left out, so a
/// rule cannot copy the webhook's secret or signature into a run's inputs.
fn headers_for_rules(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "authorization" | "cookie" | "x-hub-signature" | "x-hub-signature-256"
            )
        })
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
        .collect()
}

/// Whether an event type matches a rule's pattern. The pattern must match the whole
/// event type: unanchored, `push` would also match `repository_push_ruleset`.
fn event_type_matches(pattern: &str, event_type: &str) -> Result<bool, regex::Error> {
    Ok(regex::Regex::new(&format!("^(?:{})$", pattern))?.is_match(event_type))
}

fn validate_github_signature(
    headers: &HeaderMap,
    body: &[u8],
    secret: Option<&str>,
) -> Result<(), StatusCode> {
    let secret = required_secret(secret)?;

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

/// Furthest a signed delivery's timestamp may be from the API's clock, in seconds.
const SIGNED_TOLERANCE_SECS: i64 = 300;

/// Verifies a delivery signed the Standard Webhooks way and returns its message id.
///
/// The sender puts a message id in `webhook-id`, the time in `webhook-timestamp`
/// (seconds since the epoch) and, in `webhook-signature`, one or more space-separated
/// `v1,<base64>` values, each the HMAC-SHA256 of `<id>.<timestamp>.<body>`. Because the
/// id and the time are signed, a captured delivery cannot be relabelled, and it stops
/// being accepted once its timestamp is older than [`SIGNED_TOLERANCE_SECS`].
///
/// A secret of the form `whsec_<base64>` is used as the decoded bytes; any other secret
/// is used as it is written.
fn validate_signed_delivery(
    headers: &HeaderMap,
    body: &[u8],
    secret: Option<&str>,
    now: i64,
) -> Result<String, StatusCode> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let secret = required_secret(secret)?;
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|h| h.to_str().ok())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                tracing::warn!("Missing {} header for signed webhook", name);
                StatusCode::UNAUTHORIZED
            })
    };
    let id = header("webhook-id")?;
    let timestamp = header("webhook-timestamp")?;
    let signatures = header("webhook-signature")?;

    if id.len() > 256 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let sent_at: i64 = timestamp.parse().map_err(|_| StatusCode::UNAUTHORIZED)?;
    if (now - sent_at).abs() > SIGNED_TOLERANCE_SECS {
        tracing::warn!("Signed webhook delivery is outside the accepted time window");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let key = secret
        .strip_prefix("whsec_")
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .unwrap_or_else(|| secret.as_bytes().to_vec());
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|e| {
        tracing::error!("Failed to initialize HMAC: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    mac.update(id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);

    // Several signatures are sent while a secret is being rotated; one must match
    let verified = signatures
        .split_whitespace()
        .filter_map(|entry| entry.strip_prefix("v1,"))
        .filter_map(|encoded| STANDARD.decode(encoded).ok())
        .any(|candidate| mac.clone().verify_slice(&candidate).is_ok());
    if !verified {
        tracing::warn!("Signed webhook delivery has no valid signature");
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(id.to_string())
}

fn validate_generic_token(headers: &HeaderMap, secret: Option<&str>) -> Result<(), StatusCode> {
    let secret = required_secret(secret)?;

    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            tracing::warn!("Missing Authorization header for generic webhook");
            StatusCode::UNAUTHORIZED
        })?;

    let expected = format!("Bearer {}", secret);

    use sha2::{Digest, Sha256};
    let mut hasher1 = Sha256::new();
    hasher1.update(auth_header.as_bytes());
    let hash1 = hasher1.finalize();

    let mut hasher2 = Sha256::new();
    hasher2.update(expected.as_bytes());
    let hash2 = hasher2.finalize();

    if hash1 != hash2 {
        tracing::warn!("Invalid Authorization token for generic webhook");
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(())
}

async fn process_rule(
    state: &AppState,
    webhook: &WebhookConfig,
    rule: &stormchaser_model::event_rules::EventRule,
    event_type: &str,
    hcl_ctx: &hcl::eval::Context<'_>,
) -> Result<bool, StatusCode> {
    // 3a. Check event type pattern (simple regex for now)
    let matches = event_type_matches(&rule.event_type_pattern, event_type).map_err(|e| {
        tracing::error!(
            "Invalid regex pattern '{}': {:?}",
            rule.event_type_pattern,
            e
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    if !matches {
        return Ok(false);
    }

    // 3b. Evaluate condition expression
    if let Some(cond) = &rule.condition_expr {
        match hcl_eval::evaluate_raw_expr(cond, hcl_ctx) {
            Ok(Value::Bool(true)) => {}
            Ok(_) => return Ok(false),
            Err(e) => {
                tracing::error!("Rule '{}' condition evaluation failed: {:?}", rule.name, e);
                return Ok(false);
            }
        }
    }

    // 3c. Map inputs
    let mut inputs = serde_json::Map::new();
    for (name, expr) in rule.get_input_mappings() {
        match hcl_eval::evaluate_raw_expr(&expr, hcl_ctx) {
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
                return Ok(false);
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

    // The run acts as whoever created the rule; rules that predate ownership keep the
    // old label, which policy treats as "no owner".
    let initiating_user = authz::owner_of(&state.pool, Kind::EventRule, rule.id.into_inner())
        .await
        .unwrap_or_else(|| format!("webhook:{}", webhook.name));

    db::insert_workflow_run(
        &mut tx,
        run_id,
        &rule.workflow_name,
        &initiating_user,
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

    tracing::info!(
        "Triggered workflow '{}' (run {}) from rule '{}'",
        rule.workflow_name,
        run_id,
        rule.name
    );

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_pattern_must_match_the_whole_event_type() {
        assert!(event_type_matches("push", "push").unwrap());
        assert!(!event_type_matches("push", "repository_push_ruleset").unwrap());
        assert!(event_type_matches("pull_request.*", "pull_request_review").unwrap());
        assert!(event_type_matches("push|release", "release").unwrap());
        assert!(!event_type_matches("push|release", "prerelease").unwrap());
        assert!(event_type_matches(".*", "anything").unwrap());
        assert!(event_type_matches("(", "push").is_err());
    }

    #[test]
    fn rules_do_not_see_the_headers_that_authenticate_a_delivery() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer the-secret".parse().unwrap());
        headers.insert("x-hub-signature-256", "sha256=abc".parse().unwrap());
        headers.insert("x-github-event", "push".parse().unwrap());
        let visible = headers_for_rules(&headers);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible["x-github-event"], "push");
    }

    fn signed_headers(secret: &[u8], id: &str, timestamp: i64, body: &[u8]) -> HeaderMap {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(format!("{id}.{timestamp}.").as_bytes());
        mac.update(body);
        let signature = STANDARD.encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("webhook-id", id.parse().unwrap());
        headers.insert("webhook-timestamp", timestamp.to_string().parse().unwrap());
        headers.insert(
            "webhook-signature",
            format!("v1,{signature}").parse().unwrap(),
        );
        headers
    }

    #[test]
    fn a_signed_delivery_matches_the_standard_webhooks_reference_vector() {
        // From the Standard Webhooks specification. The secret is the published example
        // one, not a credential, hence the scanner exemption.
        let secret = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"; // gitleaks:allow
        let body = br#"{"test": 2432232314}"#;
        let mut headers = HeaderMap::new();
        headers.insert(
            "webhook-id",
            "msg_p5jXN8AQM9LWM0D4loKWxJek".parse().unwrap(),
        );
        headers.insert("webhook-timestamp", "1614265330".parse().unwrap());
        headers.insert(
            "webhook-signature",
            "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE="
                .parse()
                .unwrap(),
        );
        assert_eq!(
            validate_signed_delivery(&headers, body, Some(secret), 1614265330),
            Ok("msg_p5jXN8AQM9LWM0D4loKWxJek".to_string())
        );
    }

    #[test]
    fn a_signed_delivery_cannot_be_altered_relabelled_or_kept_for_later() {
        let secret = "plain-shared-secret";
        let body = br#"{"type":"deploy"}"#;
        let now = 1_800_000_000;
        let headers = signed_headers(secret.as_bytes(), "msg_1", now, body);
        let check = |headers: &HeaderMap, body: &[u8], now: i64| {
            validate_signed_delivery(headers, body, Some(secret), now)
        };
        assert_eq!(check(&headers, body, now), Ok("msg_1".to_string()));

        // A different body, id or timestamp no longer matches the signature
        assert!(check(&headers, br#"{"type":"destroy"}"#, now).is_err());
        let mut relabelled = headers.clone();
        relabelled.insert("webhook-id", "msg_2".parse().unwrap());
        assert!(check(&relabelled, body, now).is_err());
        let mut redated = headers.clone();
        redated.insert("webhook-timestamp", (now + 60).to_string().parse().unwrap());
        assert!(check(&redated, body, now + 60).is_err());

        // The genuine delivery stops being accepted once it is old, or if it is from
        // the future
        assert!(check(&headers, body, now + SIGNED_TOLERANCE_SECS).is_ok());
        assert!(check(&headers, body, now + SIGNED_TOLERANCE_SECS + 1).is_err());
        assert!(check(&headers, body, now - SIGNED_TOLERANCE_SECS - 1).is_err());

        // Another secret, a missing header, or no secret at all
        assert!(validate_signed_delivery(&headers, body, Some("other"), now).is_err());
        let mut unsigned = headers.clone();
        unsigned.remove("webhook-signature");
        assert!(check(&unsigned, body, now).is_err());
        assert_eq!(
            validate_signed_delivery(&headers, body, None, now),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn any_one_of_several_signatures_is_enough() {
        let body = b"{}";
        let now = 1_800_000_000;
        let mut headers = signed_headers(b"new-secret", "msg_1", now, body);
        let current = headers["webhook-signature"].to_str().unwrap().to_string();
        headers.insert(
            "webhook-signature",
            format!("v1,AAAA v2,ignored {current}").parse().unwrap(),
        );
        assert!(validate_signed_delivery(&headers, body, Some("new-secret"), now).is_ok());
    }

    #[test]
    fn a_missing_or_empty_secret_refuses_the_delivery() {
        assert_eq!(required_secret(None), Err(StatusCode::FORBIDDEN));
        assert_eq!(required_secret(Some("")), Err(StatusCode::FORBIDDEN));
        assert_eq!(required_secret(Some("s3cret")), Ok("s3cret"));
        assert_eq!(
            validate_generic_token(&HeaderMap::new(), None),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            validate_github_signature(&HeaderMap::new(), b"{}", None),
            Err(StatusCode::FORBIDDEN)
        );
    }
}
