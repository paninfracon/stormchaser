use super::{
    DirectRunRequest, EnqueueRequest, EnqueueResponse, ListRunsQuery, StepDetail,
    WorkflowRunDetail, WorkflowRunFullDetail,
};
use crate::db;
use crate::{AppState, AuthClaims, RUNS_ENQUEUED};
use async_nats::jetstream;
use async_nats::HeaderMap;
use axum::response::sse::Event;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use futures::StreamExt;
use serde_json::Value;
use stormchaser_model::events::WorkflowQueuedEvent;
use stormchaser_model::events::{EventSource, EventType, SchemaVersion, WorkflowEventType};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::workflow::RunStatus;
use stormchaser_model::RunId;
use tokio::sync::mpsc;
use uuid::Uuid;

#[utoipa::path(
    post,
    path = "/api/v1/runs",
    request_body = EnqueueRequest,
    responses(
        (status = 200, description = "Workflow enqueued", body = EnqueueResponse),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
#[tracing::instrument(skip(state, claims), fields(run_id = tracing::field::Empty, initiating_user = tracing::field::Empty))]
/// Enqueue workflow.
pub async fn enqueue_workflow(
    AuthClaims(claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<EnqueueRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let run_id = stormchaser_model::RunId::new_v4();
    let user_id = claims.email.clone().unwrap_or(claims.sub.clone());

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("initiating_user", tracing::field::display(&user_id));

    tracing::info!("Enqueuing workflow: {:?}", payload.workflow_name);
    let fencing_token = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create WorkflowRun with initiating_user
    db::insert_workflow_run(
        &mut tx,
        run_id,
        &payload.workflow_name,
        &user_id,
        &payload.repo_url,
        &payload.workflow_path,
        &payload.git_ref,
        RunStatus::Queued,
        fencing_token,
    )
    .await
    .inspect_err(|e| tracing::error!(run_id = %run_id, "Database error inserting run: {:?}", e))
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Create RunContext (placeholder for dsl_version and workflow_definition)
    db::insert_run_context(
        &mut tx,
        run_id,
        "v1",
        serde_json::json!({}),
        "",
        &payload.inputs,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 3. Create RunQuotas (always, with defaults + overrides)
    let timeout = payload
        .overrides
        .as_ref()
        .and_then(|o| o.timeout.clone())
        .unwrap_or_else(|| "1h".to_string());

    db::insert_run_quotas(&mut tx, run_id, 10, "1", "4Gi", "10Gi", &timeout)
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
        dsl: None,
        inputs: None,
        initiating_user: None,
    };

    publish_cloudevent(
        &jetstream::new(state.nats.clone()),
        NatsSubject::RunQueued,
        EventType::Workflow(WorkflowEventType::Queued),
        EventSource::Api,
        serde_json::to_value(event).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    RUNS_ENQUEUED.add(
        1,
        &[
            opentelemetry::KeyValue::new("workflow_name", payload.workflow_name.clone()),
            opentelemetry::KeyValue::new("initiating_user", user_id.clone()),
        ],
    );

    Ok(Json(EnqueueResponse {
        run_id,
        status: stormchaser_model::RunStatus::Queued,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/runs",
    params(
        ListRunsQuery
    ),
    responses(
        (status = 200, description = "List of workflow runs", body = [WorkflowRunDetail]),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
/// List workflow runs.
pub async fn list_workflow_runs(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Query(params): Query<ListRunsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    let limit = params.limit.unwrap_or(20).min(100);
    let offset = params.offset.unwrap_or(0);

    let runs = db::list_workflow_runs(&state.pool, &params, limit as i64, offset as i64)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch workflow runs: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(runs))
}

/// Gets workflow run details.
#[utoipa::path(
    get,
    path = "/api/v1/runs/{id}",
    params(
        ("id" = stormchaser_model::RunId, Path, description = "Run ID")
    ),
    responses(
        (status = 200, description = "Workflow run details", body = WorkflowRunFullDetail),
        (status = 404, description = "Run not found")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "stormchaser"
)]
/// Get workflow run.
pub async fn get_workflow_run(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
) -> Result<impl IntoResponse, StatusCode> {
    // 1. Fetch the workflow run detail
    let detail: WorkflowRunDetail = db::get_workflow_run_detail(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to fetch workflow run detail for {}: {:?}",
                run_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // 2. Fetch all step instances for this run (active or archived)
    let instances = db::get_step_instances(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch step instances for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let mut steps = Vec::new();
    for instance in instances {
        // Fetch outputs for this step (active or archived)
        let outputs = db::get_step_outputs(&state.pool, instance.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Fetch status history
        let history = db::get_step_status_history(&state.pool, instance.id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        let mut logs = if let Some(backend) = &state.log_backend {
            backend
                .fetch_step_logs(
                    &instance.step_name,
                    instance.id,
                    instance.started_at,
                    instance.finished_at,
                    Some(100), // Reduce payload size for full detail, TUI will fetch on demand
                )
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to fetch logs for step {}: {:?}", instance.id, e);
                    vec![format!("Error fetching logs: {}", e)]
                })
        } else {
            vec!["Log backend not configured".to_string()]
        };

        if logs.len() > 1000 {
            logs = logs.split_off(logs.len() - 1000);
        }

        steps.push(StepDetail {
            instance,
            outputs,
            history,
            logs,
        });
    }

    let artifacts = db::list_run_artifacts(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch run artifacts for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let test_summaries = db::list_run_test_summaries(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch test summaries for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let test_cases = db::list_run_test_cases(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch test cases for {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(WorkflowRunFullDetail {
        detail,
        steps,
        artifacts,
        test_summaries,
        test_cases,
    }))
}

/// Deletes a workflow run.
#[utoipa::path(
    delete,
    path = "/api/v1/runs/{run_id}",
    params(("run_id" = stormchaser_model::RunId, Path, description="Run ID")),
    responses(
        (status = 200, description = "Success"),
        (status = 400, description = "Bad Request"),
        (status = 404, description = "Not Found"),
        (status = 500, description = "Internal Server Error")
    ),
    tag = "workflow"
)]
pub async fn delete_workflow_run_api(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
    Path(run_id): Path<RunId>,
) -> Result<impl IntoResponse, StatusCode> {
    db::delete_workflow_run(&state.pool, run_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete workflow run {}: {:?}", run_id, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/runs/direct",
    request_body = DirectRunRequest,
    responses(
        (status = 200, description = "Workflow started", body = EnqueueResponse),
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "workflow"
)]
/// Executes a direct run.
#[tracing::instrument(skip(state, claims), fields(run_id = tracing::field::Empty, initiating_user = tracing::field::Empty))]
pub async fn direct_run(
    AuthClaims(claims): AuthClaims,
    State(state): State<AppState>,
    Json(payload): Json<DirectRunRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let run_id = stormchaser_model::RunId::new_v4();
    let user_id = claims.email.clone().unwrap_or(claims.sub.clone());

    let span = tracing::Span::current();
    span.record("run_id", tracing::field::display(run_id));
    span.record("initiating_user", tracing::field::display(&user_id));

    // Publish to NATS stormchaser.run.direct
    let payload_json = serde_json::json!({
        "run_id": run_id,
        "dsl": payload.dsl,
        "initiating_user": user_id,
        "inputs": payload.inputs,
    });

    use cloudevents::{EventBuilder, EventBuilderV10};
    let event = EventBuilderV10::new()
        .id(uuid::Uuid::new_v4().to_string())
        .ty("stormchaser.v1.run.direct")
        .source(EventSource::Api.as_str())
        .time(Utc::now())
        .data(stormchaser_model::APPLICATION_JSON, payload_json)
        .build()
        .map_err(|e| {
            tracing::error!("Failed to build CloudEvent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let event_str = serde_json::to_string(&event).unwrap_or_default();
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", "application/cloudevents+json");

    state
        .nats
        .publish_with_headers("stormchaser.v1.run.direct", headers, event_str.into())
        .await
        .map_err(|e| {
            tracing::error!("Failed to publish CloudEvent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(EnqueueResponse {
        run_id,
        status: RunStatus::Running,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/runs/stream",
    responses(
        (status = 200, description = "Workflow runs stream (SSE)")
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "workflow"
)]
/// Stream workflow runs api.
pub async fn stream_workflow_runs_api(
    AuthClaims(_claims): AuthClaims,
    State(state): State<AppState>,
) -> Result<
    axum::response::sse::Sse<
        impl futures::stream::Stream<Item = Result<Event, std::convert::Infallible>>,
    >,
    StatusCode,
> {
    let (tx, rx) = mpsc::channel::<Result<Event, std::convert::Infallible>>(100);

    let nats = state.nats.clone();
    let pool = state.pool.clone();

    tokio::spawn(async move {
        let mut subscriber = match nats.subscribe("stormchaser.v1.run.>").await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!("Failed to subscribe to NATS for workflow runs: {:?}", e);
                return;
            }
        };

        while let Some(msg) = subscriber.next().await {
            let ce: cloudevents::Event = match serde_json::from_slice(&msg.payload) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let payload: Value = if let Some(cloudevents::Data::Json(v)) = ce.data() {
                v.clone()
            } else {
                continue;
            };

            if let Some(run_id_str) = payload.get("run_id").and_then(|id| id.as_str()) {
                if let Ok(run_id) = Uuid::parse_str(run_id_str) {
                    // Fetch full detail for the run
                    let detail =
                        db::get_workflow_run_detail(&pool, stormchaser_model::RunId::new(run_id))
                            .await
                            .unwrap_or(None);

                    if let Some(run) = detail {
                        let data = serde_json::to_string(&run).unwrap_or_default();
                        let event = Event::default().event("workflow_run").data(data);
                        if tx.send(Ok(event)).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|res| match res {
        Ok(event) => Ok(event),
        Err(_) => unreachable!(),
    });

    Ok(axum::response::sse::Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}
