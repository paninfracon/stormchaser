pub mod link;
use crate::AppState;

use axum::{
    extract::{Path, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
pub use link::*;
use serde_json::{json, Value};

use stormchaser_model::auth::ApprovalOpaContext;
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepCompletedEvent, StepEventType, StepFailedEvent,
};
use stormchaser_model::step::StepStatus;
use stormchaser_model::EventId;
use stormchaser_model::RunId;
use stormchaser_model::StepInstanceId;

use crate::auth::AuthClaims;
use crate::db::{
    delete_event_correlation, get_event_correlation, get_run_outputs_for_opa,
    get_step_instance_for_approval, get_workflow_context_for_opa, get_workflow_run_fencing_token,
    insert_approval_registry,
};
use async_nats::jetstream::new as new_jetstream;
use chrono::Utc;
use stormchaser_model::dsl::{Step, Workflow};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};

async fn check_approval_opa(
    state: &AppState,
    run_id: RunId,
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

async fn resolve_fencing_token(state: &AppState, run_id: RunId) -> Result<i64, StatusCode> {
    get_workflow_run_fencing_token(&state.pool, run_id)
        .await
        .map_err(|error| {
            tracing::error!(
                "Failed to load fencing token for run {} in HITL handler: {:?}",
                run_id,
                error
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)
}

pub(crate) async fn resolve_fencing_token_or_response(
    state: &AppState,
    run_id: RunId,
) -> Result<i64, axum::response::Response> {
    resolve_fencing_token(state, run_id)
        .await
        .map_err(|status| match status {
            StatusCode::NOT_FOUND => (StatusCode::NOT_FOUND, "Run not found").into_response(),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load run").into_response(),
        })
}

/// Approves a step.
pub async fn approve_step(
    State(state): State<AppState>,
    AuthClaims(claims): AuthClaims,
    headers: HeaderMap,
    Path((run_id, step_id)): Path<(RunId, StepInstanceId)>,
    Json(inputs): Json<Value>,
) -> impl IntoResponse {
    if let Err(r) = validate_approval_state(&state, run_id, step_id, &headers).await {
        return r;
    }

    // 2. Insert into approval_registry
    let _ = insert_approval_registry(
        &state.pool,
        EventId::new_v4(),
        step_id,
        &claims.sub,
        "approved",
        &inputs,
    )
    .await;

    let fencing_token = match resolve_fencing_token_or_response(&state, run_id).await {
        Ok(token) => token,
        Err(response) => return response,
    };

    // 3. Publish to NATS simulating step completion
    let completion_event = StepCompletedEvent {
        run_id,
        step_id,
        fencing_token,
        event_type: EventType::Step(StepEventType::Completed),
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
        NatsSubject::StepCompleted(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Step(StepEventType::Completed),
        EventSource::Api,
        serde_json::to_value(completion_event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
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
    Path((run_id, step_id)): Path<(RunId, StepInstanceId)>,
) -> impl IntoResponse {
    if let Err(r) = validate_approval_state(&state, run_id, step_id, &headers).await {
        return r;
    }

    let _ = insert_approval_registry(
        &state.pool,
        EventId::new_v4(),
        step_id,
        &claims.sub,
        "rejected",
        &json!({}),
    )
    .await;

    let fencing_token = match resolve_fencing_token_or_response(&state, run_id).await {
        Ok(token) => token,
        Err(response) => return response,
    };

    let event = StepFailedEvent {
        run_id,
        step_id,
        fencing_token,
        event_type: EventType::Step(StepEventType::Failed),
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
        NatsSubject::StepFailed(Some(stormchaser_model::nats::compute_shard_id(&run_id))),
        EventType::Step(StepEventType::Failed),
        EventSource::Api,
        serde_json::to_value(event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
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

    let fencing_token = match resolve_fencing_token_or_response(&state, corr.run_id).await {
        Ok(token) => token,
        Err(response) => return response,
    };

    // 2. Publish to stormchaser.step.completed
    let completion_event = StepCompletedEvent {
        run_id: corr.run_id,
        step_id: corr.step_instance_id,
        fencing_token,
        event_type: EventType::Step(StepEventType::Completed),
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
        NatsSubject::StepCompleted(Some(stormchaser_model::nats::compute_shard_id(
            &corr.run_id,
        ))),
        EventType::Step(StepEventType::Completed),
        EventSource::Api,
        serde_json::to_value(completion_event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
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

pub(crate) async fn verify_step_for_approval(
    pool: &sqlx::PgPool,
    run_id: RunId,
    step_id: StepInstanceId,
) -> Result<stormchaser_model::step::StepInstance, axum::response::Response> {
    let step = match get_step_instance_for_approval(pool, step_id, run_id).await {
        Ok(Some(s)) => s,
        _ => return Err((StatusCode::NOT_FOUND, "Step not found").into_response()),
    };

    if step.status != StepStatus::WaitingForEvent {
        return Err((StatusCode::BAD_REQUEST, "Step is not waiting for approval").into_response());
    }

    Ok(step)
}

async fn validate_approval_state(
    state: &AppState,
    run_id: RunId,
    step_id: StepInstanceId,
    headers: &HeaderMap,
) -> Result<(), axum::response::Response> {
    let step = verify_step_for_approval(&state.pool, run_id, step_id).await?;
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    if let Err((status, msg)) = check_approval_opa(state, run_id, &step.step_name, token).await {
        return Err((status, msg).into_response());
    }

    Ok(())
}
