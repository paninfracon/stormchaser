use crate::{AppState, JWT_SECRET};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use stormchaser_model::step::StepStatus;

#[derive(serde::Deserialize, serde::Serialize)]
struct ApprovalLinkPayload {
    run_id: Uuid,
    step_id: Uuid,
    action: String,
    #[serde(default)]
    inputs: serde_json::Value,
}

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
    let step =
        crate::db::get_step_instance_for_approval(&state.pool, payload.step_id, payload.run_id)
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
    let _ = crate::db::insert_approval_registry(
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
        "stormchaser.step.completed"
    } else {
        "stormchaser.step.failed"
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

pub async fn approve_step(
    State(state): State<AppState>,
    crate::auth::AuthClaims(claims): crate::auth::AuthClaims,
    Path((run_id, step_id)): Path<(Uuid, Uuid)>,
    Json(inputs): Json<serde_json::Value>,
) -> impl IntoResponse {
    // 1. Verify step exists and is WaitingForEvent
    let step = crate::db::get_step_instance_for_approval(&state.pool, step_id, run_id)
        .await
        .unwrap_or(None);

    let step = match step {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "Step not found").into_response(),
    };

    if step.status != StepStatus::WaitingForEvent {
        return (StatusCode::BAD_REQUEST, "Step is not waiting for approval").into_response();
    }

    // 2. Insert into approval_registry
    let _ = crate::db::insert_approval_registry(
        &state.pool,
        Uuid::new_v4(),
        step_id,
        &claims.sub,
        "approved",
        &inputs,
    )
    .await;

    // 3. Publish to NATS simulating step completion
    let payload = json!({
        "run_id": run_id.to_string(),
        "step_id": step_id.to_string(),
        "exit_code": 0,
        "outputs": inputs,
    });

    match state
        .nats
        .publish("stormchaser.step.completed", payload.to_string().into())
        .await
    {
        Ok(_) => (StatusCode::OK, "Approved").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}

pub async fn reject_step(
    State(state): State<AppState>,
    Path((run_id, step_id)): Path<(Uuid, Uuid)>,
) -> impl IntoResponse {
    let step = crate::db::get_step_instance_for_approval(&state.pool, step_id, run_id)
        .await
        .unwrap_or(None);

    let step = match step {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "Step not found").into_response(),
    };

    if step.status != StepStatus::WaitingForEvent {
        return (StatusCode::BAD_REQUEST, "Step is not waiting for approval").into_response();
    }

    let _ = crate::db::insert_approval_registry(
        &state.pool,
        Uuid::new_v4(),
        step_id,
        "system",
        "rejected",
        &json!({}),
    )
    .await;

    let payload = json!({
        "run_id": run_id.to_string(),
        "step_id": step_id.to_string(),
        "exit_code": 1,
        "error": "Rejected by human",
    });

    match state
        .nats
        .publish("stormchaser.step.failed", payload.to_string().into())
        .await
    {
        Ok(_) => (StatusCode::OK, "Rejected").into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}

pub async fn correlate_event(
    State(state): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    // 1. Iterate over event_correlations, match payload against correlation_key
    // For simplicity, let's assume payload has exactly { "key": "...", "value": "..." }
    let key = payload.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let value = payload.get("value").and_then(|v| v.as_str()).unwrap_or("");

    let correlation = crate::db::get_event_correlation(&state.pool, key, value)
        .await
        .unwrap_or(None);

    let corr = match correlation {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, "No correlation matched").into_response(),
    };

    // 2. Publish to stormchaser.step.completed
    let nats_payload = json!({
        "run_id": corr.run_id.to_string(),
        "step_id": corr.step_instance_id.to_string(),
        "exit_code": 0,
        "outputs": payload,
    });

    match state
        .nats
        .publish(
            "stormchaser.step.completed",
            nats_payload.to_string().into(),
        )
        .await
    {
        Ok(_) => {
            // Delete correlation so it doesn't match again
            let _ = crate::db::delete_event_correlation(&state.pool, corr.id).await;
            (StatusCode::OK, "Event Correlated").into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}
