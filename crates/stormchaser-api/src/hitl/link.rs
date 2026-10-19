use crate::auth::Claims;
use crate::db::insert_approval_registry;
use crate::AppState;
use async_nats::jetstream::new as new_jetstream;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
};
use chrono::Utc;
use stormchaser_core::hitl::{approval_link_key, open_approval_link, LinkError};
use stormchaser_model::events::{
    EventSource, EventType, SchemaVersion, StepCompletedEvent, StepEventType, StepFailedEvent,
};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::EventId;

use super::{check_approval_opa, resolve_fencing_token_or_response, verify_step_for_approval};

/// Why a link cannot be used, as an HTTP answer.
fn refuse(error: LinkError) -> axum::response::Response {
    match error {
        LinkError::Invalid => (StatusCode::BAD_REQUEST, "Invalid approval link").into_response(),
        LinkError::Expired => (StatusCode::GONE, "This approval link has expired").into_response(),
    }
}

fn links_disabled() -> axum::response::Response {
    (StatusCode::NOT_FOUND, "Approval links are not enabled").into_response()
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Shows what an approval link will do and asks for confirmation.
///
/// Opening the link changes nothing. Mail scanners, link previews and browser prefetch
/// all fetch URLs on their own, so the decision is only recorded when the form on this
/// page is submitted.
#[utoipa::path(
    get,
    path = "/api/v1/approve-link/{token}",
    params(
        ("token" = String, Path, description = "Sealed approval token")
    ),
    responses(
        (status = 200, description = "Confirmation page"),
        (status = 400, description = "Invalid token or step state"),
        (status = 404, description = "Step not found, or approval links are not enabled"),
        (status = 410, description = "The link has expired")
    ),
    tag = "hitl"
)]
pub async fn approve_step_link_page(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    approval_link_page(&state, &token, approval_link_key().as_deref()).await
}

/// [`approve_step_link_page`] with the link key passed in.
pub async fn approval_link_page(
    state: &AppState,
    token: &str,
    key: Option<&str>,
) -> axum::response::Response {
    let Some(key) = key else {
        return links_disabled();
    };
    let link = match open_approval_link(token, key, Utc::now()) {
        Ok(link) => link,
        Err(error) => return refuse(error),
    };
    let step = match verify_step_for_approval(&state.pool, link.run_id, link.step_id).await {
        Ok(step) => step,
        Err(response) => return *response,
    };

    let verb = if link.action == "approve" {
        "Approve"
    } else {
        "Reject"
    };
    let page = format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>{verb} step</title></head>
<body>
<h1>{verb} this step?</h1>
<p>Step <strong>{step}</strong> of run <code>{run}</code> is waiting for a decision.</p>
<form method="post">
<button type="submit">{verb}</button>
</form>
<p>If you did not mean to open this page, close it. Nothing has been recorded.</p>
</body>
</html>
"#,
        verb = verb,
        step = escape_html(&step.step_name),
        run = link.run_id,
    );

    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; form-action 'self'; frame-ancestors 'none'",
            ),
        ],
        page,
    )
        .into_response()
}

/// Approves or rejects a step via a sealed link, once the confirmation page is submitted.
#[utoipa::path(
    post,
    path = "/api/v1/approve-link/{token}",
    params(
        ("token" = String, Path, description = "Sealed approval token")
    ),
    responses(
        (status = 200, description = "Step approved or rejected successfully"),
        (status = 400, description = "Invalid token or step state"),
        (status = 403, description = "Denied by policy"),
        (status = 404, description = "Step not found, or approval links are not enabled"),
        (status = 410, description = "The link has expired"),
        (status = 500, description = "Internal server error")
    ),
    tag = "hitl"
)]
/// Approve step link.
pub async fn approve_step_link(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    use_approval_link(&state, &token, approval_link_key().as_deref()).await
}

/// [`approve_step_link`] with the link key passed in.
pub async fn use_approval_link(
    state: &AppState,
    token: &str,
    key: Option<&str>,
) -> axum::response::Response {
    let Some(key) = key else {
        return links_disabled();
    };
    let payload = match open_approval_link(token, key, Utc::now()) {
        Ok(link) => link,
        Err(error) => return refuse(error),
    };

    // Verify step exists and is WaitingForEvent
    let step = match verify_step_for_approval(&state.pool, payload.run_id, payload.step_id).await {
        Ok(s) => s,
        Err(r) => return *r,
    };

    // The same policy question as an approval made while logged in. The approver is
    // whoever the link was sent to, when that is one person.
    let approver = match payload.approvers.as_slice() {
        [only] => Some(Claims {
            sub: only.clone(),
            email: Some(only.clone()),
            exp: 0,
            groups: Vec::new(),
        }),
        _ => None,
    };
    if let Err((status, message)) =
        check_approval_opa(state, payload.run_id, &step.step_name, approver.as_ref()).await
    {
        return (status, message).into_response();
    }
    let recorded_as = if payload.approvers.is_empty() {
        "system-link".to_string()
    } else {
        format!("link:{}", payload.approvers.join(","))
    };

    let is_approve = payload.action.to_lowercase() == "approve";
    let status_str = if is_approve { "approved" } else { "rejected" };

    // 6. Insert into approval_registry
    let _ = insert_approval_registry(
        &state.pool,
        EventId::new_v4(),
        payload.step_id,
        &recorded_as,
        status_str,
        &payload.inputs,
    )
    .await;

    let fencing_token = match resolve_fencing_token_or_response(state, payload.run_id).await {
        Ok(token) => token,
        Err(response) => return *response,
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
            completion_token: stormchaser_core::db::step_completion_token(
                &state.pool,
                payload.step_id,
            )
            .await,
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
            completion_token: stormchaser_core::db::step_completion_token(
                &state.pool,
                payload.step_id,
            )
            .await,
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
