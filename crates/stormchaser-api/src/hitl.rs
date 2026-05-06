use crate::{AppState, JWT_SECRET};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::auth::AuthClaims;
use crate::db::{
    delete_event_correlation, get_event_correlation, get_run_outputs_for_opa,
    get_step_instance_for_approval, get_workflow_context_for_opa, insert_approval_registry,
};
use async_nats::jetstream::new as new_jetstream;
use chrono::Utc;
use stormchaser_model::auth::ApprovalOpaContext;
use stormchaser_model::dsl::{Step, Workflow};
use stormchaser_model::events::{StepCompletedEvent, StepFailedEvent};
use stormchaser_model::nats::publish_cloudevent;
use stormchaser_model::step::StepStatus;

#[derive(serde::Deserialize, serde::Serialize)]
struct ApprovalLinkPayload {
    run_id: Uuid,
    step_id: Uuid,
    action: String,
    #[serde(default)]
    inputs: Value,
}

/// Approves a step via an encrypted link.
#[utoipa::path(
    get,
    path = "/api/v1/approve-link/{token}",
    params(
        ("token" = String, Path, description = "Encrypted approval token")
    ),
    responses(
        (status = 200, description = "Step approved or rejected successfully"),
        (status = 400, description = "Invalid token or step state"),
        (status = 404, description = "Step not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "hitl"
)]
/// Approve step link.
pub async fn approve_step_link(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    // 1. Derive key from JWT_SECRET
    let mut hasher = Sha256::new();
    hasher.update(JWT_SECRET);
    let key_bytes = hasher.finalize();
    let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    // 2. Decode base64
    let decoded = match URL_SAFE_NO_PAD.decode(token) {
        Ok(d) => d,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid token encoding").into_response(),
    };

    if decoded.len() < 12 {
        return (StatusCode::BAD_REQUEST, "Token too short").into_response();
    }

    // 3. Decrypt
    let (nonce_bytes, ciphertext) = decoded.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = match cipher.decrypt(nonce, ciphertext) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                "Invalid token signature or ciphertext",
            )
                .into_response()
        }
    };

    // 4. Parse payload
    let payload: ApprovalLinkPayload = match serde_json::from_slice(&plaintext) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, "Invalid token payload").into_response(),
    };

    // 5. Verify step exists and is WaitingForEvent
    let step = get_step_instance_for_approval(&state.pool, payload.step_id, payload.run_id)
        .await
        .unwrap_or(None);

    let step = match step {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "Step not found").into_response(),
    };

    if step.status != StepStatus::WaitingForEvent {
        return (StatusCode::BAD_REQUEST, "Step is not waiting for approval").into_response();
    }

    let is_approve = payload.action.to_lowercase() == "approve";
    let status_str = if is_approve { "approved" } else { "rejected" };

    // 6. Insert into approval_registry
    let _ = insert_approval_registry(
        &state.pool,
        Uuid::new_v4(),
        payload.step_id,
        "system-link",
        status_str,
        &payload.inputs,
    )
    .await;

    // 7. Publish to NATS simulating step completion/failure
    let nats_payload = if is_approve {
        json!({
            "run_id": payload.run_id.to_string(),
            "step_id": payload.step_id.to_string(),
            "exit_code": 0,
            "outputs": payload.inputs,
        })
    } else {
        json!({
            "run_id": payload.run_id.to_string(),
            "step_id": payload.step_id.to_string(),
            "exit_code": 1,
            "error": "Rejected by human via link",
        })
    };

    let subject = if is_approve {
        "stormchaser.v1.step.completed"
    } else {
        "stormchaser.v1.step.failed"
    };

    match state
        .nats
        .publish(subject, nats_payload.to_string().into())
        .await
    {
        Ok(_) => (StatusCode::OK, format!("Successfully {}", status_str)).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}

async fn check_approval_opa(
    state: &AppState,
    run_id: Uuid,
    step_name: &str,
    token: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if !state.opa.is_configured() {
        return Ok(());
    }

    let context_row = match get_workflow_context_for_opa(&state.pool, run_id).await {
        Ok(context_row) => context_row,
        Err(err) => {
            eprintln!(
                "Failed to load workflow context for approval OPA evaluation for run {}: {:?}",
                run_id, err
            );
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to load workflow context for approval policy evaluation".to_string(),
            ));
        }
    };

    if let Some(context_data) = context_row {
        let mut step_ast = json!({});
        if let Ok(workflow) = serde_json::from_value::<Workflow>(context_data.workflow_definition) {
            if let Some(s) = find_step(&workflow.steps, step_name) {
                step_ast = serde_json::to_value(s).unwrap_or(json!({}));
            }
        }

        let run_outputs_map = match get_run_outputs_for_opa(&state.pool, run_id).await {
            Ok(map) => map,
            Err(err) => {
                tracing::error!(
                    "Failed to load run outputs for approval OPA evaluation for run {}: {:?}",
                    run_id,
                    err
                );
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to load run outputs for approval policy evaluation".to_string(),
                ));
            }
        };

        let opa_context = ApprovalOpaContext {
            run_id,
            initiating_user: context_data.initiating_user,
            step_ast,
            inputs: context_data.run_inputs,
            run_outputs: Value::Object(run_outputs_map),
            token,
        };

        match state.opa.check_approval(opa_context).await {
            Ok(true) => Ok(()),
            Ok(false) => Err((
                StatusCode::FORBIDDEN,
                "Approval denied by OPA policy".to_string(),
            )),
            Err(_) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "OPA policy evaluation failed".to_string(),
            )),
        }
    } else {
        Ok(())
    }
}

// Recursively find the step AST
fn find_step(steps: &[Step], name: &str) -> Option<Step> {
    for s in steps {
        if s.name == name {
            return Some(s.clone());
        }
        if let Some(inner) = &s.steps {
            if let Some(found) = find_step(inner, name) {
                return Some(found);
            }
        }
    }
    None
}

/// Approves a step.
pub async fn approve_step(
    State(state): State<AppState>,
    AuthClaims(claims): AuthClaims,
    headers: HeaderMap,
    Path((run_id, step_id)): Path<(Uuid, Uuid)>,
    Json(inputs): Json<Value>,
) -> impl IntoResponse {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    // 1. Verify step exists and is WaitingForEvent
    let step = get_step_instance_for_approval(&state.pool, step_id, run_id)
        .await
        .unwrap_or(None);

    let step = match step {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "Step not found").into_response(),
    };

    if step.status != StepStatus::WaitingForEvent {
        return (StatusCode::BAD_REQUEST, "Step is not waiting for approval").into_response();
    }

    // 1.5 OPA ABAC Engine check
    if let Err((status, msg)) = check_approval_opa(&state, run_id, &step.step_name, token).await {
        return (status, msg).into_response();
    }

    // 2. Insert into approval_registry
    let _ = insert_approval_registry(
        &state.pool,
        Uuid::new_v4(),
        step_id,
        &claims.sub,
        "approved",
        &inputs,
    )
    .await;

    // 3. Publish to NATS simulating step completion
    let completion_event = StepCompletedEvent {
        run_id,
        step_id,
        event_type: "stormchaser.v1.step.completed".to_string(),
        runner_id: None,
        exit_code: Some(0),
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: serde_json::from_value(inputs).ok(),
        timestamp: Utc::now(),
    };

    match publish_cloudevent(
        &new_jetstream(state.nats.clone()),
        "stormchaser.v1.step.completed",
        "stormchaser.v1.step.completed",
        "/stormchaser/api",
        serde_json::to_value(completion_event).unwrap(),
        Some("1.0"),
        None,
    )
    .await
    {
        Ok(_) => (StatusCode::OK, "Approved").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}

/// Rejects a step.
pub async fn reject_step(
    State(state): State<AppState>,
    AuthClaims(claims): AuthClaims,
    headers: HeaderMap,
    Path((run_id, step_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let step = get_step_instance_for_approval(&state.pool, step_id, run_id)
        .await
        .unwrap_or(None);

    let step = match step {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "Step not found").into_response(),
    };

    if step.status != StepStatus::WaitingForEvent {
        return (StatusCode::BAD_REQUEST, "Step is not waiting for approval").into_response();
    }

    // 1.5 OPA ABAC Engine check
    if let Err((status, msg)) = check_approval_opa(&state, run_id, &step.step_name, token).await {
        return (status, msg).into_response();
    }

    let _ = insert_approval_registry(
        &state.pool,
        Uuid::new_v4(),
        step_id,
        &claims.sub,
        "rejected",
        &json!({}),
    )
    .await;

    let event = StepFailedEvent {
        run_id,
        step_id,
        event_type: "stormchaser.v1.step.failed".to_string(),
        error: "Rejected by human".to_string(),
        exit_code: Some(1),
        runner_id: None,
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: None,
        timestamp: Utc::now(),
    };

    match publish_cloudevent(
        &new_jetstream(state.nats.clone()),
        "stormchaser.v1.step.failed",
        "stormchaser.v1.step.failed",
        "/stormchaser/api",
        serde_json::to_value(event).unwrap(),
        Some("1.0"),
        None,
    )
    .await
    {
        Ok(_) => (StatusCode::OK, "Rejected").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}

/// Correlates an event.
pub async fn correlate_event(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    // 1. Iterate over event_correlations, match payload against correlation_key
    // For simplicity, let's assume payload has exactly { "key": "...", "value": "..." }
    let key = payload.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let value = payload.get("value").and_then(|v| v.as_str()).unwrap_or("");

    let correlation = get_event_correlation(&state.pool, key, value)
        .await
        .unwrap_or(None);

    let corr = match correlation {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, "No correlation matched").into_response(),
    };

    // 2. Publish to stormchaser.step.completed
    let completion_event = StepCompletedEvent {
        run_id: corr.run_id,
        step_id: corr.step_instance_id,
        event_type: "stormchaser.v1.step.completed".to_string(),
        runner_id: None,
        exit_code: Some(0),
        storage_hashes: None,
        artifacts: None,
        test_reports: None,
        outputs: serde_json::from_value(payload.clone()).ok(),
        timestamp: Utc::now(),
    };

    match publish_cloudevent(
        &new_jetstream(state.nats.clone()),
        "stormchaser.v1.step.completed",
        "stormchaser.v1.step.completed",
        "/stormchaser/api",
        serde_json::to_value(completion_event).unwrap(),
        Some("1.0"),
        None,
    )
    .await
    {
        Ok(_) => {
            // Delete correlation so it doesn't match again
            let _ = delete_event_correlation(&state.pool, corr.id).await;
            (StatusCode::OK, "Event Correlated").into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}
