use crate::db::insert_approval_registry;
use crate::{AppState, JWT_SECRET};
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use async_nats::jetstream::new as new_jetstream;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use serde_json::Value;
use sha2::{Digest, Sha256};
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepCompletedEvent, StepEventType, StepFailedEvent,
};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::EventId;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

use super::{resolve_fencing_token_or_response, verify_step_for_approval};

#[derive(serde::Deserialize, serde::Serialize)]
struct ApprovalLinkPayload {
    run_id: RunId,
    step_id: StepInstanceId,
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
    let _step = match verify_step_for_approval(&state.pool, payload.run_id, payload.step_id).await {
        Ok(s) => s,
        Err(r) => return r,
    };

    let is_approve = payload.action.to_lowercase() == "approve";
    let status_str = if is_approve { "approved" } else { "rejected" };

    // 6. Insert into approval_registry
    let _ = insert_approval_registry(
        &state.pool,
        EventId::new_v4(),
        payload.step_id,
        "system-link",
        status_str,
        &payload.inputs,
    )
    .await;

    let fencing_token = match resolve_fencing_token_or_response(&state, payload.run_id).await {
        Ok(token) => token,
        Err(response) => return response,
    };

    let publish_result = if is_approve {
        let outputs = match serde_json::from_value(payload.inputs) {
            Ok(outputs) => Some(outputs),
            Err(error) => {
                tracing::warn!(
                    "Approval link inputs for run {} step {} could not be deserialized: {:?}",
                    payload.run_id,
                    payload.step_id,
                    error
                );
                return (StatusCode::BAD_REQUEST, "Invalid approval inputs payload")
                    .into_response();
            }
        };
        let completion_event = StepCompletedEvent {
            run_id: payload.run_id,
            step_id: payload.step_id,
            fencing_token,
            event_type: EventType::Step(StepEventType::Completed),
            runner_id: None,
            exit_code: Some(0),
            storage_hashes: None,
            artifacts: None,
            test_reports: None,
            outputs,
            timestamp: Utc::now(),
        };
        publish_cloudevent(
            &new_jetstream(state.nats.clone()),
            NatsSubject::StepCompleted(Some(stormchaser_model::nats::compute_shard_id(
                &payload.run_id,
            ))),
            EventType::Step(StepEventType::Completed),
            EventSource::Api,
            serde_json::to_value(completion_event)
                .expect("serializing StepCompletedEvent should not fail"),
            Some(SchemaVersion::new("1.0".to_string())),
            None,
        )
        .await
    } else {
        let failure_event = StepFailedEvent {
            run_id: payload.run_id,
            step_id: payload.step_id,
            fencing_token,
            event_type: EventType::Step(StepEventType::Failed),
            error: "Rejected by human via link".to_string(),
            exit_code: Some(1),
            runner_id: None,
            storage_hashes: None,
            artifacts: None,
            test_reports: None,
            outputs: None,
            timestamp: Utc::now(),
        };
        publish_cloudevent(
            &new_jetstream(state.nats.clone()),
            NatsSubject::StepFailed(Some(stormchaser_model::nats::compute_shard_id(
                &payload.run_id,
            ))),
            EventType::Step(StepEventType::Failed),
            EventSource::Api,
            serde_json::to_value(failure_event)
                .expect("serializing StepFailedEvent should not fail"),
            Some(SchemaVersion::new("1.0".to_string())),
            None,
        )
        .await
    };

    match publish_result {
        Ok(_) => (StatusCode::OK, format!("Successfully {}", status_str)).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to publish").into_response(),
    }
}
