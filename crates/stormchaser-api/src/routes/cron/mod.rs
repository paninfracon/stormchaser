pub mod external;
use crate::db;
use async_nats::jetstream;
use chrono::Utc;
use external::*;
use stormchaser_model::cron;
use stormchaser_model::events::WorkflowQueuedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::CronWorkflowId;
use stormchaser_model::RunId;

use super::{CreateCronWorkflowRequest, CronWorkflowResponse, EnqueueResponse};
use crate::{AppState, AuthClaims};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};

/// Create cron workflow.
#[utoipa::path(
    post,
    path = "/api/v1/cron-workflows",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "cron"
)]
pub async fn create_cron_workflow(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<CreateCronWorkflowRequest>,
) -> Result<Json<CronWorkflowResponse>, StatusCode> {
    let id = CronWorkflowId::new_v4();
    let secret_token = CronWorkflowId::new_v4().to_string();

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

    // 1. Register with external cron engine
    let external_job_id =
        register_external_cron(id, &payload.name, &payload.cronspec, &secret_token).await?;

    // 2. Save to database
    db::insert_cron_workflow(
        &state.pool,
        id,
        &payload,
        &repo_url,
        &secret_token,
        external_job_id.clone(),
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create cron workflow: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(CronWorkflowResponse {
        id,
        secret_token,
        external_job_id,
    }))
}

/// List cron workflows.
#[utoipa::path(
    get,
    path = "/api/v1/cron-workflows",
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "cron"
)]
pub async fn list_cron_workflows(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
) -> Result<Json<Vec<cron::CronWorkflow>>, StatusCode> {
    let workflows = db::list_cron_workflows(&state.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(workflows))
}

/// Deletes a cron workflow.
#[utoipa::path(
    delete,
    path = "/api/v1/cron-workflows/{id}",
    params(("id" = stormchaser_model::CronWorkflowId, Path, description="Cron ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "cron"
)]
pub async fn delete_cron_workflow(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(id): Path<CronWorkflowId>,
) -> Result<StatusCode, StatusCode> {
    // 1. Fetch to get external_job_id
    let workflow = db::get_cron_workflow(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let workflow = match workflow {
        Some(w) => w,
        None => return Err(StatusCode::NOT_FOUND),
    };

    // 2. Unregister from external cron engine (K8s)
    if let Some(ext_id) = workflow.external_job_id {
        unregister_external_cron(&ext_id).await?;
    }

    // 3. Delete from DB
    db::delete_cron_workflow(&state.pool, id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/cron-trigger/{id}",
    params(
        ("id" = stormchaser_model::CronWorkflowId, Path, description = "Cron workflow ID")
    ),
    responses(
        (status = 200, description = "Workflow triggered", body = EnqueueResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Cron workflow not found"),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "cron"
)]
/// Trigger cron workflow.
pub async fn trigger_cron_workflow(
    headers: HeaderMap,
    State(state): State<AppState>,
    Path(id): Path<CronWorkflowId>,
) -> Result<Json<EnqueueResponse>, StatusCode> {
    // 1. Fetch CronWorkflow
    let cron = db::get_active_cron_workflow(&state.pool, id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?
        .ok_or(StatusCode::NOT_FOUND)?;

    // 2. Validate Token (using constant-time approach via hashing)
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let expected = format!("Bearer {}", cron.secret_token);

    use sha2::{Digest, Sha256};
    let mut hasher1 = Sha256::new();
    hasher1.update(auth_header.as_bytes());
    let hash1 = hasher1.finalize();

    let mut hasher2 = Sha256::new();
    hasher2.update(expected.as_bytes());
    let hash2 = hasher2.finalize();

    if hash1 != hash2 {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // 3. Enqueue a new run
    let run_id = RunId::new_v4();

    tracing::info!(%run_id, "Enqueuing cron workflow: {}", cron.workflow_name);
    let fencing_token = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    db::insert_workflow_run(
        &mut tx,
        run_id,
        &cron.workflow_name,
        "system:cron",
        &cron.repo_url,
        &cron.workflow_path,
        &cron.git_ref,
        RunStatus::Queued,
        fencing_token,
    )
    .await
    .inspect_err(|e| tracing::error!(run_id = %run_id, "Database error inserting run: {:?}", e))
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    db::insert_run_context(
        &mut *tx,
        run_id,
        "v1",
        serde_json::json!({}),
        "",
        &cron.inputs,
        serde_json::json!({}),
        vec![],
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    db::insert_run_quotas(&mut tx, run_id, 10, "1", "4Gi", "10Gi", "1h")
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Publish to NATS
    let event = WorkflowQueuedEvent {
        run_id,
        event_type: EventType::Workflow(WorkflowEventType::Queued),
        timestamp: Utc::now(),
        status: stormchaser_model::workflow::RunStatus::Queued,
        dsl: None,
        inputs: None,
        initiating_user: None,
        sops_file: None, // Cron jobs might need these later, but default to None for now
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

    Ok(Json(EnqueueResponse {
        run_id,
        status: RunStatus::Queued,
    }))
}
